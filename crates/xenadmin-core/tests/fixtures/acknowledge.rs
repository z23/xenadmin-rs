//! Acknowledging an unknown outcome through a recovered instance races that instance's
//! own recheck: its tracking worker writes a transient Verifying state before it reads
//! the host, and the journal refuses an acknowledgement that arrives inside that window.
//! The refusal reaches the connection only as a message, exactly as it reaches an
//! operator, who would offer the acknowledgement again once the record shows
//! OutcomeUnknown. This helper does the same, so a fixture asserts the acknowledged
//! record rather than the timing of a recheck it did not request.
use std::time::Duration;
use xenadmin_core::{Connection, OperationControl, OperationId, OperationState};

pub async fn acknowledge_unknown(connection: &Connection, id: OperationId, seconds: u64) {
    tokio::time::timeout(crate::deadlines::deadline(seconds), async {
        let mut offered: Option<tokio::time::Instant> = None;
        loop {
            let state = connection
                .resource_history()
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.state.clone());
            match state {
                Some(OperationState::AcknowledgedUnknown) => break,
                Some(OperationState::OutcomeUnknown)
                    if offered.is_none_or(|at| at.elapsed() >= Duration::from_millis(200)) =>
                {
                    connection
                        .control_resource(id, OperationControl::AcknowledgeUnknown)
                        .unwrap();
                    offered = Some(tokio::time::Instant::now());
                }
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the unknown outcome was acknowledged");
}
