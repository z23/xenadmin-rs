use super::*;
use serde::Serialize;
use std::{io::Write, path::Path, sync::mpsc};
use xenadmin_core::{OperationRecord, ResourceOperationRecord, ServerMessage, ServerTask};

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Entry {
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
pub(super) struct Step {
    label: String,
    state: String,
    detail: String,
}
#[derive(Clone, Serialize)]
pub(super) struct Created {
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
                .map(|target| Created {
                    kind: "Resource".into(),
                    uuid: target.uuid(),
                })
                .into_iter()
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
#[derive(Default)]
pub(super) struct Export {
    pub pending: bool,
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

pub(super) fn choose(s: &mut State, tx: &mut Tx) {
    if s.activity.export.pending {
        return;
    }
    let entries = s
        .activity
        .rows
        .iter()
        .map(|row| CapturedEntry {
            current_connection: s.snapshot.as_ref().is_some_and(|snapshot| {
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
        scope: s.activity.scope_label(),
        entries,
    };
    s.activity.export.pending = true;
    tx.save_file(
        rune::FileDialogOptions {
            title: "Export activity JSON".into(),
            file_name: Some("xenadmin-activity.json".into()),
            ..Default::default()
        }
        .filter("JSON", &["json"]),
        move |s: &mut State, result, _| match result {
            Ok(Some(path)) => {
                let (sender, receiver) = mpsc::channel();
                let wake = s.waker.clone();
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
                    s.activity.export.result = Some(receiver);
                    s.activity.notice = Some("Saving the captured activity selection…".into());
                } else {
                    s.activity.export.pending = false;
                    s.activity.notice = Some("Cannot start the activity writer.".into());
                }
            }
            Ok(None) => s.activity.export.pending = false,
            Err(error) => {
                s.activity.export.pending = false;
                s.activity.notice = Some(format!("Cannot open save dialog: {error}"));
            }
        },
    );
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(receiver) = &s.activity.export.result else {
        return false;
    };
    let result = match receiver.try_recv() {
        Ok(value) => value,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("Activity writer stopped without a result.".into()),
    };
    s.activity.export = Export::default();
    s.activity.notice = Some(result.unwrap_or_else(|e| format!("Cannot export activity: {e}")));
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
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Export exceeds 32 MiB; narrow the selection.".into());
    }
    if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
        return Err("Choose a regular file destination.".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = parent.join(format!(".xenadmin-activity-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
