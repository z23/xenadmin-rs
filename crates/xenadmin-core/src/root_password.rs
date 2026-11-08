use crate::{
    Error, HostId, OperationState, ResourceAction, ResourceOperationRecord, ResourceTarget,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolRootPasswordChange {
    pub coordinator: HostId,
    pub members: Vec<HostId>,
}
impl PoolRootPasswordChange {
    pub(crate) fn valid(&self) -> bool {
        !self.coordinator.uuid().is_nil()
            && !self.members.is_empty()
            && self.members.len() <= 1024
            && self.members.contains(&self.coordinator)
            && self.members.iter().all(|id| !id.uuid().is_nil())
            && self.members.iter().collect::<BTreeSet<_>>().len() == self.members.len()
    }
}

pub struct RootPassword(pub(crate) Zeroizing<String>);
impl RootPassword {
    pub fn new(password: String) -> Result<Self, Error> {
        let password = Zeroizing::new(password);
        if password.is_empty() || password.len() > 4096 || password.contains(['\0', '\n', '\r']) {
            return Err(Error::InvalidResourceCommand);
        }
        Ok(Self(password))
    }
}
impl std::fmt::Debug for RootPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RootPassword([redacted])")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootPasswordEvidence {
    pub coordinator_changed: bool,
    pub coordinator_login_verified: bool,
    pub reconnect_credentials_updated: bool,
    /// The API requests synchronization but suppresses individual member failures.
    pub unverified_members: Vec<HostId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub expected: PoolRootPasswordChange,
    pub sent: bool,
    pub completed: bool,
    pub login_verified: bool,
    pub credentials_updated: bool,
}
impl Plan {
    pub fn extends(&self, old: &Self) -> bool {
        self.expected == old.expected
            && (!old.sent || self.sent)
            && (!old.completed || self.completed)
            && (!old.login_verified || self.login_verified)
            && (!old.credentials_updated || self.credentials_updated)
    }
    fn evidence(&self) -> RootPasswordEvidence {
        RootPasswordEvidence {
            coordinator_changed: self.completed,
            coordinator_login_verified: self.login_verified,
            reconnect_credentials_updated: self.credentials_updated,
            unverified_members: self
                .expected
                .members
                .iter()
                .copied()
                .filter(|id| *id != self.expected.coordinator)
                .collect(),
        }
    }
}
pub(crate) fn project(record: &mut ResourceOperationRecord) {
    record.root_password = record.root_password_plan.as_ref().map(Plan::evidence);
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    let action = matches!(record.action, ResourceAction::RootPassword(_));
    if let ResourceAction::RootPassword(change) = &record.action
        && (record.target != ResourceTarget::Pool(record.pool) || !change.valid())
    {
        return Err(Error::Journal);
    }
    if let Some(p) = &record.root_password_plan {
        if record.action != ResourceAction::RootPassword(p.expected.clone())
            || record.target != ResourceTarget::Pool(record.pool)
            || !p.expected.valid()
            || record.steps.len() != 3
            || p.completed && !p.sent
            || p.login_verified && !p.completed
            || p.credentials_updated && !p.login_verified
            || record.root_password.as_ref() != Some(&p.evidence())
            || record.state == OperationState::Succeeded
                && (!p.credentials_updated || p.expected.members.len() != 1)
        {
            return Err(Error::Journal);
        }
    } else if record.root_password.is_some()
        || action
            && !matches!(
                record.state,
                OperationState::Preparing
                    | OperationState::Rejected
                    | OperationState::Cancelled
                    | OperationState::Failed
            )
    {
        return Err(Error::Journal);
    }
    Ok(())
}
