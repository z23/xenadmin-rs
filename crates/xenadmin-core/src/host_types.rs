use crate::{
    ActionAvailability, Error, HostId, Inventory, MemoryLimits, PowerState, ResourceTarget, VmId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostPowerOnMode {
    Disabled,
    WakeOnLan,
}
impl HostPowerOnMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "",
            Self::WakeOnLan => "wake-on-lan",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostOperation {
    DataSource {
        expected: crate::HostDataSource,
        action: crate::HostRecordingAction,
    },
    EnterMaintenance {
        expected_enabled: bool,
        resident_vms: Vec<VmId>,
    },
    Evacuate {
        resident_vms: Vec<VmId>,
    },
    ExitMaintenance,
    Reboot {
        expected_boot_time: String,
    },
    Shutdown {
        expected_boot_time: String,
    },
    RestartToolstack {
        expected_agent_start_time: String,
    },
    PowerOn,
    ConfigurePowerOn {
        expected_mode: String,
        mode: HostPowerOnMode,
    },
    SetDom0Memory {
        control_domain: VmId,
        expected: MemoryLimits,
        memory: u64,
    },
    SetSyslog {
        expected_destination: Option<String>,
        destination: Option<String>,
    },
    SetMultipath {
        expected: bool,
        enabled: bool,
    },
    SetPoolAutostart {
        expected: bool,
        enabled: bool,
    },
    ForgetDeadHost {
        verified_power_off: bool,
    },
}
impl HostOperation {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::resources::host::requirements(self)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::DataSource { action, .. } => action.label(),
            Self::EnterMaintenance { .. } => "Enter host maintenance",
            Self::Evacuate { .. } => "Evacuate host",
            Self::ExitMaintenance => "Exit host maintenance",
            Self::Reboot { .. } => "Reboot host",
            Self::Shutdown { .. } => "Shut down host",
            Self::RestartToolstack { .. } => "Restart host toolstack",
            Self::PowerOn => "Power on host",
            Self::ConfigurePowerOn { .. } => "Configure host power-on",
            Self::SetDom0Memory { .. } => "Set control-domain memory",
            Self::SetSyslog { .. } => "Set remote logging",
            Self::SetMultipath { .. } => "Set host multipathing",
            Self::SetPoolAutostart { .. } => "Set pool VM autostart",
            Self::ForgetDeadHost { .. } => "Forget dead host record",
        }
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        if matches!(self, Self::SetPoolAutostart { .. })
            != matches!(target, ResourceTarget::Pool(_))
            || (!matches!(self, Self::SetPoolAutostart { .. })
                && !matches!(target, ResourceTarget::Host(_)))
        {
            return Err(Error::InvalidResourceCommand);
        }
        let valid = match self {
            Self::DataSource { expected, action } => {
                expected.valid()
                    && (*action == crate::HostRecordingAction::ForgetHistory || !expected.enabled)
            }
            Self::EnterMaintenance { resident_vms, .. } | Self::Evacuate { resident_vms } => {
                resident_vms.len() <= 10_000
                    && resident_vms.iter().collect::<BTreeSet<_>>().len() == resident_vms.len()
            }
            Self::Reboot { expected_boot_time } | Self::Shutdown { expected_boot_time } => {
                valid_epoch(expected_boot_time)
            }
            Self::RestartToolstack {
                expected_agent_start_time,
            } => valid_epoch(expected_agent_start_time),
            Self::ConfigurePowerOn {
                expected_mode,
                mode,
            } => {
                ["", "wake-on-lan"].contains(&expected_mode.as_str())
                    && expected_mode != mode.as_str()
            }
            Self::SetDom0Memory {
                expected, memory, ..
            } => {
                *memory > 0
                    && *memory <= i64::MAX as u64
                    && memory.is_multiple_of(1024 * 1024)
                    && *memory >= expected.static_min
                    && expected.static_min <= expected.dynamic_min
                    && expected.dynamic_min <= expected.dynamic_max
                    && expected.dynamic_max <= expected.static_max
                    && (*memory != expected.static_max
                        || *memory != expected.dynamic_min
                        || *memory != expected.dynamic_max)
            }
            Self::SetSyslog {
                expected_destination,
                destination,
            } => {
                expected_destination != destination
                    && expected_destination
                        .as_ref()
                        .is_none_or(|v| v.len() <= 1024 && !v.chars().any(char::is_control))
                    && destination.as_ref().is_none_or(|v| valid_destination(v))
            }
            Self::SetMultipath { expected, enabled }
            | Self::SetPoolAutostart { expected, enabled } => expected != enabled,
            Self::ForgetDeadHost { verified_power_off } => *verified_power_off,
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
    pub fn availability(
        &self,
        inventory: &Inventory,
        target: ResourceTarget,
    ) -> ActionAvailability {
        let unavailable = |reason: &str| ActionAvailability::Prerequisite {
            reason: reason.into(),
        };
        if self.validate(target).is_err() {
            return unavailable("Choose valid host settings and a change to apply");
        }
        if let Self::SetPoolAutostart { expected, .. } = self {
            return if matches!(target,ResourceTarget::Pool(id) if id==inventory.pool_id)
                && inventory
                    .details
                    .pool
                    .as_ref()
                    .is_some_and(|pool| pool.auto_poweron == Some(*expected))
            {
                ActionAvailability::Available
            } else {
                ActionAvailability::Unknown
            };
        }
        let ResourceTarget::Host(id) = target else {
            return ActionAvailability::Unavailable;
        };
        let Some(host) = inventory.hosts.get(&id) else {
            return ActionAvailability::Stale;
        };
        let Some(details) = inventory.details.hosts.get(&id) else {
            return ActionAvailability::Unknown;
        };
        let Some(running) = &details.current_operations else {
            return ActionAvailability::Unknown;
        };
        if !running.is_empty() {
            return ActionAvailability::Busy {
                operations: running.clone(),
            };
        }
        let live = details
            .metrics
            .and_then(|id| inventory.details.host_metrics.get(&id))
            .and_then(|metrics| metrics.live);
        let residents = inventory.host_resident_guests(id);
        let allowed = |operation: &str| {
            details
                .allowed_operations
                .as_ref()
                .is_some_and(|operations| operations.iter().any(|value| value == operation))
        };
        let empty = || !host.enabled && residents.is_empty();
        let supported = match self {
            Self::DataSource { .. } => live == Some(true),
            Self::EnterMaintenance {
                expected_enabled,
                resident_vms,
            } => {
                live == Some(true)
                    && details.maintenance_mode == Some(false)
                    && host.enabled == *expected_enabled
                    && same_vms(resident_vms, &residents)
            }
            Self::Evacuate { resident_vms } => {
                live == Some(true)
                    && !host.enabled
                    && allowed("evacuate")
                    && same_vms(resident_vms, &residents)
            }
            Self::ExitMaintenance => {
                live == Some(true)
                    && !host.enabled
                    && details.maintenance_mode == Some(true)
                    && allowed("enable")
            }
            Self::Reboot { expected_boot_time } | Self::Shutdown { expected_boot_time } => {
                live == Some(true)
                    && empty()
                    && details.boot_time.as_ref() == Some(expected_boot_time)
                    && allowed(if matches!(self, Self::Reboot { .. }) {
                        "reboot"
                    } else {
                        "shutdown"
                    })
                    && (inventory.coordinator != Some(id) || inventory.hosts.len() == 1)
            }
            Self::RestartToolstack {
                expected_agent_start_time,
            } => {
                live == Some(true)
                    && details.agent_start_time.as_ref() == Some(expected_agent_start_time)
                    && inventory
                        .details
                        .pool
                        .as_ref()
                        .is_some_and(|pool| pool.ha_enabled == Some(false))
                    && inventory
                        .details
                        .tasks
                        .values()
                        .all(|task| !matches!(task.status.as_str(), "pending" | "cancelling"))
                    && matches!(
                        inventory.details.classes.get("task"),
                        Some(crate::ClassAvailability::Available)
                    )
            }
            Self::PowerOn => {
                live == Some(false)
                    && allowed("power_on")
                    && details
                        .power_on_mode
                        .as_ref()
                        .is_some_and(|mode| !mode.is_empty())
            }
            Self::ConfigurePowerOn { expected_mode, .. } => {
                live == Some(true)
                    && details.power_on_mode.as_ref() == Some(expected_mode)
                    && details.power_on_config_empty == Some(true)
            }
            Self::SetDom0Memory {
                control_domain,
                expected,
                ..
            } => {
                live == Some(true)
                    && details.control_domain == Some(*control_domain)
                    && inventory.details.vms.get(control_domain).is_some_and(|vm| {
                        vm.memory_static_min == Some(expected.static_min)
                            && vm.memory_static_max == Some(expected.static_max)
                            && vm.memory_dynamic_min == Some(expected.dynamic_min)
                            && vm.memory_dynamic_max == Some(expected.dynamic_max)
                    })
            }
            Self::SetSyslog {
                expected_destination,
                ..
            } => {
                live == Some(true)
                    && details.syslog_known
                    && details.syslog_destination == *expected_destination
            }
            Self::SetMultipath { expected, .. } => {
                live == Some(true)
                    && empty()
                    && details.maintenance_mode == Some(true)
                    && details.multipathing == Some(*expected)
                    && inventory
                        .details
                        .pbds
                        .values()
                        .filter(|pbd| pbd.host == Some(id))
                        .all(|pbd| pbd.currently_attached == Some(false))
                    && matches!(
                        inventory.details.classes.get("pbd"),
                        Some(crate::ClassAvailability::Available)
                    )
            }
            Self::ForgetDeadHost { .. } => {
                live == Some(false)
                    && !host.enabled
                    && residents.is_empty()
                    && inventory.coordinator.is_some_and(|master| master != id)
                    && inventory
                        .details
                        .pool
                        .as_ref()
                        .is_some_and(|pool| pool.ha_enabled == Some(false))
            }
            Self::SetPoolAutostart { .. } => false,
        };
        if supported {
            ActionAvailability::Available
        } else {
            unavailable(match self {
                Self::DataSource { .. } => {
                    "The selected host must be live; refresh its data-source catalog before editing"
                }
                Self::EnterMaintenance { .. } | Self::Evacuate { .. } => {
                    "Current host state and the exact resident VM list are required; evacuation is checked again by the host"
                }
                Self::ExitMaintenance => {
                    "The host must be live, disabled and marked in maintenance, with enable advertised"
                }
                Self::Reboot { .. } | Self::Shutdown { .. } => {
                    "Disable scheduling and evacuate guests first; a multi-host pool coordinator must be handed over separately"
                }
                Self::RestartToolstack { .. } => {
                    "Pool HA must be known disabled, a current toolstack start marker must be supplied and pool tasks must be finished"
                }
                Self::PowerOn => {
                    "The host must be reported offline with power-on configured and advertised"
                }
                Self::ConfigurePowerOn { .. } => {
                    "Only empty configuration for disabled or Wake-on-LAN mode is edited here; BMC credential configuration is separate"
                }
                Self::SetDom0Memory { .. } => {
                    "The host's exact control domain and current memory limits are required"
                }
                Self::SetSyslog { .. } => {
                    "The current remote logging destination must be known and match the review"
                }
                Self::SetMultipath { .. } => {
                    "Evacuate the host, enter maintenance and detach its storage connections before changing multipathing"
                }
                Self::ForgetDeadHost { .. } => {
                    "The host must be offline, empty, not the coordinator, and HA must be disabled; independently verify power is off"
                }
                Self::SetPoolAutostart { .. } => "The current pool autostart value must be known",
            })
        }
    }
}
pub(crate) fn valid_epoch(value: &str) -> bool {
    value.len() <= 64 && value.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0)
}
pub(crate) fn valid_destination(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.chars().any(char::is_whitespace)
        && (value.parse::<std::net::IpAddr>().is_ok()
            || value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            }))
}
pub(crate) fn same_vms(a: &[VmId], b: &[VmId]) -> bool {
    a.iter().collect::<BTreeSet<_>>() == b.iter().collect::<BTreeSet<_>>()
}
impl Inventory {
    pub fn host_resident_guests(&self, host: HostId) -> Vec<VmId> {
        self.vms
            .values()
            .filter(|vm| {
                vm.host == Some(host)
                    && !vm.is_control_domain
                    && !vm.is_template
                    && matches!(vm.power_state, PowerState::Running | PowerState::Paused)
            })
            .map(|vm| vm.id)
            .collect()
    }
}
