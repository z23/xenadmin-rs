//! The reviewed pool operations: the logic of the Rune `pool_view.rs` and
//! its child module `ha_policy_view.rs`, carried over.
//!
//! The Rune editor was one dialog that showed every control of the pool and
//! turned into the review of the one operation that was staged. The editor
//! here is the same struct with one field more, [`Kind`]: which part of that
//! dialog it is (a page of the pool's Properties dialog, the Configure HA
//! wizard, or the dialog of one command). Staging, the confirmation rule,
//! the staleness check, the submission and every text are the Rune ones.

use std::collections::BTreeSet;
use std::sync::Arc;

use xenadmin_core::{
    ApplianceOrder, HaRestartPriority, HostId, Inventory, ObservedResource, OperationId,
    PoolAdministration, PoolId, PoolOperation, ResourceCommand, ResourceTarget, SrId, VmId,
    VmStartupField,
};

use crate::prelude::*;

/// The refusal of a review that was made against another state of the
/// connection (Rune `apply`).
pub const STALE: &str = "The connection changed; reopen pool administration";

/// The refusal of a confirmation that is not the requested UUID.
pub const CONFIRM: &str = "Enter the exact requested UUID";

/// The closing line of every review.
pub const CLOSING: &str = "Fresh host checks run again before submission. Interrupted operations stay in history for recheck; no mutation is replayed automatically.";

/// The three settings of [`PoolOperation::SetVmStartup`], in the order of
/// the three drafts.
pub const FIELDS: [VmStartupField; 3] = [
    VmStartupField::Order,
    VmStartupField::StartDelay,
    VmStartupField::ShutdownDelay,
];

/// The three priorities a VM can be given, in the Rune order.
pub const PRIORITIES: [HaRestartPriority; 3] = [
    HaRestartPriority::Restart,
    HaRestartPriority::BestEffort,
    HaRestartPriority::Disabled,
];

/// Which part of the Rune pool dialog an editor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Page Security of the pool's Properties dialog: HTTPS only, TLS
    /// verification between members, rotation of the pool secret.
    Security,
    /// Page Network Options: migration compression.
    Network,
    /// The Configure HA wizard: heartbeat storage, then the review.
    Configure,
    /// Disable HA: opens on its review.
    Disable,
    /// Host failures to tolerate: a draft.
    Failures,
    /// Allow or disallow HA overcommit: opens on its review.
    Overcommit,
    /// The persistent HA timeout default: a draft.
    Timeout,
    /// One VM: its restart priority, startup order and delays.
    Protection,
    /// Planned coordinator handover to one member: opens on its review.
    Handover,
    /// Ejection of one member. From a server it opens on its review; from
    /// the pool the member is chosen first.
    Eject,
}

impl Kind {
    /// The id of the Properties page the kind is, if it is one.
    pub fn page(self) -> Option<&'static str> {
        match self {
            Self::Security => Some("security"),
            Self::Network => Some("network-options"),
            _ => None,
        }
    }

    pub fn of_page(id: &str) -> Option<Self> {
        [Self::Security, Self::Network]
            .into_iter()
            .find(|kind| kind.page() == Some(id))
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Security => "Security",
            Self::Network => "Network Options",
            Self::Configure => "Configure HA",
            Self::Disable => "Disable high availability",
            Self::Failures => "Host failures to tolerate",
            Self::Overcommit => "HA overcommit",
            Self::Timeout => "HA timeout default",
            Self::Protection => "VM restart priority",
            Self::Handover => "Hand over coordinator",
            Self::Eject => "Remove server from pool",
        }
    }
}

pub struct Editor {
    pub kind: Kind,
    pub pool: PoolId,
    pub name: String,
    pub observed: Option<ObservedResource>,
    pub inventory: Arc<Inventory>,
    pub heartbeat: BTreeSet<SrId>,
    pub failures: String,
    pub timeout: String,
    pub startup: [String; 3],
    pub vm: Option<VmId>,
    pub host: Option<HostId>,
    pub review: Option<PoolOperation>,
    /// The review was staged when the editor opened: there is no draft to go
    /// back to.
    pub direct: bool,
    pub confirmation: String,
    pub error: Option<String>,
}

/// The pool's administration record, all unknown when the pool's details
/// are not in the inventory.
pub fn administration(inventory: &Inventory) -> PoolAdministration {
    inventory
        .details
        .pool
        .as_ref()
        .map(|p| p.administration.clone())
        .unwrap_or_default()
}

impl Editor {
    /// The Rune `open`, for the pool of the command's target.
    pub fn capture(m: &Model, pool: PoolId, kind: Kind) -> Option<Self> {
        let inventory = m.inventory_for(pool)?;
        let administration = administration(&inventory);
        Some(Self {
            kind,
            pool,
            name: inventory.pool_name.clone(),
            observed: m
                .snapshot_for(pool)
                .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(pool)).ok()),
            heartbeat: BTreeSet::new(),
            failures: administration
                .ha_failures_to_tolerate
                .unwrap_or(0)
                .to_string(),
            timeout: administration.ha_timeout_override.unwrap_or_default(),
            startup: std::array::from_fn(|_| String::new()),
            vm: None,
            host: None,
            review: None,
            direct: false,
            confirmation: String::new(),
            error: None,
            inventory,
        })
    }

    pub fn administration(&self) -> PoolAdministration {
        administration(&self.inventory)
    }

    /// The step of the Configure HA wizard that is showing.
    pub fn step(&self) -> usize {
        usize::from(self.review.is_some())
    }
}

/// Opens the editor of `kind` for a pool, in place of whatever this area
/// had open. False when the pool has no inventory.
pub fn open(m: &mut Model, pool: PoolId, kind: Kind) -> bool {
    let Some(editor) = Editor::capture(m, pool, kind) else {
        return false;
    };
    super::close_all(m);
    m.areas.pool.editor = Some(editor);
    true
}

/// Opens an editor directly on the review of `op`.
pub fn open_review(m: &mut Model, pool: PoolId, kind: Kind, op: PoolOperation) {
    if open(m, pool, kind) {
        stage(m, op);
        if let Some(editor) = &mut m.areas.pool.editor {
            editor.direct = true;
        }
    }
}

/// Every review enters through here: a new review empties the confirmation.
pub fn stage(m: &mut Model, op: PoolOperation) {
    if let Some(editor) = &mut m.areas.pool.editor {
        editor.review = Some(op);
        editor.error = None;
        editor.confirmation = String::new();
    }
}

/// Back leaves the review; the error and the typed confirmation stay.
pub fn back(m: &mut Model) {
    if let Some(editor) = &mut m.areas.pool.editor {
        editor.review = None;
    }
}

/// The UUID the review asks for: the host's for an ejection, the pool's
/// for four operations, none for the rest.
pub fn confirmation(op: &PoolOperation, pool: PoolId) -> Option<String> {
    match op {
        PoolOperation::EjectHost { host, .. } => Some(host.to_string()),
        PoolOperation::DesignateCoordinator { .. }
        | PoolOperation::DisableHa
        | PoolOperation::RotateSecret
        | PoolOperation::EnableTlsVerification => Some(pool.to_string()),
        _ => None,
    }
}

pub fn current(m: &Model, editor: &Editor) -> bool {
    m.snapshot_for(editor.pool).and_then(|snapshot| {
        snapshot
            .observe_resource(ResourceTarget::Pool(editor.pool))
            .ok()
    }) == editor.observed
        && editor.observed.is_some()
}

/// The editor's pool is a synthetic one: nothing can be submitted.
pub fn synthetic(m: &Model, editor: &Editor) -> bool {
    m.is_synthetic_pool(editor.pool)
}

/// Why the review cannot be confirmed any more, outside a synthetic pool.
pub fn stale(m: &Model, editor: &Editor) -> Option<String> {
    (editor.review.is_some()
        && !synthetic(m, editor)
        && !(m.can_mutate_pool(editor.pool) && current(m, editor)))
    .then(|| STALE.to_owned())
}

/// The confirmation the review asks for is not typed exactly.
pub fn unconfirmed(editor: &Editor) -> bool {
    editor.review.as_ref().is_some_and(|op| {
        confirmation(op, editor.pool).is_some_and(|required| editor.confirmation != required)
    })
}

/// The Rune `apply`. The permission check is new in the function: Rune
/// refused a known denial on the button only.
pub fn apply(m: &mut Model) {
    let result = (|| {
        let editor = m.areas.pool.editor.as_ref().ok_or("Pool view is closed")?;
        let op = editor.review.as_ref().ok_or("Review a change first")?;
        if synthetic(m, editor) || !m.can_mutate_pool(editor.pool) || !current(m, editor) {
            return Err(STALE.to_owned());
        }
        if unconfirmed(editor) {
            return Err(CONFIRM.into());
        }
        let permission = permissions::assess_for(m, editor.pool, &op.permission_requirements());
        if let Some(denial) = permissions::denial(&permission) {
            return Err(denial);
        }
        m.connection_for(editor.pool)
            .ok_or("Connect before applying changes")?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Pool {
                    target: editor
                        .observed
                        .ok_or("Current pool observation is unavailable")?,
                    operation: op.clone(),
                },
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(handle) => {
            m.track_resource(handle);
            super::close(m);
        }
        Err(error) => {
            if let Some(editor) = &mut m.areas.pool.editor {
                editor.error = Some(error);
            }
        }
    }
}

fn refuse(m: &mut Model, error: &str) {
    if let Some(editor) = &mut m.areas.pool.editor {
        editor.error = Some(error.into());
    }
}

pub fn review_failures(m: &mut Model) {
    let result = (|| {
        let editor = m
            .areas
            .pool
            .editor
            .as_ref()
            .ok_or("Pool administration is closed")?;
        let expected = editor
            .administration()
            .ha_failures_to_tolerate
            .ok_or("HA failure tolerance is unavailable")?;
        let failures: u64 = editor
            .failures
            .parse()
            .map_err(|_| "Enter a whole number of host failures")?;
        if failures >= editor.inventory.hosts.len() as u64 || failures == expected {
            return Err("Choose a different value below the number of pool hosts");
        }
        Ok(PoolOperation::SetHaFailuresToTolerate { expected, failures })
    })();
    match result {
        Ok(op) => stage(m, op),
        Err(error) => refuse(m, error),
    }
}

/// The priority of the editor's VM as the host reports it; `None` when it
/// is unknown or not one this client knows.
pub fn current_priority(inventory: &Inventory, vm: VmId) -> Option<HaRestartPriority> {
    inventory
        .details
        .vms
        .get(&vm)
        .and_then(|vm| vm.ha_restart_priority.as_deref())
        .and_then(HaRestartPriority::parse)
}

pub fn priority(m: &mut Model, value: HaRestartPriority) {
    let op = m.areas.pool.editor.as_ref().and_then(|editor| {
        let vm = editor.vm?;
        let expected = current_priority(&editor.inventory, vm)?;
        (expected != value).then_some(PoolOperation::SetVmRestartPriority {
            vm,
            expected,
            priority: value,
        })
    });
    if let Some(op) = op {
        stage(m, op);
    }
}

/// The lines of the review of an operation, word for word the Rune ones.
pub fn describe(editor: &Editor, op: &PoolOperation) -> Vec<String> {
    let host = |id: HostId| {
        editor
            .inventory
            .hosts
            .get(&id)
            .map(|h| format!("{} ({id})", h.name))
            .unwrap_or_else(|| id.to_string())
    };
    let vm_name = |id: &VmId| {
        editor
            .inventory
            .vms
            .get(id)
            .map(|v| v.name.as_str())
            .unwrap_or("Unknown")
    };
    match op {
        PoolOperation::Join { .. } => vec!["Join another independently connected pool".into()],
        PoolOperation::Security(op) => vec![op.label().into()],
        PoolOperation::SetMigrationCompression { expected, enabled } => vec![
            format!("Migration compression: {expected} → {enabled}"),
            "Applies to subsequent migrations; compression trades host CPU for lower network usage.".into(),
        ],
        PoolOperation::SetHttpsOnly { expected, enabled } => vec![
            format!("HTTPS-only access: {expected} → {enabled}"),
            "Changes the pool hosts’ HTTP firewall policy. Existing tools that rely on plain HTTP may stop working.".into(),
        ],
        PoolOperation::DesignateCoordinator { expected, host: target } => vec![
            format!("Current coordinator: {}", host(*expected)),
            format!("New coordinator: {}", host(*target)),
            "Performs a planned handover. This connection may briefly disconnect or move to the new coordinator. HA must already be disabled. Does not evacuate or shut down guests.".into(),
        ],
        PoolOperation::SetHaFailuresToTolerate { expected, failures } => vec![
            format!("Host failures to tolerate: {expected} → {failures}"),
            "The server must confirm a failover plan exists before submission. A lower value reduces the promised failure tolerance.".into(),
        ],
        PoolOperation::EnableHa { heartbeat_srs, expected_failures } => {
            let mut lines = vec![
                format!("Enable HA with failure tolerance {expected_failures}."),
                "HA can fence and reboot a host that loses heartbeat connectivity. Requires prepared storage and management networking; three or more hosts are recommended.".into(),
            ];
            for id in heartbeat_srs {
                lines.push(format!(
                    "Heartbeat storage: {} ({id})",
                    editor
                        .inventory
                        .storage
                        .iter()
                        .find(|sr| sr.uuid == id.uuid())
                        .map(|sr| sr.name.as_str())
                        .unwrap_or("Unknown")
                ));
            }
            lines
        }
        PoolOperation::SetHaAllowOvercommit { expected, enabled } => vec![
            format!("Allow HA overcommit: {expected} → {enabled}"),
            "Allowing overcommit permits changes that can leave insufficient capacity to restart protected VMs. This does not add capacity or change configured failure tolerance. Disallowing it does not by itself resolve existing overcommit.".into(),
        ],
        PoolOperation::SetHaTimeout { expected, seconds } => vec![
            format!(
                "HA timeout default: {} → {}",
                expected.as_deref().unwrap_or("host default"),
                seconds
                    .map(|v| format!("{v} seconds"))
                    .unwrap_or("host default".into())
            ),
            "HA must be disabled. This sets the persistent default used at the next HA enable, and preserves other pool settings. Short timeouts increase fencing risk; long timeouts delay failure detection. No automatic enable or restart is performed.".into(),
        ],
        PoolOperation::SetVmStartup { vm, appliance, field, expected, value } => vec![
            format!("VM: {} ({vm})", vm_name(vm)),
            format!(
                "{}: {expected} → {value}{}",
                field.label(),
                if *field == VmStartupField::Order { "" } else { " seconds" }
            ),
            format!(
                "Captured vApp: {}",
                appliance.map(|id| id.to_string()).unwrap_or("none".into())
            ),
            "Startup order is shared with vApp ordering and applies to protected VM HA restarts. Delays apply to VM start/shutdown processing. These settings do not start or stop the guest or guarantee application readiness.".into(),
        ],
        PoolOperation::DisableHa => vec![
            "Disables automatic HA recovery for the entire pool. Protected VMs will no longer be restarted automatically after host failure.".into(),
        ],
        PoolOperation::SetVmRestartPriority { vm, expected, priority } => vec![
            format!("VM: {} ({vm})", vm_name(vm)),
            format!("HA priority: {} → {}", expected.label(), priority.label()),
            "Restart protection requires agile storage/network placement. This does not start or stop the VM.".into(),
        ],
        PoolOperation::EjectHost { host: target, .. } => vec![
            format!("Host leaving the pool: {}", host(*target)),
            "The host will reboot and its pool configuration will be reset. Its local disks must be empty of guest data. Disable and evacuate this host first; HA must already be disabled. No guest shutdown or disk deletion is performed by this workflow.".into(),
        ],
        PoolOperation::EnableTlsVerification => vec![
            "Enables certificate verification between all pool members. This is separate from this application's connection trust setting. Every member must support the feature and trust its peers; upgraded pools need all members on XCP-ng 8.3.".into(),
        ],
        PoolOperation::RotateSecret => vec![
            "Rotates the shared authentication secret used between pool members. All hosts must be reachable and HA must already be disabled. No secret is shown or stored by this client.".into(),
        ],
    }
}

/// "Enabled", "Disabled", or the words for a value the host did not give.
pub fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Enabled",
        Some(false) => "Disabled",
        None => "Unknown / unavailable",
    }
}

// -- Heartbeat storage (Configure HA) -------------------------------------------

/// The repositories that can carry the heartbeat: shared, with user
/// content. At most the first hundred, as in Rune.
pub fn heartbeat_candidates(inventory: &Inventory) -> Vec<(SrId, String)> {
    inventory
        .storage
        .iter()
        .filter(|sr| {
            inventory
                .details
                .storage
                .get(&SrId::from(sr.uuid))
                .is_some_and(|details| {
                    details.shared == Some(true) && details.content_type == "user"
                })
        })
        .take(100)
        .map(|sr| (SrId::from(sr.uuid), sr.name.clone()))
        .collect()
}

/// At most eight repositories.
pub fn toggle_heartbeat(m: &mut Model, id: SrId) {
    if let Some(editor) = &mut m.areas.pool.editor {
        let chosen = &mut editor.heartbeat;
        if !chosen.remove(&id) && chosen.len() < 8 {
            chosen.insert(id);
        }
    }
}

/// "Review enabling HA": the chosen repositories and the failure tolerance
/// the review was made against, which the operation does not set.
pub fn review_enable(m: &mut Model) {
    let op = m.areas.pool.editor.as_ref().and_then(|editor| {
        let expected_failures = editor.administration().ha_failures_to_tolerate?;
        (!editor.heartbeat.is_empty()).then(|| PoolOperation::EnableHa {
            heartbeat_srs: editor.heartbeat.iter().copied().collect(),
            expected_failures,
        })
    });
    if let Some(op) = op {
        stage(m, op);
    }
}

// -- Membership -------------------------------------------------------------------

/// "Eject selected member…" for the member chosen in the editor.
pub fn review_eject(m: &mut Model) {
    let op = m.areas.pool.editor.as_ref().and_then(|editor| {
        let host = editor.host?;
        let row = editor.inventory.hosts.get(&host)?;
        Some(PoolOperation::EjectHost {
            host,
            expected_name: row.name.clone(),
        })
    });
    if let Some(op) = op {
        stage(m, op);
    }
}

/// Why a member cannot be ejected, or be made the coordinator. Rune
/// disabled both buttons on these conditions without a text.
pub fn member_refusal(inventory: &Inventory, host: HostId, handover: bool) -> Option<&'static str> {
    let administration = administration(inventory);
    if administration.coordinator == Some(host) {
        return Some(if handover {
            "This server is already the pool coordinator."
        } else {
            "This server is the pool coordinator."
        });
    }
    (administration.ha_enabled != Some(false)).then_some(super::HA_FIRST)
}

// -- Rune `ha_policy_view.rs` -----------------------------------------------------

pub fn values(order: ApplianceOrder) -> Option<[u64; 3]> {
    Some([
        order.order.try_into().ok()?,
        order.start_delay,
        order.shutdown_delay,
    ])
}

/// The order and delays of a VM, when all three are known.
pub fn startup_values(inventory: &Inventory, vm: VmId) -> Option<[u64; 3]> {
    inventory
        .details
        .vms
        .get(&vm)
        .and_then(|v| v.appliance_order)
        .and_then(values)
}

/// Choosing a VM fills the three drafts from its settings, or empties them.
pub fn select_vm(m: &mut Model, id: VmId) {
    let Some(e) = m.areas.pool.editor.as_mut() else {
        return;
    };
    e.vm = Some(id);
    let fields = startup_values(&e.inventory, id);
    e.startup = std::array::from_fn(|i| fields.map(|v| v[i].to_string()).unwrap_or_default());
}

pub fn startup(m: &mut Model, index: usize) {
    let result = (|| {
        let e = m
            .areas
            .pool
            .editor
            .as_ref()
            .ok_or("Pool administration is closed")?;
        let vm = e.vm.ok_or("Select a VM")?;
        let d = e
            .inventory
            .details
            .vms
            .get(&vm)
            .ok_or("VM details are unavailable")?;
        if !d.appliance_known {
            return Err("vApp membership is unknown");
        }
        let old = d
            .appliance_order
            .and_then(values)
            .ok_or("Startup settings are unknown")?;
        let value = e.startup[index]
            .parse::<u64>()
            .map_err(|_| "Enter a nonnegative whole number")?;
        if value > if index == 0 { i64::MAX as u64 } else { 86_400 } {
            return Err(
                "Order must fit a signed 64-bit number; delays must be at most 86400 seconds",
            );
        }
        if value == old[index] {
            return Err("This setting has not changed");
        }
        Ok(PoolOperation::SetVmStartup {
            vm,
            appliance: d.appliance,
            field: FIELDS[index],
            expected: old[index],
            value,
        })
    })();
    match result {
        Ok(op) => stage(m, op),
        Err(error) => refuse(m, error),
    }
}

/// The refusal of a timeout change while HA is on, or while a separate
/// configuration timeout exists.
pub const TIMEOUT_FIXED: &str =
    "Disable HA first; a separate HA configuration timeout cannot be overwritten here";

/// The timeout default can be edited: HA is off and the configuration is
/// fully readable.
pub fn timeout_editable(a: &PoolAdministration) -> bool {
    a.ha_timeout_known && a.ha_enabled == Some(false) && a.ha_configuration_timeout.is_none()
}

pub fn timeout(m: &mut Model, reset: bool) {
    let result = (|| {
        let e = m
            .areas
            .pool
            .editor
            .as_ref()
            .ok_or("Pool administration is closed")?;
        let a = &e
            .inventory
            .details
            .pool
            .as_ref()
            .ok_or("Pool settings unavailable")?
            .administration;
        if !timeout_editable(a) {
            return Err(TIMEOUT_FIXED);
        }
        let seconds = if reset {
            None
        } else {
            Some(
                e.timeout
                    .parse::<u32>()
                    .ok()
                    .filter(|v| (10..=86_400).contains(v))
                    .ok_or("Enter 10 to 86400 seconds")?,
            )
        };
        if a.ha_timeout_override == seconds.map(|v| v.to_string()) {
            return Err("The timeout default has not changed");
        }
        Ok(PoolOperation::SetHaTimeout {
            expected: a.ha_timeout_override.clone(),
            seconds,
        })
    })();
    match result {
        Ok(op) => stage(m, op),
        Err(error) => refuse(m, error),
    }
}
