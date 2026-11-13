use crate::{Error, HostId, ObservedResource, ResourceTarget, SnapshotPolicyClock, xapi::Session};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use xenadmin_xapi::records::{HostRecord, PoolRecord};
use xenadmin_xapi::{calls, refs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPolicySupport {
    pub clock: SnapshotPolicyClock,
    pub coordinator_name: String,
    pub local_time: String,
    pub utc_time: String,
}
pub struct SnapshotPolicySupportQuery {
    receiver: watch::Receiver<Option<Result<SnapshotPolicySupport, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl SnapshotPolicySupportQuery {
    pub fn latest(&self) -> Option<Result<SnapshotPolicySupport, String>> {
        self.receiver.borrow().clone().or_else(|| {
            self.receiver
                .has_changed()
                .is_err()
                .then(|| Err(Error::Closed.to_string()))
        })
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}
impl Drop for SnapshotPolicySupportQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedResource,
    sender: watch::Sender<Option<Result<SnapshotPolicySupport, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    runtime: tokio::runtime::Handle,
) -> (SnapshotPolicySupportQuery, Request) {
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    (
        SnapshotPolicySupportQuery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
            sender,
            cancel,
        },
    )
}
impl Request {
    pub(crate) fn fail(self, error: Error) {
        self.sender.send_replace(Some(Err(error.to_string())));
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        receiver: watch::Receiver<Arc<crate::ConnectionSnapshot>>,
        generation: CancellationToken,
    ) {
        let result = tokio::select! {_=self.cancel.cancelled()=>Err(Error::Closed),_=generation.cancelled()=>Err(Error::Stale),r=tokio::time::timeout(Duration::from_secs(30),read(&session,self.observed.target))=>r.unwrap_or(Err(Error::Timeout))};
        let current = receiver
            .borrow()
            .observe_resource(self.observed.target)
            .is_ok_and(|o| o == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
}
pub(crate) async fn read(
    session: &Session,
    target: ResourceTarget,
) -> Result<SnapshotPolicySupport, Error> {
    let ResourceTarget::Pool(id) = target else {
        return Err(Error::InvalidResourceCommand);
    };
    let pr = crate::xapi::opaque_reference(
        &session
            .send(calls::pool::GetByUuid(&id.to_string()))
            .await?,
    )?;
    let pool = session
        .send(calls::pool::GetRecord(refs::pool(&pr)))
        .await?;
    if PoolRecord::new(&pool).uuid() != id.to_string() {
        return Err(Error::Stale);
    }
    if PoolRecord::new(&pool).restrictions()["restrict_vmss"] != "false" {
        return Err(Error::Protocol(
            "The pool does not explicitly advertise snapshot scheduling support",
        ));
    }
    let hr = crate::xapi::opaque_reference(PoolRecord::new(&pool).master())?;
    let host = session
        .send(calls::host::GetRecord(refs::host(&hr)))
        .await?;
    let id = HostRecord::new(&host)
        .uuid()
        .text()
        .and_then(|s| HostId::parse(s).ok())
        .ok_or(Error::Stale)?;
    let sw = &HostRecord::new(&host).software_version();
    if sw["product_brand"] != "XCP-ng"
        || !sw["product_version"]
            .as_str()
            .is_some_and(|s| s == "8.3" || s.starts_with("8.3."))
    {
        return Err(Error::Protocol(
            "This client supports the XCP-ng 8.3 VMSS package; this coordinator reports another host version",
        ));
    }
    if !session
        .send(calls::vmss::GetAllRecords)
        .await?
        .as_object()
        .is_some_and(|m| m.len() <= 10_000)
    {
        return Err(Error::Protocol(
            "Snapshot schedule inventory unavailable or too large",
        ));
    }
    let utc1 = session
        .send(calls::host::GetServertime(refs::host(&hr)))
        .await?;
    let local = session
        .send(calls::host::GetServerLocaltime(refs::host(&hr)))
        .await?;
    let utc2 = session
        .send(calls::host::GetServertime(refs::host(&hr)))
        .await?;
    let u1 = clock_seconds(
        utc1.as_str()
            .ok_or(Error::Protocol("Host UTC clock unavailable"))?,
    )?;
    let u2 = clock_seconds(
        utc2.as_str()
            .ok_or(Error::Protocol("Host UTC clock unavailable"))?,
    )?;
    let local_time = local
        .as_str()
        .ok_or(Error::Protocol("Host local clock unavailable"))?;
    let l = clock_seconds(local_time)?;
    if !(0..=10).contains(&(u2 - u1)) {
        return Err(Error::Protocol(
            "Host clock changed or clock query took too long; refresh support",
        ));
    }
    let offset = ((l - u1 + 30).div_euclid(60)) * 60;
    if (l - u1 - offset).abs() > 10 || !(-14 * 3600..=14 * 3600).contains(&offset) {
        return Err(Error::Protocol(
            "Host local timezone offset could not be determined",
        ));
    }
    let pool2 = session
        .send(calls::pool::GetRecord(refs::pool(&pr)))
        .await?;
    if PoolRecord::new(&pool2).uuid() != PoolRecord::new(&pool).uuid()
        || PoolRecord::new(&pool2).master() != PoolRecord::new(&pool).master()
    {
        return Err(Error::Stale);
    }
    Ok(SnapshotPolicySupport {
        clock: SnapshotPolicyClock {
            coordinator: id,
            utc_offset_minutes: (offset / 60) as i32,
        },
        coordinator_name: HostRecord::new(&host)
            .name_label()
            .text()
            .unwrap_or("Coordinator")
            .to_owned(),
        local_time: local_time.to_owned(),
        utc_time: utc2.as_str().unwrap_or_default().to_owned(),
    })
}
pub(crate) fn clock_seconds(s: &str) -> Result<i64, Error> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    let s = s.replace('-', "");
    if s.len() != 17 || !s.is_ascii() || &s[8..9] != "T" || &s[11..12] != ":" || &s[14..15] != ":" {
        return Err(Error::Protocol("Invalid host clock format"));
    }
    let n = |a, b| {
        s[a..b]
            .parse::<i64>()
            .map_err(|_| Error::Protocol("Invalid host clock number"))
    };
    let (y, m, d, h, mi, se) = (
        n(0, 4)?,
        n(4, 6)?,
        n(6, 8)?,
        n(9, 11)?,
        n(12, 14)?,
        n(15, 17)?,
    );
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || d < 1
        || d > days[(m - 1) as usize]
        || !(0..24).contains(&h)
        || !(0..60).contains(&mi)
        || !(0..60).contains(&se)
    {
        return Err(Error::Protocol("Invalid host clock date"));
    }
    let mut count = 0i64;
    for year in 1970..y {
        count += if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
            366
        } else {
            365
        };
    }
    count += days[..(m - 1) as usize].iter().sum::<i64>() + d - 1;
    Ok(count * 86400 + h * 3600 + mi * 60 + se)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_calendar_boundaries_without_assuming_local_timezone() {
        assert_eq!(clock_seconds("19700101T00:00:00Z").unwrap(), 0);
        assert_eq!(
            clock_seconds("2024-02-29T23:59:59Z").unwrap() + 1,
            clock_seconds("20240301T00:00:00").unwrap()
        );
        assert!(clock_seconds("20230229T00:00:00").is_err());
        assert!(clock_seconds("private-invalid-payload").is_err());
    }
}
