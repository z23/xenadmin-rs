use crate::{
    Connection, Credentials, Error, HostId, ObservedResource, PoolId, PoolOperation,
    ResourceAction, ResourceTarget,
};

/// Captured identities from two independently connected pools.
#[derive(Clone, Debug)]
pub struct PoolJoinSelection {
    pub source: ObservedResource,
    pub source_host: HostId,
    pub destination: ObservedResource,
    pub destination_coordinator: HostId,
    pub destination_address: String,
}
impl PoolJoinSelection {
    pub(crate) fn action(&self) -> Result<ResourceAction, Error> {
        let ResourceTarget::Pool(destination_pool) = self.destination.target() else {
            return Err(Error::InvalidResourceCommand);
        };
        let action = ResourceAction::Pool(PoolOperation::Join {
            source_host: self.source_host,
            destination_pool,
            destination_coordinator: self.destination_coordinator,
            destination_address: self.destination_address.clone(),
        });
        action.validate(self.source.target())?;
        Ok(action)
    }
}
pub(crate) struct Inputs {
    pub destination: crate::connection::JoinPeerHandle,
    pub observation: ObservedResource,
    pub credentials: Credentials,
}
pub(crate) fn valid_address(value: &str) -> bool {
    if value.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !value.is_empty()
        && value.len() <= 253
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|v| v.is_ascii_alphanumeric() || v == b'-')
        })
}
pub(crate) fn credentials_valid(value: &Credentials) -> bool {
    !value.username.is_empty()
        && value.username.len() <= 1024
        && !value.username.chars().any(char::is_control)
        && !value.password.is_empty()
        && value.password.len() <= 65536
        && !value.password.contains('\0')
}
pub(crate) fn destination(action: &ResourceAction) -> Option<PoolId> {
    match action {
        ResourceAction::Pool(PoolOperation::Join {
            destination_pool, ..
        }) => Some(*destination_pool),
        _ => None,
    }
}
impl Connection {
    pub fn join_pool(
        &self,
        id: crate::OperationId,
        selection: PoolJoinSelection,
        destination: &Connection,
        credentials: Credentials,
    ) -> Result<crate::ResourceOperation, Error> {
        self.submit_pool_join(id, selection, destination, credentials)
    }
}
