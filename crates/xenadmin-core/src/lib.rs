mod power_batch;
pub use power_batch::{BatchVmPower, VmPowerBatch};
mod host_power;
pub use host_power::{
    HostPowerConfiguration, HostPowerOperation, HostPowerPassword, HostPowerSecretEvidence,
    HostPowerSettings,
};
mod storage_auth;
pub use storage_auth::{StorageAuthentication, StorageCredentials, StorageSecretEvidence};
mod device_properties;
mod search;
pub use device_properties::{DeviceProperty, DeviceQos, DiskAccess};
pub use search::{
    InventorySearch, SavedSearch, SearchCatalog, SearchGroup, SearchKind, SearchResult,
};
mod appliances;
pub use appliances::{ApplianceMember, ApplianceOperation, ApplianceOrder, AppliancePower};
mod certificate_inspection;
mod cloud_init;
mod connection;
mod connection_proxy;
#[cfg(test)]
mod connection_trust_tests;
pub use certificate_inspection::{
    CertificateDetails, CertificateInspectionTask, CertificateObservation, CertificateValidation,
};
pub use cloud_init::{CloudInitConfiguration, CloudInitInputs, CloudInitMedia};
pub use connection_proxy::{ProxyConfiguration, ProxyCredentials, ProxyEndpoint};
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
    BondHash, BondMode, Ipv4Configuration, Ipv4Mode, Ipv6Configuration, Ipv6Mode, NetworkOperation,
    PifIdentity,
};
mod security_discovery;
pub use security_discovery::{
    DirectorySubject, PoolSecurityDiscovery, PoolSecurityQuery, PoolSecurityResult,
    SecurityOverview, SecurityRole,
};
mod pool_security;
pub use pool_security::{
    DirectoryHostState, PoolSecurityOperation, PoolSensitiveChange, ServerCertificate,
};
mod pool_admin;
pub use pool_admin::{HaRestartPriority, PoolAdministration, PoolOperation};
mod organization;
pub use organization::{
    CustomFieldDefinition, CustomFieldType, OrganizationChange, OrganizationEdit,
    OrganizationInventory, OrganizationObject, OrganizationObjectId, OrganizationPatch,
    valid_folder,
};
mod resources;
mod storage;
mod storage_discovery;
pub use resources::{
    ObservedResource, ResourceAction, ResourceCommand, ResourceOperation, ResourceOperationRecord,
    ResourceTarget,
};
mod cross_pool;
mod cross_pool_backend;
pub use cross_pool::{CrossPoolMode, CrossPoolVmOperation, CrossPoolVmSelection};
mod migration;
mod migration_backend;
pub use storage::{NfsVersion, StorageLocation, StorageOperation};
pub use storage_discovery::{
    DiscoveredIscsiLun, DiscoveredIscsiTarget, DiscoveredNfsExport, DiscoveredRepository,
    StorageBackend, StorageDiscovery, StorageDiscoveryResult, StorageQuery,
};
mod host_types;
mod settings;
pub use host_types::{HostOperation, HostPowerOnMode};
mod guest_boot;
mod snapshot_policy;
mod snapshot_policy_support;
pub use guest_boot::{BootReadiness, GuestBootStatus, GuestBootStatusQuery};
mod snapshot_workflow;
mod snapshots;
pub use migration::{DiskPlacement, NicPlacement, VmMigration};
pub use snapshot_policy::{
    ScheduledSnapshotKind, SnapshotPolicyClock, SnapshotPolicyConfig, SnapshotPolicyOperation,
    SnapshotPolicyState, SnapshotSchedule,
};
pub use snapshot_policy_support::{SnapshotPolicySupport, SnapshotPolicySupportQuery};
pub use snapshot_workflow::SnapshotWorkflow;
mod vm_lifecycle;
mod vm_lifecycle_backend;
pub use vm_lifecycle::{
    GuestFirmware, SnapshotDeletionSelection, VmCopyMode, VmLifecycleOperation,
};
mod disk_image;
mod ovf;
pub use ovf::{OvfAttachment, OvfDisk, OvfGuest, OvfNic, OvfPackageFormat, OvfSummary};
mod transfer;
pub use disk_image::{DiskImageFormat, DiskImageSummary};
pub use transfer::{
    ArchiveGuest, ArchiveSummary, ExportSource, TransferAction, TransferCreated, TransferFile,
    TransferProgress, TransferRequest,
};
mod trust;
mod xapi;

pub use connection::{
    Backend, Connection, ConnectionFailure, ConnectionSnapshot, ConnectionStage, ConnectionState,
    ObservedSnapshot, ObservedVm,
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
    #[error(
        "Enter a valid hostname, user-data up to 2 MiB, and optional network-config up to 1 MiB; NUL bytes are not supported"
    )]
    InvalidCloudInit,
    #[error("Could not prepare the private cloud-init media")]
    CloudInitMedia,
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
    #[error("Invalid or unsupported disk image; choose a complete raw or fixed/dynamic VHD image")]
    DiskImage,
    #[error("Unsupported or invalid OVF package: {0}")]
    Ovf(&'static str),
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

    #[error("{0}")]
    SecurityPrerequisite(&'static str),
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
    #[error(
        "Enter a changed snapshot name (1–255 characters) and description (up to 4096 characters) without unsupported control characters"
    )]
    InvalidSnapshotMetadata,
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
        "TLS inspection could not complete a valid signed handshake or decode a bounded X.509 certificate; no credentials were sent"
    )]
    CertificateInspection,
    #[error("Enter a valid HTTP proxy URL without embedded credentials, a path, query or fragment")]
    ProxyConfiguration,
    #[error(
        "Only HTTP CONNECT proxies are supported. HTTPS proxies, SOCKS and PAC are not supported; choose a supported proxy or explicitly choose Direct"
    )]
    UnsupportedProxy,
    #[error(
        "Enter this HTTP proxy's password for the current session; the host password is not used for proxy authentication"
    )]
    ProxyAuthenticationRequired,
    #[error("Cannot connect to the configured HTTP proxy")]
    ProxyConnect,
    #[error("HTTP proxy refused CONNECT with status {0}; check proxy access and authentication")]
    ProxyRefused(u16),
    #[error("The HTTP proxy returned an invalid or oversized CONNECT response")]
    ProxyProtocol,
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

mod pool_join;
pub use pool_join::PoolJoinSelection;
