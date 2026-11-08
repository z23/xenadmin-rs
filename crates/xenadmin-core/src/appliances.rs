use crate::{ApplianceId, Error, PowerState, ResourceTarget, VmId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplianceOrder {
    pub order: i64,
    pub start_delay: u64,
    pub shutdown_delay: u64,
}
impl ApplianceOrder {
    pub fn validate(self) -> Result<(), Error> {
        if self.order >= 0 && self.start_delay <= 86_400 && self.shutdown_delay <= 86_400 {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppliancePower {
    Start,
    CleanShutdown,
    HardShutdown,
}
impl AppliancePower {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::CleanShutdown => "clean_shutdown",
            Self::HardShutdown => "hard_shutdown",
        }
    }
    pub(crate) fn destination(self) -> PowerState {
        match self {
            Self::Start => PowerState::Running,
            _ => PowerState::Halted,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplianceMember {
    pub vm: VmId,
    pub state: PowerState,
    pub order: ApplianceOrder,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplianceOperation {
    Create {
        name: String,
        description: String,
    },
    Rename {
        appliance: ApplianceId,
        previous_name: String,
        previous_description: String,
        name: String,
        description: String,
    },
    DeleteEmpty {
        appliance: ApplianceId,
        expected_name: String,
    },
    Membership {
        appliance: ApplianceId,
        expected_members: Vec<VmId>,
        vm: VmId,
        join: bool,
    },
    SetOrder {
        appliance: ApplianceId,
        vm: VmId,
        previous: ApplianceOrder,
        value: ApplianceOrder,
    },
    Power {
        appliance: ApplianceId,
        members: Vec<ApplianceMember>,
        action: AppliancePower,
    },
}
impl ApplianceOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create vApp",
            Self::Rename { .. } => "Rename vApp",
            Self::DeleteEmpty { .. } => "Delete empty vApp",
            Self::Membership { join: true, .. } => "Add VM to vApp",
            Self::Membership { join: false, .. } => "Remove VM from vApp",
            Self::SetOrder { .. } => "Set vApp order and delays",
            Self::Power {
                action: AppliancePower::Start,
                ..
            } => "Start vApp",
            Self::Power {
                action: AppliancePower::CleanShutdown,
                ..
            } => "Shut down vApp",
            Self::Power {
                action: AppliancePower::HardShutdown,
                ..
            } => "Force shut down vApp",
        }
    }
    pub(crate) fn asynchronous(&self) -> bool {
        matches!(
            self,
            Self::Create { .. } | Self::DeleteEmpty { .. } | Self::Power { .. }
        )
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let name = |s: &str| {
            !s.trim().is_empty() && s.chars().count() <= 255 && !s.chars().any(char::is_control)
        };
        let desc = |s: &str| s.len() <= 16_384 && !s.contains('\0');
        let valid = matches!(target, ResourceTarget::Pool(_))
            && match self {
                Self::Create {
                    name: n,
                    description: d,
                } => name(n) && desc(d),
                Self::Rename {
                    previous_name,
                    previous_description,
                    name: n,
                    description: d,
                    ..
                } => {
                    name(n)
                        && desc(d)
                        && desc(previous_name)
                        && desc(previous_description)
                        && (n != previous_name || d != previous_description)
                }
                Self::DeleteEmpty { expected_name, .. } => desc(expected_name),
                Self::Membership {
                    expected_members,
                    vm,
                    join,
                    ..
                } => {
                    expected_members.len() <= 256
                        && expected_members.iter().collect::<BTreeSet<_>>().len()
                            == expected_members.len()
                        && expected_members.contains(vm) != *join
                }
                Self::SetOrder {
                    previous, value, ..
                } => previous != value && value.validate().is_ok() && previous.validate().is_ok(),
                Self::Power { members, .. } => {
                    !members.is_empty()
                        && members.len() <= 256
                        && members.iter().map(|m| m.vm).collect::<BTreeSet<_>>().len()
                            == members.len()
                        && members.iter().all(|m| m.order.validate().is_ok())
                }
            };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}
