use crate::{Error, PowerAction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionUnknown {
    NotConnected,
    Unsupported,
    DiscoveryDenied,
    InvalidResponse,
    Unavailable,
    KeyDependent,
    WorkflowDependent,
}
impl PermissionUnknown {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::NotConnected => {
                "Session permissions are unavailable until the connection is current"
            }
            Self::Unsupported => {
                "This host does not expose session permissions; the host will check authorization"
            }
            Self::DiscoveryDenied => {
                "The host denied permission discovery; it will check authorization when the action is submitted"
            }
            Self::InvalidResponse => {
                "The host returned incomplete permission information; it will check authorization"
            }
            Self::Unavailable => {
                "Session permission discovery was unavailable; the host will check authorization"
            }
            Self::WorkflowDependent => {
                "Some required permissions depend on fresh host data; the host will check them before each request"
            }
            Self::KeyDependent => {
                "Authorization for this map change depends on protected keys and will be checked by the host"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionDecision {
    Granted,
    Denied,
    Unknown(PermissionUnknown),
}
impl PermissionDecision {
    pub fn is_denied(self) -> bool {
        matches!(self, Self::Denied)
    }
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Granted => {
                "The session grants this API permission; host state and other restrictions still apply"
            }
            Self::Denied => {
                "The current session does not grant permission for this action. Reconnect as an authorized user to change session permissions"
            }
            Self::Unknown(reason) => reason.explanation(),
        }
    }
}

/// Write permissions provable from an immutable reviewed change. Read prerequisites,
/// host state and server-side authorization remain independent checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequirements {
    methods: BTreeSet<String>,
    conditional: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionAssessment {
    decision: PermissionDecision,
    denied_methods: Vec<String>,
}
impl PermissionAssessment {
    pub fn decision(&self) -> PermissionDecision {
        self.decision
    }
    pub fn denied_methods(&self) -> &[String] {
        &self.denied_methods
    }
    pub fn explanation(&self) -> String {
        if self.decision.is_denied() {
            format!(
                "The current session lacks required write permissions: {}. No steps will be submitted. Reconnect as an authorized user to change session permissions.",
                self.denied_methods.join(", ")
            )
        } else if self.decision == PermissionDecision::Granted {
            "The session grants the required write permissions. Fresh object checks, read permissions and host restrictions still apply.".into()
        } else {
            self.decision.explanation().into()
        }
    }
}
impl PermissionRequirements {
    pub(crate) fn writes<I, S>(methods: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            methods: methods.into_iter().map(|m| m.as_ref().to_owned()).collect(),
            conditional: false,
        }
    }
    pub fn with_host_dependent_steps(mut self) -> Self {
        self.conditional = true;
        self
    }
    pub fn methods(&self) -> impl Iterator<Item = &str> {
        self.methods.iter().map(String::as_str)
    }
    pub fn assess(&self, permissions: &SessionPermissions) -> PermissionAssessment {
        let mut unknown = (self.conditional || self.methods.is_empty())
            .then_some(PermissionUnknown::WorkflowDependent);
        let mut denied_methods = Vec::new();
        for method in &self.methods {
            match permissions.method(method) {
                PermissionDecision::Denied => denied_methods.push(method.clone()),
                PermissionDecision::Unknown(reason) => unknown = Some(reason),
                PermissionDecision::Granted => (),
            }
        }
        PermissionAssessment {
            decision: if !denied_methods.is_empty() {
                PermissionDecision::Denied
            } else if let Some(reason) = unknown {
                PermissionDecision::Unknown(reason)
            } else {
                PermissionDecision::Granted
            },
            denied_methods,
        }
    }
    /// Refuses locally, naming the denied methods; the host's token stays in the text so the
    /// reason is recognisable, and the wording says that nothing was sent (lab observation 41).
    pub(crate) fn guard(&self, permissions: &SessionPermissions) -> Result<(), Error> {
        let assessment = self.assess(permissions);
        if assessment.decision.is_denied() {
            Err(Error::RoleDenied(format!(
                "{} (RBAC_PERMISSION_DENIED)",
                assessment.denied_methods.join(", ")
            )))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Authority {
    LocalSuperuser,
    Permissions(BTreeSet<String>),
    Unknown(PermissionUnknown),
}
/// Effective permissions belong to one authenticated session, never a role-name hierarchy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPermissions {
    authority: Authority,
}
impl Default for SessionPermissions {
    fn default() -> Self {
        Self::unknown(PermissionUnknown::NotConnected)
    }
}
impl SessionPermissions {
    pub(crate) fn unknown(reason: PermissionUnknown) -> Self {
        Self {
            authority: Authority::Unknown(reason),
        }
    }
    pub fn is_local_superuser(&self) -> bool {
        matches!(self.authority, Authority::LocalSuperuser)
    }
    pub fn permission_count(&self) -> Option<usize> {
        match &self.authority {
            Authority::Permissions(p) => Some(p.len()),
            _ => None,
        }
    }
    pub fn roles(&self) -> Vec<&str> {
        let Authority::Permissions(permissions) = &self.authority else {
            return vec![];
        };
        [
            "pool-admin",
            "pool-operator",
            "vm-power-admin",
            "vm-admin",
            "vm-operator",
            "read-only",
        ]
        .into_iter()
        .filter(|role| permissions.contains(*role))
        .collect()
    }
    pub fn summary(&self) -> String {
        match &self.authority {
            Authority::LocalSuperuser => "Local superuser session".into(),
            Authority::Permissions(p) => format!("{} effective session permissions", p.len()),
            Authority::Unknown(reason) => reason.explanation().into(),
        }
    }
    pub fn method(&self, method: &str) -> PermissionDecision {
        let Some(method) = normalize(method) else {
            return PermissionDecision::Unknown(PermissionUnknown::InvalidResponse);
        };
        match &self.authority {
            Authority::LocalSuperuser => PermissionDecision::Granted,
            Authority::Unknown(reason) => PermissionDecision::Unknown(*reason),
            Authority::Permissions(permissions) => {
                // XAPI substitutes protected-key permissions before checking map edits. The
                // session's own list cannot tell us keys granted only to other roles.
                let operation = method.split_once('.').map(|(_, op)| op).unwrap_or("");
                if operation.starts_with("add_to_") || operation.starts_with("remove_from_") {
                    return PermissionDecision::Unknown(PermissionUnknown::KeyDependent);
                }
                if permissions.contains(&method) {
                    PermissionDecision::Granted
                } else {
                    PermissionDecision::Denied
                }
            }
        }
    }
    pub fn power(&self, action: PowerAction) -> PermissionDecision {
        self.method(&format!("VM.{}", action.wire()))
    }
    pub(crate) fn guard(&self, method: &str) -> Result<(), Error> {
        if self.method(method).is_denied() {
            Err(Error::RoleDenied(format!(
                "{method} (RBAC_PERMISSION_DENIED)"
            )))
        } else {
            Ok(())
        }
    }
    pub(crate) fn decode(record: &Value) -> Self {
        if record["is_local_superuser"] == true {
            return Self {
                authority: Authority::LocalSuperuser,
            };
        }
        let Some(values) = record["rbac_permissions"]
            .as_array()
            .filter(|v| v.len() <= 16384)
        else {
            return Self::unknown(PermissionUnknown::InvalidResponse);
        };
        if record["is_local_superuser"] != false {
            return Self::unknown(PermissionUnknown::InvalidResponse);
        }
        let mut permissions = BTreeSet::new();
        for value in values {
            let Some(value) = value.as_str().filter(|s| {
                !s.is_empty() && s.len() <= 512 && s.bytes().all(|b| b.is_ascii_graphic())
            }) else {
                return Self::unknown(PermissionUnknown::InvalidResponse);
            };
            // Session values are canonical server identifiers. Never grant an invented
            // lowercased permission from a malformed server response.
            if value.bytes().any(|b| b.is_ascii_uppercase()) {
                return Self::unknown(PermissionUnknown::InvalidResponse);
            }
            permissions.insert(value.to_owned());
        }
        Self {
            authority: Authority::Permissions(permissions),
        }
    }
}
fn normalize(method: &str) -> Option<String> {
    if method.is_empty()
        || method.len() > 256
        || !method
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_')
    {
        return None;
    }
    Some(
        method
            .strip_prefix("Async.")
            .unwrap_or(method)
            .to_ascii_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn effective_permissions_normalize_async_but_never_infer_from_role_names() {
        let p = SessionPermissions::decode(
            &json!({"is_local_superuser":false,"rbac_permissions":["pool-admin","vm.start","vm.clean_shutdown"]}),
        );
        assert_eq!(p.method("Async.VM.start"), PermissionDecision::Granted);
        assert_eq!(p.method("VM.hard_shutdown"), PermissionDecision::Denied);
        assert_eq!(p.roles(), vec!["pool-admin"]);
        assert!(p.guard("Async.VM.hard_shutdown").is_err());
        assert_eq!(
            p.method("VM.add_to_other_config"),
            PermissionDecision::Unknown(PermissionUnknown::KeyDependent)
        );
        assert_eq!(
            p.method("VM.remove_from_other_config"),
            PermissionDecision::Unknown(PermissionUnknown::KeyDependent)
        );
    }
    #[test]
    fn unknown_discovery_is_not_a_denial_and_local_superuser_is_explicit() {
        for record in [
            json!({}),
            json!({"rbac_permissions":[]}),
            json!({"is_local_superuser":false,"rbac_permissions":["VM.start"]}),
        ] {
            let p = SessionPermissions::decode(&record);
            assert!(matches!(
                p.power(PowerAction::Start),
                PermissionDecision::Unknown(_)
            ));
            assert!(p.guard("VM.start").is_ok());
        }
        let denied =
            SessionPermissions::decode(&json!({"is_local_superuser":false,"rbac_permissions":[]}));
        assert_eq!(denied.power(PowerAction::Start), PermissionDecision::Denied);
        let root = SessionPermissions::decode(&json!({"is_local_superuser":true}));
        assert_eq!(root.power(PowerAction::Start), PermissionDecision::Granted);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::{DeviceOperation, DeviceProperty, DeviceQos, HostOperation};
    use serde_json::json;
    fn authority(methods: &[&str]) -> SessionPermissions {
        SessionPermissions::decode(&json!({"is_local_superuser":false,"rbac_permissions":methods}))
    }
    #[test]
    fn known_denial_wins_over_unresolved_key_or_workflow_checks() {
        let requirements =
            PermissionRequirements::writes(["VM.add_to_other_config", "VDI.destroy"])
                .with_host_dependent_steps();
        let assessment = requirements.assess(&authority(&[]));
        assert_eq!(assessment.decision(), PermissionDecision::Denied);
        assert_eq!(assessment.denied_methods(), &["VDI.destroy"]);
        assert!(
            assessment
                .explanation()
                .contains("No steps will be submitted")
        );
        assert!(matches!(
            requirements.assess(&authority(&["vdi.destroy"])).decision(),
            PermissionDecision::Unknown(_)
        ));
        assert!(matches!(
            requirements
                .assess(&SessionPermissions::default())
                .decision(),
            PermissionDecision::Unknown(_)
        ));
    }
    #[test]
    fn nic_move_does_not_require_replacement_and_replacement_checks_create_before_destroy() {
        let id = uuid::Uuid::from_u128(1);
        let mut operation = DeviceOperation::EditNic {
            vif: id.into(),
            expected_network: id.into(),
            expected_mac: "02:00:00:00:00:aa".into(),
            network: uuid::Uuid::from_u128(2).into(),
            mac: "02:00:00:00:00:AA".into(),
        };
        assert_eq!(
            operation
                .permission_requirements()
                .assess(&authority(&["vif.move"]))
                .decision(),
            PermissionDecision::Granted
        );
        if let DeviceOperation::EditNic { mac, .. } = &mut operation {
            *mac = "02:00:00:00:00:bb".into();
        }
        let denied = operation
            .permission_requirements()
            .assess(&authority(&["vif.destroy"]));
        assert_eq!(denied.denied_methods(), &["VIF.create"]);
    }
    #[test]
    fn unchanged_metadata_and_qos_fields_never_require_their_setter() {
        let id = uuid::Uuid::from_u128(1);
        let op = DeviceOperation::Property {
            change: DeviceProperty::DiskMetadata {
                vbd: id.into(),
                vdi: id.into(),
                previous_name: "old".into(),
                name: "new".into(),
                previous_description: "retained".into(),
                description: "retained".into(),
            },
        };
        assert_eq!(
            op.permission_requirements()
                .assess(&authority(&["vdi.set_name_label"]))
                .decision(),
            PermissionDecision::Granted
        );
        let op = DeviceOperation::Property {
            change: DeviceProperty::NicRate {
                vif: id.into(),
                previous: DeviceQos {
                    algorithm: "ratelimit".into(),
                    parameters: std::collections::BTreeMap::from([("kbps".into(), "1024".into())]),
                },
                kilobytes_per_second: None,
            },
        };
        assert_eq!(
            op.permission_requirements()
                .assess(&authority(&["vif.set_qos_algorithm_type"]))
                .decision(),
            PermissionDecision::Granted
        );
    }
    #[test]
    fn maintenance_checks_only_steps_captured_in_review_and_settings_share_dispatch_plan() {
        let op = HostOperation::EnterMaintenance {
            expected_enabled: false,
            resident_vms: vec![],
        };
        assert_eq!(
            op.permission_requirements()
                .assess(&authority(&["host.set_other_config"]))
                .decision(),
            PermissionDecision::Granted
        );
        let op = HostOperation::EnterMaintenance {
            expected_enabled: true,
            resident_vms: vec![uuid::Uuid::from_u128(1).into()],
        };
        assert_eq!(
            op.permission_requirements()
                .assess(&authority(&["host.set_other_config"]))
                .denied_methods(),
            &["Async.host.disable", "Async.host.evacuate"]
        );
        let patch = crate::VmSettingsPatch::new(vec![
            crate::VmSettingEdit::Name {
                previous: "old".into(),
                value: "new".into(),
            },
            crate::VmSettingEdit::Description {
                previous: "old".into(),
                value: "new".into(),
            },
        ])
        .unwrap();
        assert_eq!(
            patch
                .permission_requirements()
                .assess(&authority(&["vm.set_name_label"]))
                .denied_methods(),
            &["VM.set_name_description"]
        );
    }
}
