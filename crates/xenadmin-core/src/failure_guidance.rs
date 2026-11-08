/// Offline interpretation of a bounded XAPI error code. Guidance is informational:
/// it neither grants eligibility nor establishes an operation's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureGuidance {
    pub title: &'static str,
    pub explanation: &'static str,
    pub checks: &'static [&'static str],
}
impl FailureGuidance {
    pub fn for_api_code(code: &str) -> Option<Self> {
        if crate::xapi::safe_error_code(code) != code {
            return None;
        }
        let (title, explanation, checks): (_, _, &'static [&'static str]) = match code {
            "RBAC_PERMISSION_DENIED" | "PERMISSION_DENIED" | "SESSION_AUTHORIZATION_FAILED" => (
                "Session permission denied",
                "The authenticated session is not authorized for the requested operation.",
                &[
                    "Inspect Session permissions and the required methods in the operation review.",
                    "Reconnect as an authorized user if appropriate. Changing a role elsewhere does not change this captured review or prove earlier steps were undone.",
                ],
            ),
            "SESSION_AUTHENTICATION_FAILED" => (
                "Login refused",
                "The host did not accept the supplied login credentials.",
                &[
                    "Check the connection's username and credential source. Correct or forget an outdated saved password in Connections.",
                    "If directory authentication is involved, inspect its availability and configuration; do not repeatedly submit the same rejected credentials.",
                ],
            ),
            "SESSION_INVALID" => (
                "Session is no longer valid",
                "The host rejected the session, which can happen after expiry or a toolstack restart.",
                &[
                    "Allow the connection to recover, or reconnect in Connections.",
                    "Inspect and recheck any interrupted operation before submitting a new change.",
                ],
            ),
            "OTHER_OPERATION_IN_PROGRESS" | "HOST_EVACUATE_IN_PROGRESS" => (
                "Another operation is active",
                "The host reports conflicting work on the affected object.",
                &[
                    "Inspect current operations and Activity tasks, including tasks started by other clients.",
                    "Wait for that work to finish. Use a separately reviewed cancellation only when appropriate; cancellation is not proof that the task stopped.",
                ],
            ),
            "VM_BAD_POWER_STATE" => (
                "VM power state does not match",
                "The requested action requires a different VM power state from the one the host observed.",
                &[
                    "Refresh the VM state and inspect any current power task.",
                    "Review the intended action against the current state. A force action is a separate decision and may lose guest data.",
                ],
            ),
            "HANDLE_INVALID" | "VM_NOT_RESIDENT_HERE" => (
                "The object or its location changed",
                "The host could not use the captured object reference or resident-host location.",
                &[
                    "Wait for current inventory, then locate the object by UUID and inspect its host.",
                    "If a move or deletion was in progress, inspect its result before making another change.",
                ],
            ),
            "OPERATION_NOT_ALLOWED" | "OPERATION_BLOCKED" => (
                "The host disallowed this operation",
                "An operation restriction was reported. This code alone does not identify the restriction.",
                &[
                    "Inspect the object's current operations, administrative block message and action-availability check.",
                    "Preserve an intentional administrative restriction. Use private diagnostics if the host's reason is absent or contradictory.",
                ],
            ),
            "VM_MISSING_PV_DRIVERS" | "VM_OLD_PV_DRIVERS" => (
                "Guest tools are missing or outdated",
                "The operation requires guest drivers that the host could not detect at the required version.",
                &[
                    "Inspect General information for guest tools and OS status.",
                    "Use appropriate guest tools installation media and complete installation inside the guest. Mounting an ISO alone does not install the tools.",
                ],
            ),
            "VM_LACKS_FEATURE" | "VM_NON_SUSPENDABLE" => (
                "Guest capability is unavailable",
                "The VM does not advertise a capability required by this operation.",
                &[
                    "Inspect the guest's tools, configuration and current advertised capabilities.",
                    "Choose a supported workflow after reviewing its different effects; do not treat an unavailable checkpoint or suspend as a disk-only snapshot.",
                ],
            ),
            "VM_FAILED_SHUTDOWN_ACK"
            | "VM_FAILED_SUSPEND_ACK"
            | "VM_SHUTDOWN_TIMEOUT"
            | "VM_SUSPEND_TIMEOUT"
            | "VMS_FAILED_TO_COOPERATE" => (
                "The guest did not cooperate in time",
                "The guest did not complete or acknowledge the requested coordinated operation.",
                &[
                    "Inspect the console, guest tools and current task before deciding whether it is still progressing.",
                    "A force shutdown or reboot requires its own review and can lose unsaved data; this guidance does not perform one.",
                ],
            ),
            "HOST_NOT_ENOUGH_FREE_MEMORY" | "HOST_NOT_ENOUGH_PCPUS" => (
                "Host capacity is insufficient",
                "The chosen host could not provide the CPU or memory capacity required by this operation.",
                &[
                    "Inspect host capacity and the VM's configured CPU and memory limits, including HA reservations.",
                    "Review a compatible alternative host or an explicit resource change; do not infer available capacity from a single free-memory sample.",
                ],
            ),
            "MEMORY_CONSTRAINT_VIOLATION"
            | "MEMORY_CONSTRAINT_VIOLATION_ORDER"
            | "MEMORY_CONSTRAINT_VIOLATION_MAXPIN"
            | "VM_MEMORY_SIZE_TOO_LOW" => (
                "Memory limits are unsuitable",
                "The host rejected the VM's configured memory range or minimum size.",
                &[
                    "Reload VM properties and review static and dynamic bounds together.",
                    "Check the guest's requirements before changing its limits. A successful setter does not establish guest memory availability.",
                ],
            ),
            "HOST_OFFLINE"
            | "HOST_NOT_LIVE"
            | "CANNOT_CONTACT_HOST"
            | "VM_MIGRATE_CONTACT_REMOTE_SERVICE_FAILED" => (
                "A required host cannot be reached",
                "The server could not contact a host required by the operation.",
                &[
                    "Check that host's management path, status and current coordinator through a known connection.",
                    "For migration, inspect connectivity between both hosts and the selected transfer network. A client connection alone does not prove host-to-host reachability.",
                ],
            ),
            "HOST_DISABLED" | "HOST_DISABLED_UNTIL_REBOOT" | "HOST_DISABLED_INDEFINITELY" => (
                "Host scheduling is disabled",
                "The host is not currently accepting the operation in its scheduling state.",
                &[
                    "Inspect maintenance, current tasks and the host's reported reason.",
                    "Leaving maintenance or rebooting is a separate reviewed change; do not re-enable a host while another maintenance workflow is active.",
                ],
            ),
            "HOST_HAS_RESIDENT_VMS" | "HOST_IN_USE" | "HOST_NOT_DISABLED" => (
                "Host is still in use",
                "The operation requires a host that is disabled or free of active consumers.",
                &[
                    "Inspect resident guests, reserved operations and storage consumers.",
                    "Review maintenance and evacuation separately, including destination capacity. This failure does not authorize shutting down guests.",
                ],
            ),
            "SR_FULL" | "SR_SOURCE_SPACE_INSUFFICIENT" | "SR_SUSPEND_SPACE_INSUFFICIENT" => (
                "Storage capacity is insufficient",
                "The storage repository cannot provide the capacity needed for the requested data or memory image.",
                &[
                    "Inspect physical usage, virtual allocation, snapshots and any checkpoint memory requirements on the selected SRs.",
                    "Review an eligible alternative SR or explicitly selected cleanup. Deleting a snapshot or disk has different ownership and data-loss consequences.",
                ],
            ),
            "SR_NOT_ATTACHED" | "SR_NO_PBDS" | "SR_ATTACH_FAILED" | "VM_REQUIRES_SR" => (
                "Required storage is not accessible",
                "A host does not have a usable attachment to storage required by the operation.",
                &[
                    "Inspect the SR's per-host attachments and the affected VM's disk consumers.",
                    "Check the storage path and backend configuration before reviewing repair or reattachment. A rescan alone does not establish path health.",
                ],
            ),
            "SR_BACKEND_FAILURE" | "SR_VDI_LOCKING_FAILED" | "SR_UNHEALTHY" => storage_backend(),
            value
                if value
                    .strip_prefix("SR_BACKEND_FAILURE_")
                    .is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
                    }) =>
            {
                storage_backend()
            }
            "VDI_IN_USE" | "SR_DEVICE_IN_USE" | "SR_NOT_EMPTY" | "SR_HAS_PBD" => (
                "Storage still has consumers",
                "The selected disk or repository is in use or still has attachments that prevent this operation.",
                &[
                    "Inspect all VM, snapshot, suspend-image and host attachments, including other clients' tasks.",
                    "Review detachment or owned-resource cleanup explicitly. An absent running VM is not proof that a disk is unused.",
                ],
            ),
            "VDI_NOT_AVAILABLE" | "VDI_MISSING" | "VDI_LOCATION_MISSING" | "VM_REQUIRES_VDI" => (
                "A required disk is unavailable",
                "The host could not locate or attach a required virtual disk.",
                &[
                    "Inspect the disk UUID, its SR and host attachment/path observations.",
                    "Preserve existing disk and snapshot records while investigating the backend. Do not create a replacement under the same name as a recovery shortcut.",
                ],
            ),
            "SR_OPERATION_NOT_SUPPORTED"
            | "SR_UNKNOWN_DRIVER"
            | "SR_DOES_NOT_SUPPORT_MIGRATION"
            | "VDI_IS_A_PHYSICAL_DEVICE"
            | "VDI_INCOMPATIBLE_TYPE"
            | "VDI_CBT_ENABLED" => (
                "Storage capability does not match",
                "The selected backend or disk type does not support this operation in its current configuration.",
                &[
                    "Inspect the SR's advertised capabilities and the disk's type, sharing and tracking flags.",
                    "Choose a workflow supported by that exact host and storage driver. Do not disable tracking or change disk type merely to bypass the refusal.",
                ],
            ),
            "NETWORK_CONTAINS_VIF" | "NETWORK_CONTAINS_PIF" => (
                "Network still has consumers",
                "VM or host interfaces are attached to the network required to be unused.",
                &[
                    "Inspect both guest NICs and host interfaces on the network.",
                    "Review each required migration or detachment separately, including management and storage reachability.",
                ],
            ),
            "PIF_IS_MANAGEMENT_IFACE"
            | "PIF_DOES_NOT_ALLOW_UNPLUG"
            | "HA_CANNOT_CHANGE_BOND_STATUS_OF_MGMT_IFACE" => (
                "Protected host connectivity",
                "The requested change affects a protected host interface or management path.",
                &[
                    "Inspect management addressing, HA state and the full physical/VLAN/bond topology.",
                    "Use a supported reviewed management handover with a verified recovery path. Do not unplug the interface to bypass this check.",
                ],
            ),
            "PIF_HAS_NO_NETWORK_CONFIGURATION"
            | "PIF_HAS_NO_V6_NETWORK_CONFIGURATION"
            | "PIF_INCOMPATIBLE_PRIMARY_ADDRESS_TYPE"
            | "PIF_CONFIGURATION_ERROR" => (
                "Host network configuration is unsuitable",
                "The interface lacks a compatible address configuration for the requested operation.",
                &[
                    "Inspect IPv4/IPv6 modes, addresses, routes and the selected primary address family.",
                    "Review a supported addressing change with an independently reachable recovery endpoint before changing management connectivity.",
                ],
            ),
            "VM_REQUIRES_NET" | "HOST_CANNOT_ATTACH_NETWORK" => (
                "Required network is unavailable on the host",
                "The chosen host cannot provide a network required by this VM.",
                &[
                    "Inspect every VM NIC's network and the destination host's PIF/VLAN/bond attachments.",
                    "Review complete destination mappings or choose a compatible host; a matching network name does not establish identity or connectivity.",
                ],
            ),
            "VM_INCOMPATIBLE_WITH_THIS_HOST"
            | "VM_HOST_INCOMPATIBLE_VERSION"
            | "VM_HOST_INCOMPATIBLE_VERSION_MIGRATE"
            | "VM_HOST_INCOMPATIBLE_VIRTUAL_HARDWARE_PLATFORM_VERSION" => (
                "Destination host is incompatible",
                "CPU features, host versions or virtual hardware compatibility prevent the operation.",
                &[
                    "Compare the source VM's configuration with the destination host's reported capabilities and migration checks.",
                    "Choose a compatible destination or a separately planned configuration change. Do not force migration past a host compatibility refusal.",
                ],
            ),
            "HA_OPERATION_WOULD_BREAK_FAILOVER_PLAN" | "HA_NO_PLAN" => (
                "HA restart protection cannot be maintained",
                "The host cannot guarantee the required restart plan with the proposed change.",
                &[
                    "Inspect protected VMs, restart priorities, failure tolerance and available capacity in Pool HA.",
                    "Review capacity or policy changes explicitly; disabling HA or allowing overcommit reduces protection and is not an automatic remedy.",
                ],
            ),
            "HA_HOST_CANNOT_ACCESS_STATEFILE"
            | "HA_LOST_STATEFILE"
            | "HA_HOST_CANNOT_SEE_PEERS"
            | "HA_FAILED_TO_FORM_LIVESET"
            | "HA_HEARTBEAT_DAEMON_STARTUP_FAILED" => (
                "HA heartbeat prerequisites failed",
                "HA could not establish or maintain the required storage or peer heartbeat state.",
                &[
                    "Inspect heartbeat SR access from every member and host-to-host management connectivity.",
                    "Preserve fencing protections while investigating. Ordinary connection recovery must not force coordinator promotion or override quorum.",
                ],
            ),
            "HA_IS_ENABLED" | "HA_HOST_IS_ARMED" => (
                "HA prevents this change",
                "The operation is incompatible with the current HA or fencing state.",
                &[
                    "Inspect the pool's HA state and the operation's documented prerequisites.",
                    "Any change to HA protection needs its own review and recovery plan; do not disable fencing as an incidental retry.",
                ],
            ),
            "IMPORT_ERROR_PREMATURE_EOF"
            | "IMPORT_ERROR_SOME_CHECKSUMS_FAILED"
            | "IMPORT_ERROR_UNEXPECTED_FILE" => (
                "Archive data is incomplete or invalid",
                "The importer found truncated, corrupt or unexpected archive content.",
                &[
                    "Validate the original file and available local disk space; compare its size and checksum with the source.",
                    "Inspect retained import resources and its existing outcome before choosing another file. Do not resume by replaying an uncertain upload.",
                ],
            ),
            "IMPORT_INCOMPATIBLE_VERSION"
            | "IMPORT_ERROR_CANNOT_HANDLE_CHUNKED"
            | "IMPORT_ERROR_FAILED_TO_FIND_OBJECT"
            | "IMPORT_ERROR_ATTACHED_DISKS_NOT_FOUND" => (
                "Archive configuration is incompatible or incomplete",
                "The importer could not resolve the archive version, layout or required objects.",
                &[
                    "Inspect the archive's supported format and explicit disk/network mappings.",
                    "Use an interoperable export or deliberate conversion of a copy. Retain any resources from a partial import until their ownership and consumers are verified.",
                ],
            ),
            "VM_IS_TEMPLATE" | "VM_CANNOT_DELETE_DEFAULT_TEMPLATE" => (
                "Template restriction",
                "The requested VM operation is not valid for this template or protected default template.",
                &[
                    "Confirm the object's UUID and type in Templates.",
                    "Use a supported template creation/property workflow; do not clear a type or default-template flag to bypass the restriction.",
                ],
            ),
            "MESSAGE_METHOD_UNKNOWN"
            | "MESSAGE_REMOVED"
            | "MESSAGE_DEPRECATED"
            | "NOT_IMPLEMENTED"
            | "NOT_SUPPORTED" => (
                "Host API capability is unavailable",
                "The host does not expose this operation in the requested form.",
                &[
                    "Inspect the host version, negotiated capabilities and private diagnostic method code.",
                    "Use the supported workflow for that exact host version. A newer API description alone does not establish host support.",
                ],
            ),
            "TASK_CANCELLED" => (
                "The host reports task cancellation",
                "The task was cancelled, but earlier steps or created resources may remain.",
                &[
                    "Inspect the operation's per-step outcome and retained resource UUIDs.",
                    "Review any cleanup separately; cancellation does not roll back completed changes.",
                ],
            ),
            "UNKNOWN_API_ERROR" => (
                "The host error code is unavailable",
                "A usable XAPI error code was not available. This is not evidence of a specific administrative restriction.",
                &[
                    "Inspect the original operation and a fresh action-availability check if relevant.",
                    "Use private live-client diagnostics to inspect sanitized method and outcome evidence. Keep host names, identifiers and diagnostic exports private.",
                ],
            ),
            _ => (
                "Host error needs investigation",
                "This error code has no specific guidance in this client version.",
                &[
                    "Keep the original code and inspect the affected object's current state, tasks and operation steps.",
                    "Use private diagnostics and the documentation for the exact host version. Do not infer permission to retry from an unrecognized code.",
                ],
            ),
        };
        Some(Self {
            title,
            explanation,
            checks,
        })
    }
}
fn storage_backend() -> (&'static str, &'static str, &'static [&'static str]) {
    (
        "Storage backend reported a failure",
        "The storage driver failed. This code alone does not identify the device, path or backend cause.",
        &[
            "Inspect the SR and per-host path observations, then collect the affected host's support or storage logs privately.",
            "Verify existing tasks and disk identities before repair. Do not destroy, forget or recreate storage as an automatic response.",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_codes_and_numeric_backend_subcodes_have_bounded_guidance() {
        assert_eq!(
            FailureGuidance::for_api_code("SR_BACKEND_FAILURE_44"),
            FailureGuidance::for_api_code("SR_BACKEND_FAILURE")
        );
        assert_ne!(
            FailureGuidance::for_api_code("SR_BACKEND_FAILURE_NEW"),
            FailureGuidance::for_api_code("SR_BACKEND_FAILURE")
        );
        for code in [
            "",
            "RBAC_PERMISSION_DENIED\nsecret",
            "SR_FULL parameter",
            "sr_full",
            &"A".repeat(101),
        ] {
            assert!(FailureGuidance::for_api_code(code).is_none());
        }
        assert_eq!(
            FailureGuidance::for_api_code("FUTURE_ERROR").unwrap().title,
            "Host error needs investigation"
        );
    }
    #[test]
    fn unknown_error_does_not_assert_admin_block_or_retry_authority() {
        let guide = FailureGuidance::for_api_code("UNKNOWN_API_ERROR").unwrap();
        assert!(guide.explanation.contains("not evidence"));
        for code in [
            "HA_OPERATION_WOULD_BREAK_FAILOVER_PLAN",
            "PIF_IS_MANAGEMENT_IFACE",
            "TASK_CANCELLED",
            "IMPORT_ERROR_PREMATURE_EOF",
        ] {
            let guide = FailureGuidance::for_api_code(code).unwrap();
            assert!(!guide.checks.is_empty());
            assert_ne!(guide.title, "Host error needs investigation");
        }
    }
}
