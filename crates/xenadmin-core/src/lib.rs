mod connection;
mod console;
mod model;
mod operations;
mod trust;
mod xapi;

pub use connection::{
    Backend, Connection, ConnectionSnapshot, ConnectionState, ObservedSnapshot, ObservedVm,
};
pub use console::{Console, ConsoleFrame, ConsoleInput, ConsoleSnapshot, ConsoleState};
pub use model::{
    Host, HostId, Inventory, Network, PoolId, PowerState, Snapshot, SnapshotAction, SnapshotId,
    SnapshotInventory, Storage, Vm, VmId,
};
pub use operations::{
    Operation, OperationAction, OperationControl, OperationId, OperationRecord, OperationState,
    PowerAction, SnapshotName, SnapshotOperation, VmCommand,
};
pub use trust::{CertificateFingerprint, ConnectionOptions, Credentials, Endpoint};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Enter a snapshot name of 1–255 characters without control characters")]
    InvalidSnapshotName,
    #[error("Shut down the VM before restoring a snapshot")]
    SnapshotRequiresHaltedVm,
    #[error("Enter an HTTPS host URL without credentials, a query, or a fragment")]
    InvalidEndpoint,
    #[error("The certificate fingerprint must contain 64 hexadecimal digits")]
    InvalidFingerprint,
    #[error("Enter additional host certificates as one https://host=SHA256 entry per line")]
    InvalidHostCertificate,
    #[error(
        "TLS verification failed. Supply a trusted CA certificate or verify and pin the host certificate fingerprint"
    )]
    Certificate,
    #[error(
        "TLS verification failed for {0}. Verify this host's certificate and add its endpoint-specific fingerprint"
    )]
    EndpointCertificate(Endpoint),
    #[error("The host did not respond within the request timeout")]
    Timeout,
    #[error("Cannot reach the XCP-ng management endpoint")]
    Transport,
    #[error("The host returned HTTP status {0}")]
    Http(u16),
    #[error("XAPI refused the request: {0}")]
    Api(String),
    #[error(
        "This pool member redirects to {0}. Connect to that endpoint after verifying its certificate"
    )]
    Coordinator(Endpoint),
    #[error("Invalid XAPI response: {0}")]
    Protocol(&'static str),
    #[error("Invalid XAPI response: invalid inventory number in {field} (expected {expected})")]
    InventoryNumber {
        field: &'static str,
        expected: &'static str,
    },
    #[error("The configured certificate file cannot be read or decoded")]
    CertificateFile,
    #[error("The background runtime could not start")]
    Runtime,
    #[error("{0}")]
    Connection(String),
    #[error("The connection was closed")]
    Closed,
    #[error("The connection is not ready or the selected object is no longer current")]
    Stale,
    #[error("The input or request queue is full")]
    Busy,
    #[error("No RFB console is available for this VM")]
    NoConsole,
    #[error("The advertised console endpoint is not a verified host in this pool")]
    ConsoleEndpoint,
    #[error("The console protocol failed or the connection closed")]
    ConsoleProtocol,
    #[error(
        "Cannot read or save the operation journal. Further mutations are blocked until it is available"
    )]
    Journal,
    #[error("The operation record changed while this worker was running")]
    JournalChanged,
    #[error(
        "Another operation on this VM is still running or unresolved. Inspect operation history before submitting another action"
    )]
    ConflictingOperation,
    #[error("The operation journal has reached its unresolved-operation limit")]
    JournalFull,
    #[error("The requested action is unavailable for this VM")]
    UnavailableAction,
    #[error("This connection is read-only")]
    ReadOnly,
    #[error("The operation ID was already used for a different request")]
    OperationConflict,
}

impl Error {
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Transport | Self::Http(502..=504) | Self::Coordinator(_)
        ) || matches!(self, Self::Api(code) if code == "SESSION_INVALID" || code == "EVENT_FROM_TOKEN_PARSE_FAILURE")
    }
}
