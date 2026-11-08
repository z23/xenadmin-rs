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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolOperation {
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
    pub fn label(&self) -> &'static str {
        match self {
            Self::Security(op) => op.label(),
            Self::SetMigrationCompression { .. } => "Set migration compression",
            Self::SetHttpsOnly { .. } => "Set HTTPS-only access",
            Self::DesignateCoordinator { .. } => "Hand over pool coordinator",
            Self::SetHaFailuresToTolerate { .. } => "Set HA failure tolerance",
            Self::EnableHa { .. } => "Enable high availability",
            Self::DisableHa => "Disable high availability",
            Self::SetVmRestartPriority { .. } => "Set VM HA restart priority",
            Self::EjectHost { .. } => "Eject pool member",
            Self::EnableTlsVerification => "Enable pool TLS verification",
            Self::RotateSecret => "Rotate pool authentication secret",
        }
    }
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Security(op) => op.method(),
            Self::SetMigrationCompression { .. } => "pool.set_migration_compression",
            Self::SetHttpsOnly { .. } => "Async.pool.set_https_only",
            Self::DesignateCoordinator { .. } => "Async.pool.designate_new_master",
            Self::SetHaFailuresToTolerate { .. } => "Async.pool.set_ha_host_failures_to_tolerate",
            Self::EnableHa { .. } => "Async.pool.enable_ha",
            Self::DisableHa => "Async.pool.disable_ha",
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
    pub heartbeat_srs: Vec<SrId>,
    pub heartbeat_srs_known: bool,
    pub migration_compression: Option<bool>,
    pub https_only: Option<bool>,
    pub tls_verification_enabled: Option<bool>,
    pub secret_rotation_pending: Option<bool>,
}
