use crate::{
    Error, HostId, NetworkId, NetworkSriovId, ObservedResource, PciId, PifId, PoolId,
    ResourceTarget, SriovConfigurationMode, xapi::Session,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SriovCapacity {
    pub pool: PoolId,
    pub sriov: NetworkSriovId,
    pub host: HostId,
    pub network: NetworkId,
    pub physical: PifId,
    pub logical: PifId,
    pub pci: PciId,
    pub remaining_vfs: u64,
    pub observed_at_ms: u64,
    pub requires_reboot: Option<bool>,
    pub mode: Option<SriovConfigurationMode>,
    pub attached: Option<bool>,
}

pub struct SriovCapacityQuery {
    observed: ObservedResource,
    receiver: watch::Receiver<Option<Result<SriovCapacity, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}

impl SriovCapacityQuery {
    pub fn latest(&self) -> Option<Result<SriovCapacity, String>> {
        self.receiver.borrow().clone().or_else(|| {
            self.receiver
                .has_changed()
                .is_err()
                .then(|| Err(Error::Closed.to_string()))
        })
    }

    pub fn is_current(&self, snapshot: Option<&crate::ConnectionSnapshot>) -> bool {
        snapshot.and_then(|s| s.observe_resource(self.observed.target()).ok())
            == Some(self.observed)
    }

    pub fn notify_on_change(&self, waker: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                waker();
            }
        });
    }
}

impl Drop for SriovCapacityQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(crate) struct Request {
    pub observed: ObservedResource,
    sender: watch::Sender<Option<Result<SriovCapacity, String>>>,
    cancel: CancellationToken,
}

pub(crate) fn channel(
    observed: ObservedResource,
    runtime: tokio::runtime::Handle,
) -> Result<(SriovCapacityQuery, Request), Error> {
    if !matches!(observed.target(), ResourceTarget::Sriov(_)) {
        return Err(Error::InvalidResourceCommand);
    }
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    Ok((
        SriovCapacityQuery {
            observed,
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
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
        let ResourceTarget::Sriov(id) = self.observed.target() else {
            self.fail(Error::InvalidResourceCommand);
            return;
        };
        let result = tokio::select! {
            _ = self.cancel.cancelled() => Err(Error::Closed),
            _ = generation.cancelled() => Err(Error::Stale),
            value = tokio::time::timeout(
                Duration::from_secs(30),
                crate::resources::network_fabric::capacity(&session, self.observed.pool, id),
            ) => value.unwrap_or(Err(Error::Timeout)),
        };
        let current = receiver
            .borrow()
            .observe_resource(self.observed.target())
            .is_ok_and(|o| o == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
}
