use super::*;
use crate::{ConnectionSnapshot, Inventory, operations::OperationUpdate, xapi::Session};
use std::time::SystemTime;
use tokio::sync::mpsc;

pub(crate) struct MetricsRequest {
    pub observed: ObservedMetrics,
    options: MetricsOptions,
    sender: watch::Sender<Arc<MetricsSnapshot>>,
    cancel: CancellationToken,
}
impl MetricsRequest {
    pub(super) fn new(
        observed: ObservedMetrics,
        options: MetricsOptions,
        sender: watch::Sender<Arc<MetricsSnapshot>>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            observed,
            options,
            sender,
            cancel,
        }
    }
    pub(crate) fn fail(self, error: Error) {
        self.publish_error(MetricsState::Failed, Some(error.to_string()));
    }
    fn publish_error(&self, state: MetricsState, error: Option<String>) {
        let mut snapshot = (*self.sender.borrow().clone()).clone();
        snapshot.state = state;
        snapshot.error = error;
        self.sender.send_replace(Arc::new(snapshot));
    }
    fn current(
        &self,
        receiver: &watch::Receiver<Arc<ConnectionSnapshot>>,
    ) -> Result<Arc<Inventory>, Error> {
        let snapshot = receiver.borrow().clone();
        if snapshot.observe_metrics(self.observed.target)? != self.observed {
            return Err(Error::Stale);
        }
        snapshot.inventory().cloned().ok_or(Error::Stale)
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        receiver: watch::Receiver<Arc<ConnectionSnapshot>>,
        generation_cancel: CancellationToken,
        updates: mpsc::Sender<OperationUpdate>,
    ) {
        let mut route = None;
        let mut end = None;
        let mut delay = Duration::ZERO;
        let mut retry = 5;
        loop {
            tokio::select! {
                _=self.cancel.cancelled()=>{self.publish_error(MetricsState::Closed,None);break;},
                _=generation_cancel.cancelled()=>{self.publish_error(MetricsState::Stale,Some("The connection changed; reopen performance metrics".into()));break;},
                _=tokio::time::sleep(delay)=>{},
            }
            let result = tokio::select! {
                _=self.cancel.cancelled()=>{self.publish_error(MetricsState::Closed,None);break;},
                _=generation_cancel.cancelled()=>{self.publish_error(MetricsState::Stale,Some("The connection changed; reopen performance metrics".into()));break;},
                result=self.refresh(&session,&receiver,&mut route,&mut end)=>result,
            };
            match result {
                Ok(()) => {
                    delay = self.options.window.poll();
                    retry = 5;
                }
                Err(error) => {
                    if let Error::Coordinator(endpoint) = &error {
                        let _ = updates
                            .send(OperationUpdate::Coordinator(endpoint.clone()))
                            .await;
                    }
                    let retryable = error.retryable() || matches!(error, Error::Stale);
                    self.publish_error(
                        if retryable {
                            MetricsState::Stale
                        } else {
                            MetricsState::Failed
                        },
                        Some(error.to_string()),
                    );
                    if !retryable {
                        break;
                    }
                    delay = Duration::from_secs(retry);
                    retry = (retry * 2).min(30);
                }
            }
        }
    }
    async fn refresh(
        &self,
        session: &Session,
        receiver: &watch::Receiver<Arc<ConnectionSnapshot>>,
        previous_route: &mut Option<transport::Route>,
        previous_end: &mut Option<u64>,
    ) -> Result<(), Error> {
        let inventory = self.current(receiver)?;
        let route = transport::resolve(session, self.observed, &inventory).await?;
        let reset = previous_route.as_ref() != Some(&route);
        let mut sources = transport::sources(session, self.observed.target, &route).await?;
        let selected = decode::select(&mut sources, &self.options.sources);
        let interval = self.options.window.interval_seconds();
        let start = if reset {
            -(self.options.window.duration_seconds() as i64)
        } else {
            previous_end
                .map(|end| {
                    i64::try_from(end.saturating_sub(interval))
                        .map_err(|_| Error::Protocol("metric timestamp is out of range"))
                })
                .transpose()?
                .unwrap_or(-(self.options.window.duration_seconds() as i64))
        };
        let bytes = transport::fetch(
            session,
            &route,
            self.observed.target,
            &inventory,
            start,
            interval,
        )
        .await?;
        let current = self.current(receiver)?;
        // Migration during the HTTP request invalidates that response before publication.
        if transport::resolve(session, self.observed, &current).await? != route {
            return Err(Error::Stale);
        }
        self.current(receiver)?;
        let batch = decode::decode(&bytes, self.observed.target, &sources, &selected)?;
        *previous_end = Some(batch.end);
        *previous_route = Some(route);
        let mut snapshot = (*self.sender.borrow().clone()).clone();
        snapshot.sources = sources;
        decode::merge(&mut snapshot, batch, self.options.window, reset);
        snapshot.state = MetricsState::Ready;
        snapshot.error = None;
        snapshot.updated_at_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis() as u64);
        self.sender.send_replace(Arc::new(snapshot));
        Ok(())
    }
}
