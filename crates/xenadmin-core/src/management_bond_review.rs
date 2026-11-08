use crate::{Error, ManagementNetworkOperation, ObservedResource, ResourceTarget, xapi::Session};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct ManagementBondReviewQuery {
    receiver: watch::Receiver<Option<Result<ManagementNetworkOperation, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl ManagementBondReviewQuery {
    pub fn latest(&self) -> Option<Result<ManagementNetworkOperation, String>> {
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
impl Drop for ManagementBondReviewQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedResource,
    operation: ManagementNetworkOperation,
    sender: watch::Sender<Option<Result<ManagementNetworkOperation, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedResource,
    operation: ManagementNetworkOperation,
    runtime: tokio::runtime::Handle,
) -> Result<(ManagementBondReviewQuery, Request), Error> {
    operation.validate(observed.target())?;
    if !matches!(operation, ManagementNetworkOperation::Bond { .. })
        || observed.target() != ResourceTarget::Host(operation.source().identity.host)
    {
        return Err(Error::InvalidResourceCommand);
    }
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    Ok((
        ManagementBondReviewQuery {
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
            result=tokio::time::timeout(std::time::Duration::from_secs(45), crate::resources::management::review_bond(&session,self.observed,self.operation))=>result.unwrap_or(Err(Error::Timeout)),
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
