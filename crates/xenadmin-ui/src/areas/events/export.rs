//! Export of the listed entries as JSON: the public projection of tasks,
//! messages and operation records, the save prompt, the writer thread and
//! the owner-only atomic write.
//!
//! The Rune `activity_export.rs`. `Entry`, `Step`, `Created`, the document
//! and `encode_bounded` are as they were; its `write_private` is
//! [`crate::files::write_private`], shared with the search files. `choose`
//! asks for the file with `Model::choose_file` in place of the Rune
//! `tx.save_file`; the reply of a prompt is a `Fn`, so the captured
//! document waits in [`Export`] until the reply takes it. A prompt that
//! could not be shown ends the export with the Rune notice "Cannot open
//! save dialog: {error}".

use std::{io::Write, sync::mpsc};

use serde::Serialize;
use xenadmin_core::{
    HostId, OperationRecord, PoolId, ResourceOperationRecord, ServerMessage, ServerTask,
};

use super::activity::{self, Tab};
use crate::files::write_private;
use crate::prelude::*;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Task {
        pool: PoolId,
        uuid: uuid::Uuid,
        name: String,
        description: String,
        status: String,
        progress: Option<f64>,
        created: Option<String>,
        finished: Option<String>,
        error_code: Option<String>,
        resident_host: Option<HostId>,
        parent: Option<xenadmin_core::TaskId>,
        subtasks: Vec<xenadmin_core::TaskId>,
        result_type: Option<String>,
    },
    Message {
        pool: PoolId,
        uuid: uuid::Uuid,
        name: String,
        priority: Option<i64>,
        object_class: String,
        object_uuid: Option<uuid::Uuid>,
        timestamp: Option<String>,
        body: String,
    },
    Operation {
        pool: PoolId,
        uuid: xenadmin_core::OperationId,
        family: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        management_network: Option<xenadmin_core::ManagementNetworkEvidence>,
        management_ipv6_bond: Option<Box<xenadmin_core::ManagementIpv6BondEvidence>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        destination_pool: Option<PoolId>,
        target: uuid::Uuid,
        action: String,
        state: String,
        detail: String,
        progress: Option<f64>,
        created_at_ms: u64,
        steps: Vec<Step>,
        created: Vec<Created>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        removed: Vec<Created>,
    },
}
#[derive(Clone, Serialize)]
pub struct Step {
    label: String,
    state: String,
    detail: String,
}
#[derive(Clone, Serialize)]
pub struct Created {
    kind: String,
    uuid: uuid::Uuid,
}
impl Entry {
    pub fn task(pool: PoolId, task: &ServerTask) -> Self {
        Self::Task {
            pool,
            uuid: task.id.uuid(),
            name: task.name.clone(),
            description: task.description.clone(),
            status: task.status.clone(),
            progress: task.progress,
            created: task.created.clone(),
            finished: task.finished.clone(),
            error_code: task.error_code.clone(),
            resident_host: task.resident_host,
            parent: task.parent,
            subtasks: task.subtasks.clone(),
            result_type: task.result_type.clone(),
        }
    }
    pub fn message(pool: PoolId, message: &ServerMessage) -> Self {
        Self::Message {
            pool,
            uuid: message.id.uuid(),
            name: message.name.clone(),
            priority: message.priority,
            object_class: message.object_class.clone(),
            object_uuid: message.object_uuid,
            timestamp: message.timestamp.clone(),
            body: message.body.clone(),
        }
    }
    pub fn vm(record: &OperationRecord) -> Self {
        let mut created = record
            .created_devices
            .iter()
            .map(|device| Created {
                kind: device.label().into(),
                uuid: device.uuid(),
            })
            .collect::<Vec<_>>();
        if let Some(id) = record.created_vm {
            created.push(Created {
                kind: "VM".into(),
                uuid: id.uuid(),
            });
        }
        if let Some(id) = record.created_snapshot {
            created.push(Created {
                kind: "Snapshot".into(),
                uuid: id.uuid(),
            });
        }
        Self::Operation {
            pool: record.pool,
            uuid: record.id,
            family: "vm",
            management_network: None,
            management_ipv6_bond: None,
            destination_pool: None,
            target: record.vm.uuid(),
            action: record.action.label().into(),
            state: format!("{:?}", record.state),
            detail: record.detail.clone(),
            progress: record.progress,
            created_at_ms: record.created_at_ms,
            steps: steps(&record.steps),
            created,
            removed: Vec::new(),
        }
    }
    pub fn resource(record: &ResourceOperationRecord) -> Self {
        Self::Operation {
            pool: record.pool,
            uuid: record.id,
            family: "resource",
            management_network: record.management_network.clone(),
            management_ipv6_bond: record.management_ipv6_bond.clone().map(Box::new),
            destination_pool: record.peer_pool(),
            target: match &record.action {
                xenadmin_core::ResourceAction::CrossPoolVm(operation) => operation.vm.uuid(),
                _ => record.target.uuid(),
            },
            action: record.action.label().into(),
            state: format!("{:?}", record.state),
            detail: record.detail.clone(),
            progress: record.progress,
            created_at_ms: record.created_at_ms,
            steps: steps(&record.steps),
            created: record
                .created
                .filter(|target| {
                    record
                        .network_fabric_created
                        .as_ref()
                        .is_none_or(|e| e.object.target() != *target)
                })
                .map(|target| Created {
                    kind: "Resource".into(),
                    uuid: target.uuid(),
                })
                .into_iter()
                .chain(record.network_fabric_created.iter().flat_map(|e| {
                    [
                        Created {
                            kind: "Network object".into(),
                            uuid: e.object.uuid(),
                        },
                        Created {
                            kind: "Host interface".into(),
                            uuid: e.interface.uuid(),
                        },
                    ]
                }))
                .chain(
                    record
                        .storage_graph
                        .iter()
                        .filter(|r| r.created)
                        .map(|r| Created {
                            kind: "Storage attachment".into(),
                            uuid: r.pbd.uuid(),
                        }),
                )
                .chain(record.transfer_created.vms.iter().map(|id| Created {
                    kind: "VM".into(),
                    uuid: id.uuid(),
                }))
                .chain(record.transfer_created.disks.iter().map(|id| Created {
                    kind: "Disk".into(),
                    uuid: id.uuid(),
                }))
                .chain(record.transfer_created.networks.iter().map(|id| Created {
                    kind: "Network".into(),
                    uuid: id.uuid(),
                }))
                .chain(record.transfer_created.vbds.iter().map(|id| Created {
                    kind: "Disk attachment".into(),
                    uuid: id.uuid(),
                }))
                .chain(record.transfer_created.vifs.iter().map(|id| Created {
                    kind: "NIC".into(),
                    uuid: id.uuid(),
                }))
                .chain(
                    record
                        .topology_changes
                        .created_vlans
                        .iter()
                        .map(|id| Created {
                            kind: "VLAN".into(),
                            uuid: id.uuid(),
                        }),
                )
                .chain(
                    record
                        .topology_changes
                        .created_pifs
                        .iter()
                        .map(|id| Created {
                            kind: "Physical interface".into(),
                            uuid: id.uuid(),
                        }),
                )
                .chain(record.topology_changes.created_fabric.iter().flat_map(|e| {
                    [
                        Created {
                            kind: "Network object".into(),
                            uuid: e.object.uuid(),
                        },
                        Created {
                            kind: "Physical interface".into(),
                            uuid: e.interface.uuid(),
                        },
                    ]
                }))
                .collect(),
            removed: record
                .topology_changes
                .removed_vlans
                .iter()
                .map(|id| Created {
                    kind: "VLAN".into(),
                    uuid: id.uuid(),
                })
                .chain(
                    record
                        .topology_changes
                        .removed_pifs
                        .iter()
                        .map(|id| Created {
                            kind: "Physical interface".into(),
                            uuid: id.uuid(),
                        }),
                )
                .chain(record.topology_changes.removed_fabric.iter().flat_map(|e| {
                    [
                        Created {
                            kind: "Network object".into(),
                            uuid: e.object.uuid(),
                        },
                        Created {
                            kind: "Physical interface".into(),
                            uuid: e.interface.uuid(),
                        },
                    ]
                }))
                .collect(),
        }
    }
}
fn steps(values: &[xenadmin_core::OperationStep]) -> Vec<Step> {
    values
        .iter()
        .map(|step| Step {
            label: step.label.clone(),
            state: format!("{:?}", step.state),
            detail: step.detail.clone(),
        })
        .collect()
}
/// The export in flight: one at a time.
#[derive(Default)]
pub struct Export {
    pub pending: bool,
    /// The document captured when the command ran, until the prompt is
    /// answered.
    captured: Option<Document>,
    result: Option<mpsc::Receiver<Result<String, String>>>,
}
#[derive(Serialize)]
struct Document {
    schema_version: u8,
    exported_at_ms: u128,
    scope: String,
    entries: Vec<CapturedEntry>,
}
#[derive(Serialize)]
struct CapturedEntry {
    current_connection: bool,
    entry: Entry,
}

/// What the export says it covers: the list and its filters.
pub fn scope_label(m: &Model, tab: Tab) -> String {
    let a = &m.areas.events;
    format!(
        "{}; pool {}; search {:?}; state/priority {:?}; hidden {}; batch members {}",
        tab.title(),
        a.pool
            .map(|id| id.to_string())
            .unwrap_or_else(|| "all observed pools".into()),
        a.search,
        activity::status_filter(m, tab),
        a.show_hidden,
        a.show_children
    )
}

/// Captures the listed rows of `tab` now, so that later tab or filter
/// changes do not alter the export, and asks where to save them.
pub fn choose(m: &mut Model, tab: Tab) {
    if m.areas.events.export.pending {
        return;
    }
    let entries = activity::current(m, tab)
        .rows
        .iter()
        .map(|row| CapturedEntry {
            current_connection: m.snapshot_for(row.pool).is_some_and(|snapshot| {
                snapshot.is_current() && snapshot.inventory().is_some_and(|i| i.pool_id == row.pool)
            }),
            entry: row.export.clone(),
        })
        .collect();
    let document = Document {
        schema_version: 1,
        exported_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        scope: scope_label(m, tab),
        entries,
    };
    m.areas.events.export.pending = true;
    m.areas.events.export.captured = Some(document);
    m.choose_file(
        FileRequest::save("Export activity JSON", "xenadmin-activity.json", reply)
            .on_failure(prompt_failed)
            .filter("JSON", ["json"]),
    );
}

/// The save prompt could not be shown: nothing is written, and the notice
/// says why, in the Rune words.
fn prompt_failed(m: &mut Model, error: &str) {
    if m.areas.events.export.captured.take().is_none() {
        return;
    }
    m.areas.events.export.pending = false;
    m.areas.events.notice = Some(format!("Cannot open save dialog: {error}"));
}

/// The answer of the save prompt. It runs in a later update: the export may
/// have been reset meanwhile, and then there is nothing to write.
fn reply(m: &mut Model, path: Option<std::path::PathBuf>) {
    let Some(document) = m.areas.events.export.captured.take() else {
        return;
    };
    let Some(path) = path else {
        m.areas.events.export.pending = false;
        return;
    };
    let (sender, receiver) = mpsc::channel();
    let wake = m.waker.clone();
    let spawned = std::thread::Builder::new()
        .name("activity-export".into())
        .spawn(move || {
            let result = encode_bounded(&document)
                .and_then(|bytes| write_private(&path, &bytes))
                .map(|()| format!("Activity saved to {}", path.display()));
            let _ = sender.send(result);
            if let Some(wake) = wake {
                wake();
            }
        });
    if spawned.is_ok() {
        m.areas.events.export.result = Some(receiver);
        m.areas.events.notice = Some("Saving the captured activity selection…".into());
    } else {
        m.areas.events.export.pending = false;
        m.areas.events.notice = Some("Cannot start the activity writer.".into());
    }
}

/// Takes the writer's result when it has one. Never waits.
pub fn poll(m: &mut Model) -> bool {
    let Some(receiver) = &m.areas.events.export.result else {
        return false;
    };
    let result = match receiver.try_recv() {
        Ok(value) => value,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("Activity writer stopped without a result.".into()),
    };
    m.areas.events.export = Export::default();
    m.areas.events.notice =
        Some(result.unwrap_or_else(|e| format!("Cannot export activity: {e}")));
    true
}
fn encode_bounded(document: &impl Serialize) -> Result<Vec<u8>, String> {
    struct Limited(Vec<u8>);
    impl Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > 32 * 1024 * 1024 {
                return Err(std::io::Error::other(
                    "Export exceeds 32 MiB; narrow the selection.",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Limited(Vec::new());
    serde_json::to_writer_pretty(&mut writer, document).map_err(|e| e.to_string())?;
    Ok(writer.0)
}
