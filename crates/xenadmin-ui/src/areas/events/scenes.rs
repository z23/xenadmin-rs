//! The scenes of the area, under their Rune names.

use std::sync::Arc;

use xenadmin_core::{MessageId, ServerMessage, ServerTask, TaskId};

use super::activity::{self, Tab};
use super::failure_help::FailureHelp;
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("activity", "events", scene),
        SceneSpec::new("failure-help", "events", failure_help_scene),
    ]
}

/// The demo pool with two tasks ("Synthetic task 1" pending at 35 percent
/// with cancel allowed, "Synthetic task 2" finished) and two messages
/// (priorities 1 and 2). Public for the tests, which start from it.
pub fn prepare(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(mut inventory) = m.inventory.take().map(|i| i.as_ref().clone()) else {
        return;
    };
    for index in 1..=2 {
        let id = TaskId::from(uuid::Uuid::from_u128(900 + index));
        inventory.details.tasks.insert(
            id,
            ServerTask {
                id,
                name: format!("Synthetic task {index}"),
                description: "Synthetic storage operation".into(),
                status: if index == 1 { "pending" } else { "success" }.into(),
                progress: Some(if index == 1 { 0.35 } else { 1.0 }),
                created: Some("20261108T10:00:00Z".into()),
                finished: None,
                error_code: None,
                allowed_operations: Some(vec!["cancel".into()]),
                resident_host: None,
                parent: None,
                subtasks: Vec::new(),
                result_type: None,
            },
        );
        let id = MessageId::from(uuid::Uuid::from_u128(1000 + index));
        inventory.details.messages.insert(
            id,
            ServerMessage {
                id,
                name: format!("Synthetic message {index}"),
                priority: Some(index as i64),
                object_class: "VM".into(),
                object_uuid: None,
                timestamp: Some("20261108T10:00:00Z".into()),
                body: format!("Synthetic alert text {index}. No host data is used."),
            },
        );
    }
    m.inventory = Some(Arc::new(inventory));
}

/// The Rune `activity::scene`: the Activity dialog on Tasks with the first
/// task selected. Here: the Notifications view on its Events tab, the Tasks
/// list, the first task selected.
fn scene(m: &mut Model) {
    prepare(m);
    super::show(m);
    m.areas.events.list = Tab::Tasks;
    let first = activity::current(m, Tab::Tasks)
        .rows
        .first()
        .map(|row| row.key.clone());
    m.areas.events.selected = first;
}

/// The Rune `failure_help::scene`: a captured explanation of a partial
/// snapshot with an exact host error code.
fn failure_help_scene(m: &mut Model) {
    m.load_demo(3, 48);
    super::show_help(
        m,
        FailureHelp::new(
            "Create snapshot · synthetic operation\nPool 00000000-0000-0000-0000-000000000001\nVM 00000000-0000-0000-0000-000000000002".into(),
            "Some work completed and later work stopped. Inspect each step and retained resource UUID. Repeating the entire workflow could duplicate completed work; any remaining change or cleanup needs a fresh review.",
            Some("SR_BACKEND_FAILURE_44"),
        ),
    );
}
