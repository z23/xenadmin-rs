use crate::{ActionAvailability, Error, ObservedVm, PowerAction, xapi::Session};
use std::sync::Arc;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub struct ActionDiagnostic {
    receiver: watch::Receiver<Option<Result<ActionAvailability, String>>>,
    runtime: tokio::runtime::Handle,
}
impl ActionDiagnostic {
    pub fn latest(&self) -> Option<Result<ActionAvailability, String>> {
        self.receiver.borrow().clone()
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                callback()
            }
        });
    }
}
pub(crate) struct DiagnosticRequest {
    pub target: ObservedVm,
    action: PowerAction,
    sender: watch::Sender<Option<Result<ActionAvailability, String>>>,
}
pub(crate) fn channel(
    target: ObservedVm,
    action: PowerAction,
    runtime: tokio::runtime::Handle,
) -> (ActionDiagnostic, DiagnosticRequest) {
    let (sender, receiver) = watch::channel(None);
    (
        ActionDiagnostic { receiver, runtime },
        DiagnosticRequest {
            target,
            action,
            sender,
        },
    )
}
impl DiagnosticRequest {
    pub fn fail(self, error: Error) {
        self.sender.send_replace(Some(Err(error.to_string())));
    }
    pub async fn run(self, session: Arc<Session>, cancel: CancellationToken) {
        let result = tokio::select! {_=cancel.cancelled()=>Err(Error::Stale),result=session.diagnose_power(self.target.id,self.action)=>result};
        self.sender
            .send_replace(Some(result.map_err(|error| error.to_string())));
    }
}
