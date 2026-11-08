use crate::{ActionAvailability, Error, Inventory, SnapshotId, SrId, VmId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[path = "transfer_archive.rs"]
mod archive;
#[path = "transfer_files.rs"]
pub(crate) mod files;
#[path = "transfer_http.rs"]
pub(crate) mod http;
#[path = "transfer_xapi.rs"]
pub(crate) mod xapi;

pub const MAX_TRANSFER_BYTES: u64 = 8 * 1024 * 1024 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportSource {
    Vm { vm: VmId, expected_template: bool },
    Snapshot { snapshot: SnapshotId, parent: VmId },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferAction {
    ExportXva { source: ExportSource },
    ImportXva { sr: SrId },
}
impl TransferAction {
    pub fn is_export(&self) -> bool {
        matches!(self, Self::ExportXva { .. })
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::ExportXva { .. } => "Export XVA",
            Self::ImportXva { .. } => "Import XVA",
        }
    }
    pub fn availability(&self, i: &Inventory) -> ActionAvailability {
        let no = |s: &str| ActionAvailability::Prerequisite { reason: s.into() };
        match self {
            Self::ImportXva { sr } => {
                if i.details.storage.get(sr).is_none_or(|r| {
                    r.content_type != "user"
                        || r.allowed_operations
                            .as_ref()
                            .is_none_or(|ops| !ops.iter().any(|o| o == "vdi_create"))
                }) {
                    return no("Choose a storage repository that permits creating data disks");
                }
                if !i
                    .details
                    .pbds
                    .values()
                    .any(|p| p.sr == Some(*sr) && p.currently_attached == Some(true))
                {
                    return no("The selected storage repository has no attached host");
                }
            }
            Self::ExportXva {
                source:
                    ExportSource::Vm {
                        vm,
                        expected_template,
                    },
            } => {
                let Some(v) = i
                    .vms
                    .get(vm)
                    .filter(|v| !v.is_control_domain && v.is_template == *expected_template)
                else {
                    return ActionAvailability::Stale;
                };
                if v.power_state != crate::PowerState::Halted {
                    return no("Shut down this VM or export a disk snapshot instead");
                }
                let Some(d) = i.details.vms.get(vm) else {
                    return ActionAvailability::Unknown;
                };
                if !d.current_operations.is_empty() {
                    return ActionAvailability::Busy {
                        operations: d.current_operations.clone(),
                    };
                }
                if let Some(reason) = d.blocked_operations.get("export") {
                    return ActionAvailability::Blocked {
                        reason: reason.clone(),
                    };
                }
                if d.allowed_operations
                    .as_ref()
                    .is_none_or(|a| !a.iter().any(|s| s == "export"))
                {
                    return ActionAvailability::Unavailable;
                }
            }
            Self::ExportXva {
                source: ExportSource::Snapshot { snapshot, parent },
            } => {
                if i.snapshots.get(snapshot).is_none_or(|s| {
                    s.vm != Some(*parent) || s.power_state != crate::PowerState::Halted
                }) {
                    return no(
                        "Choose a current disk snapshot; memory-checkpoint export is not supported here",
                    );
                }
            }
        }
        ActionAvailability::Available
    }
}
/// Selected local file identity. Paths are never included in operation actions or diagnostic projections.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferFile {
    pub(crate) path: PathBuf,
    pub(crate) identity: Option<files::Identity>,
    pub(crate) parent: files::Identity,
    pub(crate) manifest: Option<ArchiveSummary>,
}
impl TransferFile {
    pub fn source(path: impl AsRef<Path>) -> Result<Self, Error> {
        files::select_source(path.as_ref())
    }
    pub fn destination(path: impl AsRef<Path>) -> Result<Self, Error> {
        files::select_destination(path.as_ref())
    }
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
    pub fn size(&self) -> Option<u64> {
        self.identity.as_ref().map(|v| v.size)
    }
    pub fn archive(&self) -> Option<&ArchiveSummary> {
        self.manifest.as_ref()
    }
}
#[derive(Clone)]
pub struct TransferRequest {
    pub action: TransferAction,
    pub file: TransferFile,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveGuest {
    pub uuid: uuid::Uuid,
    pub name: String,
    pub template: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveSummary {
    pub guests: Vec<ArchiveGuest>,
    pub data_disks: usize,
    pub virtual_bytes: u64,
    pub networks: usize,
    pub(crate) metadata_digest: [u8; 32],
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferProgress {
    pub bytes: u64,
    pub total: Option<u64>,
    pub stream_complete: bool,
    pub phase: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferCreated {
    pub vms: Vec<VmId>,
    pub disks: Vec<crate::VdiId>,
    pub networks: Vec<crate::NetworkId>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub file: TransferFile,
    pub temporary: Option<PathBuf>,
    pub temporary_identity: Option<files::Identity>,
    pub baseline: Option<xapi::Baseline>,
    pub dispatched: bool,
    #[serde(default)]
    pub stream_invalid: bool,
    pub stream_receipt: Option<files::Receipt>,
    pub task_uuid: Option<uuid::Uuid>,
    pub task_success: bool,
    pub task_failed: bool,
    pub task_cancelled: bool,
    pub result: Option<Vec<String>>,
    pub expires_at_ms: u64,
}
impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferPlan")
            .field("dispatched", &self.dispatched)
            .field("task_success", &self.task_success)
            .finish_non_exhaustive()
    }
}
impl Plan {
    pub fn new(file: TransferFile) -> Self {
        Self {
            file,
            temporary: None,
            temporary_identity: None,
            baseline: None,
            dispatched: false,
            stream_invalid: false,
            stream_receipt: None,
            task_uuid: None,
            task_success: false,
            task_failed: false,
            task_cancelled: false,
            result: None,
            expires_at_ms: 0,
        }
    }
}

pub(crate) fn valid_update(
    old: &crate::ResourceOperationRecord,
    new: &crate::ResourceOperationRecord,
) -> bool {
    match (&old.transfer_plan, &new.transfer_plan) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.file == b.file
                && (a.baseline.is_none() || a.baseline == b.baseline)
                && (a.temporary.is_none() || a.temporary == b.temporary)
                && (a.temporary_identity.is_none() || a.temporary_identity == b.temporary_identity)
                && (!a.dispatched || b.dispatched)
                && (!a.stream_invalid || b.stream_invalid)
                && (a.task_uuid.is_none() || a.task_uuid == b.task_uuid)
                && (!a.task_success || b.task_success)
                && (!a.task_failed || b.task_failed)
                && (!a.task_cancelled || b.task_cancelled)
                && (a.result.is_none() || a.result == b.result)
                && (a.stream_receipt.is_none() || a.stream_receipt == b.stream_receipt)
                && (a.expires_at_ms == 0 || a.expires_at_ms == b.expires_at_ms)
                && old
                    .transfer_created
                    .vms
                    .iter()
                    .all(|id| new.transfer_created.vms.contains(id))
                && old
                    .transfer_created
                    .disks
                    .iter()
                    .all(|id| new.transfer_created.disks.contains(id))
                && old
                    .transfer_created
                    .networks
                    .iter()
                    .all(|id| new.transfer_created.networks.contains(id))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
