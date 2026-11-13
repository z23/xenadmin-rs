use crate::{Error, OperationId, ResourceAction, ResourceOperationRecord, SecretId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, net::IpAddr};
use xenadmin_xapi::records::HostRecord;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostPowerSettings {
    Disabled,
    WakeOnLan,
    Ipmi { address: IpAddr, username: String },
}
impl HostPowerSettings {
    pub fn mode(&self) -> &'static str {
        match self {
            Self::Disabled => "",
            Self::WakeOnLan => "wake-on-lan",
            Self::Ipmi { .. } => "IPMI",
        }
    }
    pub fn needs_password(&self) -> bool {
        matches!(self, Self::Ipmi { .. })
    }
    fn valid(&self) -> bool {
        match self {
            Self::Ipmi { address, username } => {
                !address.is_unspecified()
                    && !address.is_multicast()
                    && !username.is_empty()
                    && username.len() <= 255
                    && !username.chars().any(char::is_control)
            }
            _ => true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPowerConfiguration {
    pub settings: HostPowerSettings,
    pub secret: Option<SecretId>,
}
impl HostPowerConfiguration {
    fn valid(&self) -> bool {
        self.settings.valid()
            && self.settings.needs_password() == self.secret.is_some()
            && self.secret.is_none_or(|id| !id.uuid().is_nil())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostPowerOperation {
    Configure {
        expected: HostPowerConfiguration,
        settings: HostPowerSettings,
    },
    CleanupSecret {
        operation: OperationId,
        secret: SecretId,
    },
}
impl HostPowerOperation {
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        use crate::PermissionRequirements as Required;
        match self {
            Self::CleanupSecret { .. } => Required::writes(["secret.destroy"]),
            Self::Configure { settings, .. } => {
                let mut methods = vec!["host.set_power_on_mode"];
                if settings.needs_password() {
                    methods.push("secret.create");
                }
                Required::writes(methods)
            }
        }
    }

    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Configure { expected, settings } => {
                expected.valid()
                    && settings.valid()
                    && (settings.needs_password() || &expected.settings != settings)
            }
            Self::CleanupSecret { secret, .. } => !secret.uuid().is_nil(),
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Configure { .. } => "Configure host power-on credentials",
            Self::CleanupSecret { .. } => "Remove unused host power-on Secret",
        }
    }
}
pub struct HostPowerPassword(pub(crate) Zeroizing<String>);
impl std::fmt::Debug for HostPowerPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostPowerPassword([redacted])")
    }
}
impl HostPowerPassword {
    pub fn new(password: String) -> Result<Self, Error> {
        let password = Zeroizing::new(password);
        if password.is_empty() || password.len() > 4096 || password.contains('\0') {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(Self(password))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPowerSecretEvidence {
    pub secret: SecretId,
    pub cleaned: bool,
}

pub(crate) fn configuration(record: &Value) -> Option<HostPowerConfiguration> {
    let mode = HostRecord::new(record).power_on_mode().text()?;
    let config = HostRecord::new(record).power_on_config().entries()?;
    let (settings, secret) = match mode {
        "" if config.is_empty() => (HostPowerSettings::Disabled, None),
        "wake-on-lan" if config.is_empty() => (HostPowerSettings::WakeOnLan, None),
        "IPMI" if config.len() == 3 => (
            HostPowerSettings::Ipmi {
                address: config.get("power_on_ip")?.as_str()?.parse().ok()?,
                username: config.get("power_on_user")?.as_str()?.into(),
            },
            Some(SecretId::parse(config.get("power_on_password_secret")?.as_str()?).ok()?),
        ),
        _ => return None,
    };
    let value = HostPowerConfiguration { settings, secret };
    value.valid().then_some(value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub operation: HostPowerOperation,
    pub baseline: Vec<SecretId>,
    pub secret_sent: bool,
    pub secret_completed: bool,
    pub secret: Option<SecretId>,
    pub configure_sent: bool,
    pub configure_completed: bool,
    pub verified: bool,
    pub cleanup_sent: bool,
    pub cleaned: bool,
}
impl Plan {
    pub fn new(operation: HostPowerOperation, baseline: Vec<SecretId>) -> Self {
        Self {
            operation,
            baseline,
            secret_sent: false,
            secret_completed: false,
            secret: None,
            configure_sent: false,
            configure_completed: false,
            verified: false,
            cleanup_sent: false,
            cleaned: false,
        }
    }
    pub fn extends(&self, old: &Self) -> bool {
        self.operation == old.operation
            && self.baseline == old.baseline
            && old.secret.is_none_or(|id| self.secret == Some(id))
            && [
                (old.secret_sent, self.secret_sent),
                (old.secret_completed, self.secret_completed),
                (old.configure_sent, self.configure_sent),
                (old.configure_completed, self.configure_completed),
                (old.verified, self.verified),
                (old.cleanup_sent, self.cleanup_sent),
                (old.cleaned, self.cleaned),
            ]
            .iter()
            .all(|(old, new)| !old || *new)
    }
    fn valid(&self, record: &ResourceOperationRecord) -> bool {
        if record.action != ResourceAction::HostPower(self.operation.clone())
            || !self.operation.valid()
            || self.baseline.len() > 4096
            || self.baseline.iter().any(|id| id.uuid().is_nil())
            || self.baseline.iter().collect::<BTreeSet<_>>().len() != self.baseline.len()
        {
            return false;
        }
        let count = match &self.operation {
            HostPowerOperation::Configure { settings, .. } => {
                1 + usize::from(settings.needs_password())
            }
            HostPowerOperation::CleanupSecret { .. } => 1,
        };
        if record.steps.len() != count
            || !matches!(record.target, crate::ResourceTarget::Host(_))
            || record.state == crate::OperationState::Succeeded && !(self.verified || self.cleaned)
        {
            return false;
        }
        match &self.operation {
            HostPowerOperation::Configure { settings, .. } => {
                !self.cleanup_sent
                    && !self.cleaned
                    && (!self.secret_sent || settings.needs_password())
                    && (!self.secret_completed || self.secret_sent)
                    && self.secret.is_none_or(|id| {
                        self.secret_sent && !id.uuid().is_nil() && !self.baseline.contains(&id)
                    })
                    && (!self.configure_sent
                        || !settings.needs_password()
                        || self.secret_completed && self.secret.is_some())
                    && (!self.configure_completed || self.configure_sent)
                    && (!self.verified || self.configure_completed)
            }
            HostPowerOperation::CleanupSecret { secret, .. } => {
                self.baseline.is_empty()
                    && self.secret == Some(*secret)
                    && !self.secret_sent
                    && !self.secret_completed
                    && !self.configure_sent
                    && !self.configure_completed
                    && !self.verified
                    && (!self.cleaned || self.cleanup_sent)
            }
        }
    }
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    let action = matches!(record.action, ResourceAction::HostPower(_));
    if let Some(plan) = &record.host_power_plan {
        if !action
            || !plan.valid(record)
            || record.host_power_secret
                != plan.secret.map(|secret| HostPowerSecretEvidence {
                    secret,
                    cleaned: plan.cleaned,
                })
        {
            return Err(Error::Journal);
        }
    } else if record.host_power_secret.is_some()
        || action
            && !matches!(
                record.state,
                crate::OperationState::Preparing
                    | crate::OperationState::Rejected
                    | crate::OperationState::Cancelled
                    | crate::OperationState::Failed
            )
    {
        return Err(Error::Journal);
    }
    Ok(())
}
pub(crate) fn project(record: &mut ResourceOperationRecord) {
    if let Some(p) = &record.host_power_plan {
        record.host_power_secret = p.secret.map(|secret| HostPowerSecretEvidence {
            secret,
            cleaned: p.cleaned,
        });
    }
}
