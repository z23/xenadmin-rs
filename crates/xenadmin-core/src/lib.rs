mod connection;
mod device_lifecycle;
mod devices;
pub use device_lifecycle::CreatedDevice;
pub use devices::{DeviceCommand, DeviceOperation};
mod credentials;
pub use credentials::{
    CredentialKey, CredentialMode, CredentialOutcome, CredentialStore, CredentialStoreError,
    CredentialTask,
};
mod diagnostics;
pub use diagnostics::ActionDiagnostic;
mod inspection;
pub use inspection::{DevInspection, InspectionClass, InspectionRequest, inspect_live};
mod console;
mod metrics;
mod model;
pub use metrics::{
    MetricCategory, MetricSample, MetricSeries, MetricSource, MetricTarget, Metrics,
    MetricsOptions, MetricsSnapshot, MetricsState, MetricsWindow, ObservedMetrics,
};
mod network_types;
mod operations;
mod profiles;
pub use network_types::{
    BondHash, BondMode, Ipv4Configuration, Ipv4Mode, NetworkOperation, PifIdentity,
};
mod pool_admin;
pub use pool_admin::{HaRestartPriority, PoolAdministration, PoolOperation};
mod resources;
mod storage;
mod storage_discovery;
pub use resources::{
    ObservedResource, ResourceAction, ResourceCommand, ResourceOperation, ResourceOperationRecord,
    ResourceTarget,
};
mod migration;
mod migration_backend;
pub use storage::{NfsVersion, StorageLocation, StorageOperation};
pub use storage_discovery::{
    DiscoveredRepository, StorageBackend, StorageDiscovery, StorageDiscoveryResult, StorageQuery,
};
mod host_types;
mod settings;
pub use host_types::{HostOperation, HostPowerOnMode};
mod snapshots;
pub use migration::{DiskPlacement, NicPlacement, VmMigration};
mod vm_lifecycle;
mod vm_lifecycle_backend;
pub use vm_lifecycle::{
    GuestFirmware, SnapshotDeletionSelection, VmCopyMode, VmLifecycleOperation,
};
mod transfer;
pub use transfer::{
    ArchiveGuest, ArchiveSummary, ExportSource, TransferAction, TransferCreated, TransferFile,
    TransferProgress, TransferRequest,
};
mod trust;
mod xapi;

pub use connection::{
    Backend, Connection, ConnectionSnapshot, ConnectionState, ObservedSnapshot, ObservedVm,
};
pub use console::{
    Console, ConsoleCursor, ConsoleFrame, ConsoleInput, ConsoleSnapshot, ConsoleState,
};
pub use model::*;
pub use operations::{
    Operation, OperationAction, OperationControl, OperationId, OperationRecord, OperationState,
    PlacementOperation, PowerAction, SnapshotName, SnapshotOperation, VmCommand,
};
pub use profiles::{ConnectionProfile, ProfileError, ProfileId, ProfileStore};
pub use settings::{MemoryLimits, OperationStep, StepState, VmSettingEdit, VmSettingsPatch};
pub use trust::{CertificateFingerprint, ConnectionOptions, Credentials, Endpoint};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Choose a regular local file and an existing destination directory")]
    TransferFile,
    #[error("The selected local file or directory changed since review")]
    TransferFileChanged,
    #[error("The destination already exists; choose a new filename")]
    TransferDestinationExists,
    #[error("The transfer exceeds the supported size or time limit")]
    TransferLimit,
    #[error(
        "This archive is malformed or uses a format/configuration not supported by this importer"
    )]
    TransferArchive,
    #[error(
        "The transfer redirect is outside the authenticated pool or changes the requested operation"
    )]
    TransferEndpoint,
    #[error(
        "The transfer session expired; reconnect and inspect its outcome before starting another transfer"
    )]
    TransferExpired,
    #[error("The transfer resource identities or provenance could not be verified")]
    TransferConflict,

    #[error("Choose up to 32 distinct metric source names")]
    InvalidMetricsOptions,
    #[error("The metrics endpoint or redirect is outside the verified pool")]
    MetricsEndpoint,
    #[error("Choose a valid migration destination and complete resource mappings")]
    InvalidMigration,
    #[error("The VM, host, storage or network changed; refresh and review the migration again")]
    MigrationConflict,
    #[error("This migration topology or resource configuration is not supported")]
    UnsupportedMigration,
    #[error("Choose valid resource settings and a change to apply")]
    InvalidResourceCommand,
    #[error("{0}")]
    HostPrerequisite(&'static str),
    #[error("The resource changed since review. Refresh it before retrying")]
    ResourceConflict,
    #[error(
        "This network has physical or virtual interfaces attached; detach them explicitly before this operation"
    )]
    ResourceInUse,
    #[error("Another VM or infrastructure operation in this pool is running or unresolved")]
    ConflictingPoolOperation,
    #[error(
        "Storage is attached, in use, or required by pool or host configuration; resolve these dependencies first"
    )]
    StorageInUse,
    #[error("Enter valid VM lifecycle configuration and exact resource selections")]
    InvalidVmLifecycle,
    #[error(
        "No compatible host was found for the new VM configuration; the VM remains retained and halted"
    )]
    NoCompatibleVmHost,
    #[error("The VM, template, or its devices changed; refresh and review again")]
    VmLifecycleConflict,
    #[error("This template or configuration is not supported by this creation workflow")]
    UnsupportedVmLifecycle,
    #[error("Choose a valid device operation and a larger disk size")]
    InvalidDeviceOperation,
    #[error("The device changed since this view opened. Refresh before trying again")]
    DeviceConflict,
    #[error("Shut down the VM before growing its disk")]
    DeviceRequiresHaltedVm,
    #[error("This disk is shared, attached elsewhere, or unsuitable for this operation")]
    DeviceInUse,
    #[error("Shut down the VM before changing its device configuration")]
    DeviceLifecycleRequiresHaltedVm,
    #[error("Enter a valid device slot, disk size/name, or unicast MAC address")]
    InvalidDeviceConfiguration,
    #[error(
        "This NIC has additional configuration that this editor cannot safely preserve during replacement"
    )]
    UnsupportedNicConfiguration,
    #[error("This clipboard text encoding is not supported by the console")]
    ConsoleClipboardUnsupported,
    #[error("Clipboard text exceeds the console size limit")]
    ConsoleClipboardTooLarge,
    #[error("Enter valid VM property values; CPU and memory limits must be ordered and positive")]
    InvalidSettings,
    #[error("Shut down the VM before changing CPU, boot or memory limits")]
    SettingsRequiresHaltedVm,
    #[error("VM properties changed since this editor opened. Reload them before saving")]
    SettingsConflict,
    #[error("Enter a snapshot name of 1–255 characters without control characters")]
    InvalidSnapshotName,
    #[error("Shut down the VM before restoring a snapshot")]
    SnapshotRequiresHaltedVm,
    #[error(
        "The snapshot or its reviewed disk ownership changed; reload the snapshot before continuing"
    )]
    SnapshotConflict,
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
    #[error(
        "The operation journal was written by a newer application. Use that version before submitting more changes"
    )]
    JournalVersion,
    #[error("The operation record changed while this worker was running")]
    JournalChanged,
    #[error(
        "Another operation on this VM is still running or unresolved. Inspect operation history before submitting another action"
    )]
    ConflictingOperation,
    #[error(
        "VM creation and copying require no other running or unresolved changes in this pool. Inspect operation history before continuing"
    )]
    PoolCreationConflict,
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
