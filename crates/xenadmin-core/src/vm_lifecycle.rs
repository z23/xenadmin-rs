use crate::{
    ActionAvailability, Error, Inventory, NetworkId, PowerState, SnapshotId, SrId, VdiId, VmId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use xenadmin_xapi::enumerations::{storage_operations, vbd_type, vdi_type, vm_operations};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuestFirmware {
    #[default]
    Inherited,
    Bios,
    Uefi,
    SecureUefi,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmCopyMode {
    Fast,
    Full { sr: SrId },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotDeletionSelection {
    pub snapshot: SnapshotId,
    pub disks: Vec<VdiId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "vm_lifecycle", rename_all = "snake_case")]
pub enum VmLifecycleOperation {
    Create {
        name: String,
        description: String,
        sr: SrId,
        network: NetworkId,
        iso: Option<VdiId>,
        vcpus: u64,
        memory: u64,
        disk_size: Option<u64>,
        firmware: GuestFirmware,
        #[serde(default)]
        bios_host: Option<crate::HostId>,
    },
    Clone {
        name: String,
        expected_template: bool,
        mode: VmCopyMode,
        as_template: bool,
    },
    ConvertToTemplate,
    EditTemplate {
        name: String,
        description: String,
    },
    Delete {
        expected_template: bool,
        disks: Vec<VdiId>,
        snapshots: Vec<SnapshotDeletionSelection>,
    },
}
impl VmLifecycleOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create VM from template",
            Self::Clone {
                mode: VmCopyMode::Fast,
                ..
            } => "Fast clone VM or template",
            Self::Clone { .. } => "Full copy VM or template",
            Self::ConvertToTemplate => "Convert VM to template",
            Self::EditTemplate { .. } => "Edit template",
            Self::Delete { .. } => "Delete VM or template",
        }
    }
    pub(crate) fn template(&self) -> bool {
        match self {
            Self::Create { .. } | Self::EditTemplate { .. } => true,
            Self::Clone {
                expected_template, ..
            }
            | Self::Delete {
                expected_template, ..
            } => *expected_template,
            Self::ConvertToTemplate => false,
        }
    }
    pub(crate) fn capability(&self) -> Option<&'static str> {
        match self {
            Self::Create { .. } => Some(vm_operations::CLONE),
            Self::Clone {
                mode: VmCopyMode::Fast,
                ..
            } => Some(vm_operations::CLONE),
            Self::Clone { .. } => Some(vm_operations::COPY),
            Self::ConvertToTemplate => Some(vm_operations::MAKE_INTO_TEMPLATE),
            Self::Delete { .. } => Some(vm_operations::DESTROY),
            Self::EditTemplate { .. } => None,
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        let valid = match self {
            Self::Create {
                name,
                description,
                vcpus,
                memory,
                disk_size,
                ..
            } => {
                name_valid(name)
                    && description.len() <= 4096
                    && (1..=256).contains(vcpus)
                    && *memory >= 16 * 1024 * 1024
                    && *memory <= i64::MAX as u64
                    && memory % 1024 == 0
                    && disk_size.is_none_or(|n| n > 0 && n <= i64::MAX as u64 && n % 512 == 0)
            }
            Self::Clone { name, .. } => name_valid(name),
            Self::EditTemplate { name, description } => {
                name_valid(name) && description.len() <= 4096
            }
            Self::Delete {
                disks, snapshots, ..
            } => {
                disks.len() <= 256
                    && snapshots.len() <= 256
                    && disks.iter().collect::<BTreeSet<_>>().len() == disks.len()
                    && snapshots
                        .iter()
                        .map(|s| s.snapshot)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == snapshots.len()
                    && snapshots.iter().all(|s| {
                        s.disks.len() <= 256
                            && s.disks.iter().collect::<BTreeSet<_>>().len() == s.disks.len()
                    })
            }
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidVmLifecycle)
        }
    }
    pub fn availability(&self, inventory: &Inventory, vm: VmId) -> ActionAvailability {
        if self.validate().is_err() {
            return ActionAvailability::Prerequisite {
                reason: "Enter valid names, resource sizes and unique deletion targets".into(),
            };
        }
        let Some(guest) = inventory.vms.get(&vm) else {
            return ActionAvailability::Stale;
        };
        if guest.is_control_domain || guest.is_template != self.template() {
            return ActionAvailability::Stale;
        }
        if guest.power_state != PowerState::Halted {
            return ActionAvailability::Prerequisite {
                reason: "Shut down the source VM before this operation".into(),
            };
        }
        let required = if matches!(self, Self::Create { .. } | Self::Clone { .. }) {
            &["vbd", "vdi", "vif", "sr", "network"][..]
        } else if matches!(self,Self::Delete{disks,snapshots,..} if !disks.is_empty() || snapshots.iter().any(|s|!s.disks.is_empty()))
        {
            &["vbd", "vdi"][..]
        } else {
            &[][..]
        };
        if required.iter().any(|class| {
            inventory.details.classes.get(*class) != Some(&crate::ClassAvailability::Available)
        }) {
            return ActionAvailability::Unknown;
        }
        let Some(details) = inventory.details.vms.get(&vm) else {
            return ActionAvailability::Unknown;
        };
        if matches!(self, Self::Delete { .. } | Self::EditTemplate { .. })
            && details.is_default_template != Some(false)
        {
            return ActionAvailability::Prerequisite {
                reason: "Built-in templates cannot be edited or deleted here; copy them first"
                    .into(),
            };
        }
        if !details.current_operations.is_empty() {
            return ActionAvailability::Busy {
                operations: details.current_operations.clone(),
            };
        }
        let capability = match self {
            Self::Create { .. }
                if inventory.details.vm_disks.get(&vm).is_some_and(|ids| {
                    ids.iter().any(|id| {
                        inventory
                            .details
                            .vbds
                            .get(id)
                            .is_some_and(|v| v.kind == vbd_type::DISK)
                    })
                }) =>
            {
                Some(vm_operations::COPY)
            }
            _ => self.capability(),
        };
        if let Some(cap) = capability {
            if let Some(reason) = details.blocked_operations.get(cap) {
                return ActionAvailability::Blocked {
                    reason: reason.clone(),
                };
            }
            let Some(allowed) = &details.allowed_operations else {
                return ActionAvailability::Unknown;
            };
            if !allowed.iter().any(|v| v == cap) {
                return ActionAvailability::Unavailable;
            }
        }
        if let Self::Delete {
            disks, snapshots, ..
        } = self
        {
            let candidates = inventory.vm_deletion_disks(vm);
            if disks.iter().any(|disk| !candidates.contains(disk)) {
                return ActionAvailability::Prerequisite {
                    reason: "Only exclusively owned disks can be selected for data deletion".into(),
                };
            }
            if snapshots.iter().any(|selection| {
                inventory
                    .snapshots
                    .get(&selection.snapshot)
                    .is_none_or(|s| {
                        s.vm != Some(vm)
                            || s.power_state != PowerState::Halted
                            || !s.allowed_actions.contains(&crate::SnapshotAction::Delete)
                    })
                    || selection.disks.iter().any(|d| {
                        !inventory
                            .snapshot_deletion_disks(selection.snapshot)
                            .contains(d)
                    })
            }) {
                return ActionAvailability::Prerequisite{reason:"A selected snapshot changed or needs its separate memory-checkpoint deletion flow".into()};
            }
        }
        let sr = match self {
            Self::Create { sr, .. }
            | Self::Clone {
                mode: VmCopyMode::Full { sr },
                ..
            } => Some(sr),
            _ => None,
        };
        if sr.is_some_and(|id| {
            inventory.details.storage.get(id).is_none_or(|s| {
                s.content_type != "user"
                    || s.allowed_operations.as_ref().is_none_or(|ops| {
                        !ops.iter().any(|op| op == storage_operations::VDI_CREATE)
                    })
            })
        }) {
            return ActionAvailability::Unavailable;
        }
        if let Self::Create { network, iso, .. } = self
            && (!inventory.details.networks.contains_key(network)
                || iso.is_some_and(|id| {
                    inventory.details.vdis.get(&id).is_none_or(|v| {
                        v.kind != "iso" || v.read_only != Some(true) || v.missing != Some(false)
                    })
                }))
        {
            return ActionAvailability::Unavailable;
        }
        ActionAvailability::Available
    }
    pub(crate) fn resources(&self) -> Vec<String> {
        match self {
            Self::Create {
                sr,
                network,
                iso,
                bios_host,
                ..
            } => {
                let mut r = vec![format!("sr:{sr}"), format!("network:{network}")];
                r.extend(iso.iter().map(|id| format!("vdi:{id}")));
                r.extend(bios_host.iter().map(|id| format!("host:{id}")));
                r
            }
            Self::Clone {
                mode: VmCopyMode::Full { sr },
                ..
            } => vec![format!("sr:{sr}")],
            Self::Delete {
                disks, snapshots, ..
            } => disks
                .iter()
                .map(|id| format!("vdi:{id}"))
                .chain(snapshots.iter().flat_map(|s| {
                    std::iter::once(s.snapshot.to_string())
                        .chain(s.disks.iter().map(|id| format!("vdi:{id}")))
                }))
                .collect(),
            _ => vec![],
        }
    }
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 255 && !name.chars().any(char::is_control)
}
impl Inventory {
    pub fn vm_deletion_disks(&self, vm: VmId) -> Vec<VdiId> {
        if ["vdi", "vbd"].iter().any(|class| {
            self.details.classes.get(*class) != Some(&crate::ClassAvailability::Available)
        }) {
            return vec![];
        }
        self.details
            .vdis
            .values()
            .filter(|d| {
                d.kind == vdi_type::USER
                    && d.is_snapshot == Some(false)
                    && d.managed == Some(true)
                    && d.missing == Some(false)
                    && d.sharable == Some(false)
                    && self.details.vdi_attachments.get(&d.id).is_some_and(|ids| {
                        !ids.is_empty()
                            && ids.iter().all(|id| {
                                self.details.vbds.get(id).is_some_and(|v| {
                                    v.vm == Some(vm) && v.kind == vbd_type::DISK && v.owned
                                })
                            })
                    })
            })
            .map(|d| d.id)
            .collect()
    }
}
