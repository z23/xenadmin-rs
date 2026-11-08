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
    ExportOvf {
        source: ExportSource,
        format: crate::OvfPackageFormat,
    },
    ExportXva {
        source: ExportSource,
    },
    ImportXva {
        sr: SrId,
    },
    ImportOvf {
        sr: SrId,
        networks: std::collections::BTreeMap<String, crate::NetworkId>,
    },
    ExportDisk {
        vdi: crate::VdiId,
        expected_sr: SrId,
        expected_size: u64,
        format: crate::DiskImageFormat,
    },
    ImportDisk {
        sr: SrId,
        name: String,
        format: crate::DiskImageFormat,
    },
}
impl TransferAction {
    pub fn is_export(&self) -> bool {
        matches!(
            self,
            Self::ExportXva { .. } | Self::ExportDisk { .. } | Self::ExportOvf { .. }
        )
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::ExportXva { .. } => "Export XVA",
            Self::ExportOvf { .. } => "Export OVF/OVA",
            Self::ImportXva { .. } => "Import XVA",
            Self::ImportOvf { .. } => "Import OVF/OVA",
            Self::ExportDisk { .. } => "Export disk image",
            Self::ImportDisk { .. } => "Import disk image",
        }
    }
    pub fn availability(&self, i: &Inventory) -> ActionAvailability {
        if let Self::ExportOvf { source, .. } = self {
            return Self::ExportXva {
                source: source.clone(),
            }
            .availability(i);
        }
        let no = |s: &str| ActionAvailability::Prerequisite { reason: s.into() };
        if let Self::ImportDisk { name, .. } = self
            && (name.trim().is_empty()
                || name.chars().count() > 255
                || name.chars().any(char::is_control))
        {
            return no("Enter a new disk name of 1–255 printable characters");
        };
        match self {
            Self::ExportOvf { .. } => unreachable!(),
            Self::ExportDisk {
                vdi,
                expected_sr,
                expected_size,
                format,
            } => return disk::availability(i, *vdi, *expected_sr, *expected_size, *format),
            Self::ImportXva { sr } | Self::ImportDisk { sr, .. } | Self::ImportOvf { sr, .. } => {
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
        if let Self::ImportOvf { networks, .. } = self
            && (networks.len() > 64
                || networks
                    .values()
                    .any(|n| !i.details.networks.contains_key(n)))
        {
            return no("Map every package network to a current pool network");
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
    #[serde(default)]
    pub(crate) disk: Option<crate::DiskImageSummary>,
    #[serde(default)]
    pub(crate) range: Option<files::FileRange>,
    #[serde(default)]
    pub(crate) ovf: Option<Box<crate::ovf::Selection>>,
}
impl TransferFile {
    pub fn ovf_source(path: impl AsRef<Path>) -> Result<Self, Error> {
        crate::ovf::select(path.as_ref())
    }
    pub fn ovf_source_with_cancel(
        path: impl AsRef<Path>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Self, Error> {
        crate::ovf::package::select_cancel(path.as_ref(), cancel)
    }
    pub fn ovf(&self) -> Option<&crate::OvfSummary> {
        self.ovf.as_ref().map(|v| &v.summary)
    }
    pub fn source(path: impl AsRef<Path>) -> Result<Self, Error> {
        files::select_source(path.as_ref())
    }
    pub fn disk_source(
        path: impl AsRef<Path>,
        format: crate::DiskImageFormat,
    ) -> Result<Self, Error> {
        files::select_disk_source(path.as_ref(), format)
    }
    pub fn disk_image(&self) -> Option<&crate::DiskImageSummary> {
        self.disk.as_ref()
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
        self.range
            .as_ref()
            .map(|r| r.length)
            .or_else(|| self.identity.as_ref().map(|v| v.size))
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
    #[serde(default)]
    pub vbds: Vec<crate::VbdId>,
    #[serde(default)]
    pub vifs: Vec<crate::VifId>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub file: TransferFile,
    #[serde(default)]
    pub ovf_import: Option<Box<crate::ovf::import::ImportPlan>>,
    #[serde(default)]
    pub ovf_export: Option<Box<crate::ovf::export::ExportPlan>>,
    #[serde(default)]
    pub disk_creation: Option<disk::Creation>,
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
            ovf_import: None,
            ovf_export: None,
            disk_creation: None,
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
            plan_accepts(a, b)
                && old
                    .transfer_created
                    .vbds
                    .iter()
                    .all(|id| new.transfer_created.vbds.contains(id))
                && old
                    .transfer_created
                    .vifs
                    .iter()
                    .all(|id| new.transfer_created.vifs.contains(id))
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

#[path = "transfer_disk.rs"]
pub(crate) mod disk;

pub(crate) fn plan_accepts(a: &Plan, b: &Plan) -> bool {
    (match (&a.ovf_import, &b.ovf_import) {
        (None, _) => true,
        (Some(a), Some(b)) => a.accepts(b),
        _ => false,
    }) && (match (&a.ovf_export, &b.ovf_export) {
        (None, _) => true,
        (Some(a), Some(b)) => a.accepts(b),
        _ => false,
    }) && a.file == b.file
        && disk::valid_creation_update(a.disk_creation.as_ref(), b.disk_creation.as_ref())
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
}
