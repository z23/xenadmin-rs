use crate::{Error, ManagementIpv6Bond, ObservedResource, xapi::Session};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct ManagementIpv6BondReviewQuery {
    receiver: watch::Receiver<Option<Result<ManagementIpv6Bond, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl ManagementIpv6BondReviewQuery {
    pub fn latest(&self) -> Option<Result<ManagementIpv6Bond, String>> {
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
impl Drop for ManagementIpv6BondReviewQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedResource,
    operation: ManagementIpv6Bond,
    sender: watch::Sender<Option<Result<ManagementIpv6Bond, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    operation: ManagementIpv6Bond,
    runtime: tokio::runtime::Handle,
) -> Result<(ManagementIpv6BondReviewQuery, Request), Error> {
    operation.validate(observed.target())?;
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    Ok((
        ManagementIpv6BondReviewQuery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
            operation,
            sender,
            cancel,
        },
    ))
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
        let result = tokio::select! {
            _=self.cancel.cancelled()=>Err(Error::Closed),
            _=generation.cancelled()=>Err(Error::Stale),
            result=tokio::time::timeout(std::time::Duration::from_secs(45), crate::resources::management_ipv6::review(&session,self.observed,self.operation))=>result.unwrap_or(Err(Error::Timeout)),
        };
        let current = receiver
            .borrow()
            .observe_resource(self.observed.target())
            .is_ok_and(|v| v == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
}
