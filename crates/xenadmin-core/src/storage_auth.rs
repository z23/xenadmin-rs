//! Transient storage credentials and durable, value-free provisioning evidence.
use crate::{Error, SecretId, StorageLocation, StorageOperation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageAuthentication {
    Chap,
    MutualChap,
    Smb,
}
impl StorageAuthentication {
    pub(crate) fn kinds(self) -> &'static [SecretKind] {
        match self {
            Self::Chap => &[SecretKind::Chap],
            Self::MutualChap => &[SecretKind::Chap, SecretKind::IncomingChap],
            Self::Smb => &[SecretKind::Smb],
        }
    }
    pub(crate) fn valid_for(self, operation: &StorageOperation) -> bool {
        let location = match operation {
            StorageOperation::Create { location, .. }
            | StorageOperation::CreateAttachment { location, .. } => location,
            _ => return false,
        };
        matches!(
            (self, location),
            (Self::Chap | Self::MutualChap, StorageLocation::Iscsi { .. })
                | (Self::Smb, StorageLocation::IsoSmb { .. })
        )
    }
}
pub struct StorageCredentials {
    pub(crate) authentication: StorageAuthentication,
    pub(crate) entries: Vec<Credential>,
}
pub(crate) struct Credential {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
}
impl std::fmt::Debug for StorageCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StorageCredentials([redacted])")
    }
}
impl StorageCredentials {
    pub fn chap(username: String, password: String) -> Self {
        Self::new(StorageAuthentication::Chap, username, password)
    }
    pub fn smb(username: String, password: String) -> Self {
        Self::new(StorageAuthentication::Smb, username, password)
    }
    pub fn mutual_chap(
        username: String,
        password: String,
        incoming_username: String,
        incoming_password: String,
    ) -> Self {
        let mut value = Self::new(StorageAuthentication::MutualChap, username, password);
        value.entries.push(Credential {
            username: Zeroizing::new(incoming_username),
            password: Zeroizing::new(incoming_password),
        });
        value
    }
    fn new(authentication: StorageAuthentication, username: String, password: String) -> Self {
        Self {
            authentication,
            entries: vec![Credential {
                username: Zeroizing::new(username),
                password: Zeroizing::new(password),
            }],
        }
    }
    pub(crate) fn validate(&self, operation: &StorageOperation) -> Result<(), Error> {
        if !self.authentication.valid_for(operation) {
            return Err(Error::InvalidResourceCommand);
        }
        self.validate_values()
    }
    pub(crate) fn validate_values(&self) -> Result<(), Error> {
        if self.entries.len() != self.authentication.kinds().len()
            || self.entries.iter().any(|c| {
                c.username.is_empty()
                    || c.username.len() > 1024
                    || c.username.chars().any(char::is_control)
                    || c.password.is_empty()
                    || c.password.len() > 4096
                    || c.password.contains('\0')
            })
        {
            return Err(Error::InvalidResourceCommand);
        }
        if self.authentication == StorageAuthentication::Smb
            && self.entries[0].username.split('\\').count() > 2
        {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SecretKind {
    Chap,
    IncomingChap,
    Smb,
}
impl SecretKind {
    pub(crate) fn password_key(self) -> &'static str {
        match self {
            Self::Chap => "chappassword_secret",
            Self::IncomingChap => "incoming_chappassword_secret",
            Self::Smb => "cifspassword_secret",
        }
    }
    pub(crate) fn username_key(self) -> &'static str {
        match self {
            Self::Chap => "chapuser",
            Self::IncomingChap => "incoming_chapuser",
            Self::Smb => "username",
        }
    }
}
pub(crate) fn username_digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSecretEvidence {
    pub secret: SecretId,
    pub cleaned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SecretStep {
    pub kind: SecretKind,
    pub username_digest: [u8; 32],
    pub sent: bool,
    pub completed: bool,
    pub id: Option<SecretId>,
    pub cleanup_sent: bool,
    pub cleaned: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub operation: StorageOperation,
    pub authentication: StorageAuthentication,
    pub baseline: Vec<SecretId>,
    pub task_baseline: Vec<crate::TaskId>,
    pub task_uuid: Option<crate::TaskId>,
    pub secrets: Vec<SecretStep>,
    pub provision_sent: bool,
    pub provision_succeeded: bool,
    pub verified: bool,
}
impl Plan {
    pub(crate) fn valid(&self, record: &crate::ResourceOperationRecord) -> bool {
        let crate::ResourceAction::AuthenticatedStorage {
            operation,
            authentication,
        } = &record.action
        else {
            return false;
        };
        if &self.operation != operation
            || &self.authentication != authentication
            || !authentication.valid_for(operation)
            || self.baseline.len() > 4096
            || self
                .baseline
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.baseline.len()
            || self.baseline.iter().any(|id| id.uuid().is_nil())
            || self.task_baseline.iter().any(|id| id.uuid().is_nil())
            || self.task_baseline.len() > 10000
            || self
                .task_baseline
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.task_baseline.len()
            || self.task_uuid.is_some_and(|id| {
                id.uuid().is_nil()
                    || !self.provision_sent
                    || record.task.is_none()
                    || self.task_baseline.contains(&id)
            })
            || self.secrets.len() != authentication.kinds().len()
            || self.provision_succeeded
                && (!self.provision_sent
                    || self.task_uuid.is_none()
                    || record.task.is_none()
                    || record.result_reference.is_none())
            || self.verified && !self.provision_succeeded
            || self.provision_sent && self.secrets.iter().any(|s| !s.completed || s.id.is_none())
        {
            return false;
        }
        let mut previous = true;
        let mut ids = std::collections::BTreeSet::new();
        for (step, kind) in self.secrets.iter().zip(authentication.kinds()) {
            if step.kind != *kind
                || step.sent && !previous
                || step.completed && !step.sent
                || step.id.is_some() && !step.sent
                || step.id.is_some_and(|id| {
                    id.uuid().is_nil() || self.baseline.contains(&id) || !ids.insert(id)
                })
                || step.cleanup_sent && (!self.verified || step.id.is_none())
                || step.cleaned && !step.cleanup_sent
            {
                return false;
            }
            previous = step.completed && step.id.is_some();
        }
        record.steps.len() == self.secrets.len() * 2 + 1
            && record.storage_secrets
                == self
                    .secrets
                    .iter()
                    .filter_map(|s| {
                        s.id.map(|secret| StorageSecretEvidence {
                            secret,
                            cleaned: s.cleaned,
                        })
                    })
                    .collect::<Vec<_>>()
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.operation == old.operation
            && self.authentication == old.authentication
            && self.baseline == old.baseline
            && self.task_baseline == old.task_baseline
            && old.task_uuid.is_none_or(|id| self.task_uuid == Some(id))
            && self.secrets.len() == old.secrets.len()
            && (!old.provision_sent || self.provision_sent)
            && (!old.provision_succeeded || self.provision_succeeded)
            && (!old.verified || self.verified)
            && self.secrets.iter().zip(&old.secrets).all(|(n, o)| {
                n.kind == o.kind
                    && n.username_digest == o.username_digest
                    && (!o.sent || n.sent)
                    && (!o.completed || n.completed)
                    && (o.id.is_none() || n.id == o.id)
                    && (!o.cleanup_sent || n.cleanup_sent)
                    && (!o.cleaned || n.cleaned)
            })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanupStep {
    pub id: SecretId,
    pub sent: bool,
    pub completed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanupPlan {
    pub source: crate::OperationId,
    pub secrets: Vec<CleanupStep>,
}
impl CleanupPlan {
    pub(crate) fn valid(&self, record: &crate::ResourceOperationRecord) -> bool {
        let crate::ResourceAction::CleanupStorageCredentials { operation, secrets } =
            &record.action
        else {
            return false;
        };
        &self.source == operation
            && self.secrets.iter().map(|s| s.id).collect::<Vec<_>>() == *secrets
            && self.secrets.iter().all(|s| !s.completed || s.sent)
            && record.steps.len() == self.secrets.len()
            && record.storage_secrets
                == self
                    .secrets
                    .iter()
                    .map(|s| StorageSecretEvidence {
                        secret: s.id,
                        cleaned: s.completed,
                    })
                    .collect::<Vec<_>>()
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.source == old.source
            && self.secrets.len() == old.secrets.len()
            && self
                .secrets
                .iter()
                .zip(&old.secrets)
                .all(|(n, o)| n.id == o.id && (!o.sent || n.sent) && (!o.completed || n.completed))
    }
}
pub(crate) fn validate_record(record: &crate::ResourceOperationRecord) -> Result<(), Error> {
    let empty = record.state == crate::OperationState::Preparing && record.steps.is_empty();
    let valid = match (
        &record.action,
        &record.storage_auth_plan,
        &record.storage_secret_cleanup,
    ) {
        (crate::ResourceAction::AuthenticatedStorage { .. }, Some(plan), None) => {
            plan.valid(record)
        }
        (crate::ResourceAction::CleanupStorageCredentials { .. }, None, Some(plan)) => {
            plan.valid(record)
        }
        (
            crate::ResourceAction::AuthenticatedStorage { .. }
            | crate::ResourceAction::CleanupStorageCredentials { .. },
            None,
            None,
        ) => empty || record.state.terminal(),
        (_, None, None) => true,
        _ => false,
    };
    if valid { Ok(()) } else { Err(Error::Journal) }
}

pub(crate) fn project(record: &mut crate::ResourceOperationRecord) {
    if let Some(plan) = &record.storage_auth_plan {
        record.storage_secrets = plan
            .secrets
            .iter()
            .filter_map(|s| {
                s.id.map(|secret| StorageSecretEvidence {
                    secret,
                    cleaned: s.cleaned,
                })
            })
            .collect();
        let count = plan.secrets.len();
        if record.steps.len() == count * 2 + 1 {
            for (i, s) in plan.secrets.iter().enumerate() {
                if s.completed && s.id.is_some() {
                    record.steps[i].state = crate::StepState::Verified;
                }
                if s.cleaned {
                    record.steps[count + 1 + i].state = crate::StepState::Verified;
                }
            }
            if plan.verified {
                record.steps[count].state = crate::StepState::Verified;
            }
        }
    }
    if let Some(plan) = &record.storage_secret_cleanup {
        record.storage_secrets = plan
            .secrets
            .iter()
            .map(|s| StorageSecretEvidence {
                secret: s.id,
                cleaned: s.completed,
            })
            .collect();
        for (step, s) in record.steps.iter_mut().zip(&plan.secrets) {
            if s.completed {
                step.state = crate::StepState::Verified;
            }
        }
    }
}

impl StorageOperation {
    pub fn authenticated_permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(
            self.permission_requirements()
                .methods()
                .map(str::to_owned)
                .chain(["secret.create".into(), "secret.destroy".into()]),
        )
    }
}
