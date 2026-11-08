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
            Err(Error::Api("RBAC_PERMISSION_DENIED".into()))
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
