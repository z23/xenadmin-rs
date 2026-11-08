use crate::{Error, HostId, ResourceTarget, SrId, VmId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HaRestartPriority {
    Disabled,
    BestEffort,
    Restart,
}
impl HaRestartPriority {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "" => Some(Self::Disabled),
            "best-effort" => Some(Self::BestEffort),
            "restart" => Some(Self::Restart),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "",
            Self::BestEffort => "best-effort",
            Self::Restart => "restart",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Do not restart",
            Self::BestEffort => "Best effort",
            Self::Restart => "Restart",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmStartupField {
    Order,
    StartDelay,
    ShutdownDelay,
}
impl VmStartupField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Order => "Startup order",
            Self::StartDelay => "Start delay",
            Self::ShutdownDelay => "Shutdown delay",
        }
    }
    pub(crate) fn field(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::StartDelay => "start_delay",
            Self::ShutdownDelay => "shutdown_delay",
        }
    }
    fn method(self) -> &'static str {
        match self {
            Self::Order => "VM.set_order",
            Self::StartDelay => "VM.set_start_delay",
            Self::ShutdownDelay => "VM.set_shutdown_delay",
        }
    }
    pub(crate) fn valid(self, value: u64) -> bool {
        value
            <= if self == Self::Order {
                i64::MAX as u64
            } else {
                86_400
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolOperation {
    Join {
        source_host: HostId,
        destination_pool: crate::PoolId,
        destination_coordinator: HostId,
        destination_address: String,
    },
    Security(crate::PoolSecurityOperation),
    SetMigrationCompression {
        expected: bool,
        enabled: bool,
    },
    SetHttpsOnly {
        expected: bool,
        enabled: bool,
    },
    DesignateCoordinator {
        expected: HostId,
        host: HostId,
    },
    SetHaFailuresToTolerate {
        expected: u64,
        failures: u64,
    },
    EnableHa {
        heartbeat_srs: Vec<SrId>,
        expected_failures: u64,
    },
    DisableHa,
    SetHaAllowOvercommit {
        expected: bool,
        enabled: bool,
    },
    SetHaTimeout {
        expected: Option<String>,
        seconds: Option<u32>,
    },
    SetVmStartup {
        vm: VmId,
        appliance: Option<crate::ApplianceId>,
        field: VmStartupField,
        expected: u64,
        value: u64,
    },
    SetVmRestartPriority {
        vm: VmId,
        expected: HaRestartPriority,
        priority: HaRestartPriority,
    },
    EjectHost {
        host: HostId,
        expected_name: String,
    },
    EnableTlsVerification,
    RotateSecret,
}
impl PoolOperation {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        let required = crate::PermissionRequirements::writes([self.method()]);
        if matches!(self, Self::Join { .. }) {
            required.with_host_dependent_steps()
        } else {
            required
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Join { .. } => "Join destination pool",
            Self::Security(op) => op.label(),
            Self::SetMigrationCompression { .. } => "Set migration compression",
            Self::SetHttpsOnly { .. } => "Set HTTPS-only access",
            Self::DesignateCoordinator { .. } => "Hand over pool coordinator",
            Self::SetHaFailuresToTolerate { .. } => "Set HA failure tolerance",
            Self::EnableHa { .. } => "Enable high availability",
            Self::DisableHa => "Disable high availability",
            Self::SetHaAllowOvercommit { .. } => "Set HA overcommit policy",
            Self::SetHaTimeout { .. } => "Set HA heartbeat timeout default",
            Self::SetVmStartup { field, .. } => field.label(),
            Self::SetVmRestartPriority { .. } => "Set VM HA restart priority",
            Self::EjectHost { .. } => "Eject pool member",
            Self::EnableTlsVerification => "Enable pool TLS verification",
            Self::RotateSecret => "Rotate pool authentication secret",
        }
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Join { .. } => "pool.join",
            Self::Security(op) => op.method(),
            Self::SetMigrationCompression { .. } => "pool.set_migration_compression",
            Self::SetHttpsOnly { .. } => "Async.pool.set_https_only",
            Self::DesignateCoordinator { .. } => "Async.pool.designate_new_master",
            Self::SetHaFailuresToTolerate { .. } => "Async.pool.set_ha_host_failures_to_tolerate",
            Self::EnableHa { .. } => "Async.pool.enable_ha",
            Self::DisableHa => "Async.pool.disable_ha",
            Self::SetHaAllowOvercommit { .. } => "pool.set_ha_allow_overcommit",
            Self::SetHaTimeout { .. } => "pool.set_other_config",
            Self::SetVmStartup { field, .. } => field.method(),
            Self::SetVmRestartPriority { .. } => "VM.set_ha_restart_priority",
            Self::EjectHost { .. } => "Async.pool.eject",
            Self::EnableTlsVerification => "pool.enable_tls_verification",
            Self::RotateSecret => "Async.pool.rotate_secret",
        }
    }
    pub(crate) fn asynchronous(&self) -> bool {
        self.method().starts_with("Async.")
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let valid = matches!(target, ResourceTarget::Pool(_))
            && match self {
                Self::Join {
                    source_host,
                    destination_pool,
                    destination_coordinator,
                    destination_address,
                } => {
                    target != ResourceTarget::Pool(*destination_pool)
                        && source_host != destination_coordinator
                        && crate::pool_join::valid_address(destination_address)
                }
                Self::Security(op) => op.valid(),
                Self::SetMigrationCompression { expected, enabled }
                | Self::SetHttpsOnly { expected, enabled } => expected != enabled,
                Self::DesignateCoordinator { expected, host } => expected != host,
                Self::SetHaFailuresToTolerate { expected, failures } => {
                    expected != failures && *expected <= 255 && *failures <= 255
                }
                Self::EnableHa {
                    heartbeat_srs,
                    expected_failures,
                } => {
                    !heartbeat_srs.is_empty()
                        && heartbeat_srs.len() <= 8
                        && *expected_failures <= 255
                        && heartbeat_srs
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == heartbeat_srs.len()
                }
                Self::SetVmRestartPriority {
                    expected, priority, ..
                } => expected != priority,
                Self::SetHaAllowOvercommit { expected, enabled } => expected != enabled,
                Self::SetHaTimeout { expected, seconds } => {
                    expected
                        .as_ref()
                        .is_none_or(|v| v.len() <= 64 && !v.chars().any(char::is_control))
                        && seconds.is_none_or(|v| (10..=86_400).contains(&v))
                        && *expected != seconds.map(|v| v.to_string())
                }
                Self::SetVmStartup {
                    field,
                    expected,
                    value,
                    ..
                } => expected != value && field.valid(*expected) && field.valid(*value),
                Self::EjectHost { expected_name, .. } => {
                    expected_name.len() <= 1024 && !expected_name.chars().any(char::is_control)
                }
                _ => true,
            };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
}

/// Missing fields remain unknown; they are never interpreted as disabled.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PoolAdministration {
    pub coordinator: Option<HostId>,
    pub ha_enabled: Option<bool>,
    pub ha_failures_to_tolerate: Option<u64>,
    pub ha_plan_exists_for: Option<u64>,
    pub ha_overcommitted: Option<bool>,
    pub ha_allow_overcommit: Option<bool>,
    #[serde(default)]
    pub ha_timeout_known: bool,
    #[serde(default)]
    pub ha_timeout_override: Option<String>,
    #[serde(default)]
    pub ha_configuration_timeout: Option<String>,
    pub heartbeat_srs: Vec<SrId>,
    pub heartbeat_srs_known: bool,
    pub migration_compression: Option<bool>,
    pub https_only: Option<bool>,
    pub tls_verification_enabled: Option<bool>,
    pub secret_rotation_pending: Option<bool>,
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    #[test]
    fn ha_timeout_and_shared_startup_order_bounds_match_supported_contract() {
        let pool = ResourceTarget::Pool(uuid::Uuid::from_u128(1).into());
        for (seconds, valid) in [(9, false), (10, true), (86400, true), (86401, false)] {
            assert_eq!(
                PoolOperation::SetHaTimeout {
                    expected: None,
                    seconds: Some(seconds)
                }
                .validate(pool)
                .is_ok(),
                valid
            );
        }
        assert!(
            crate::ApplianceOrder {
                order: -1,
                start_delay: 0,
                shutdown_delay: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            crate::ApplianceOrder {
                order: 0,
                start_delay: 0,
                shutdown_delay: 0
            }
            .validate()
            .is_ok()
        );
    }
}
