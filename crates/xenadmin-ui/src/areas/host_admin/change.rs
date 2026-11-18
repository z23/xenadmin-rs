//! The logic of the Rune `host_admin.rs`, carried over: what a change is,
//! the operation it builds, its review, its staleness check, its submission.
//!
//! The order of the checks is the Rune order and matters:
//!
//! 1. [`operation`] validates the draft and builds the typed operation from
//!    the inventory **captured when the editor opened**. Unknown values
//!    refuse; they are never read as `false` or as empty.
//! 2. [`review`] asks the core whether that operation is available against
//!    the same captured inventory, and keeps the operation. What is kept is
//!    what is submitted: nothing is rebuilt from the draft afterwards.
//! 3. [`current`] compares the captured observation with a fresh one and
//!    asks the core again against the **live** inventory. A review that is
//!    no longer current can be read but not confirmed, and it is never
//!    rebuilt silently: the user closes the surface and starts again.
//! 4. [`submit`] checks `current` once more, then hands the reviewed
//!    operation and the captured observation to the core.
//!
//! The pool everything is read from is the pool of the object the command
//! was given, which is not always the scope session's (a row of the Search
//! tab can belong to another pool). Hence `m.inventory_for(pool)` and its
//! siblings throughout, and no use of `m.inventory`, `m.snapshot` or
//! `m.connection`.

use std::sync::Arc;

use xenadmin_core::{
    Error, HostOperation, HostPowerOnMode, Inventory, MemoryLimits, ObservedResource, OperationId,
    PoolId, ResourceCommand, ResourceOperation, ResourceTarget,
};

use crate::prelude::*;

/// One change of a server or, for [`Kind::Autostart`], of a pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Maintenance,
    Evacuate,
    Exit,
    Reboot,
    Shutdown,
    Restart,
    PowerOn,
    WakeOnLan,
    DisablePowerOn,
    Memory,
    Logging,
    Multipath,
    Autostart,
    Forget,
}

impl Kind {
    /// The Rune title: the title of the dialog, or the heading of the page.
    pub fn title(self) -> &'static str {
        match self {
            Self::Maintenance => "Enter maintenance",
            Self::Evacuate => "Evacuate guests",
            Self::Exit => "Exit maintenance",
            Self::Reboot => "Reboot host",
            Self::Shutdown => "Shut down host",
            Self::Restart => "Restart toolstack",
            Self::PowerOn => "Power on host",
            Self::WakeOnLan => "Configure Wake-on-LAN",
            Self::DisablePowerOn => "Disable remote power-on",
            Self::Memory => "Change dom0 memory",
            Self::Logging => "Remote logging",
            Self::Multipath => "Change multipathing",
            Self::Autostart => "Pool VM autostart",
            Self::Forget => "Forget dead host",
        }
    }

    /// Whether the change is outside the supported part. Entering and
    /// leaving maintenance, evacuation, a reboot, a shutdown and a toolstack
    /// restart are supported (docs/FEATURES.md); the other changes a server
    /// or pool offers here are not. This is the Rune `experimental`, and
    /// AREAS.md decision 1 makes it the rule of `Surface::experimental`.
    pub fn experimental(self) -> bool {
        !matches!(
            self,
            Self::Maintenance
                | Self::Evacuate
                | Self::Exit
                | Self::Reboot
                | Self::Shutdown
                | Self::Restart
        )
    }

    /// The page of the server's Properties dialog the change is edited on.
    /// `None`: the change has a dialog of its own.
    pub fn page(self) -> Option<Page> {
        match self {
            Self::Multipath => Some(Page::Multipathing),
            Self::Logging => Some(Page::LogDestination),
            Self::WakeOnLan | Self::DisablePowerOn => Some(Page::PowerOn),
            _ => None,
        }
    }

    /// Whether the user enters something before the review: a text or the
    /// acknowledgement.
    pub fn has_draft(self) -> bool {
        matches!(self, Self::Memory | Self::Logging | Self::Forget)
    }

    /// Whether the command of the change opens its dialog on the review.
    /// That is so when there is nothing to enter and nothing to choose: the
    /// dialog's first state would only repeat the review with another
    /// button. A change with a draft, and a page (which is looked at, and
    /// left for another page), open on the draft, and their review has a
    /// Back to it.
    pub fn opens_on_review(self) -> bool {
        !self.has_draft() && self.page().is_none()
    }
}

/// A page of the server's Properties dialog that this area provides, in the
/// plan's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Multipathing,
    LogDestination,
    PowerOn,
}

impl Page {
    pub const ALL: [Page; 3] = [Page::Multipathing, Page::LogDestination, Page::PowerOn];

    pub fn id(self) -> &'static str {
        match self {
            Self::Multipathing => "multipathing",
            Self::LogDestination => "log-destination",
            Self::PowerOn => "power-on",
        }
    }

    /// The plan's name of the page.
    pub fn label(self) -> &'static str {
        match self {
            Self::Multipathing => "Multipathing",
            Self::LogDestination => "Log Destination",
            Self::PowerOn => "Power On",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.id() == id)
    }

    /// A change the page edits (the Power On page has two; either says
    /// whether the page is outside the supported part).
    pub fn kind(self) -> Kind {
        match self {
            Self::Multipathing => Kind::Multipath,
            Self::LogDestination => Kind::Logging,
            Self::PowerOn => Kind::WakeOnLan,
        }
    }
}

/// The open change: the Rune `Workspace`, with a `String` for its
/// `TextEdit`.
#[derive(Debug)]
pub struct Editor {
    /// The inventory as it was when the editor opened. The draft is
    /// validated and the review is written against this, never against the
    /// live inventory.
    pub initial: Arc<Inventory>,
    /// The server, or the pool for [`Kind::Autostart`].
    pub target: ResourceTarget,
    /// The observation taken when the editor opened: what the submission
    /// names as its target. `None` without a current connection (always in
    /// the demo), and then nothing can be submitted.
    pub observed: Option<ObservedResource>,
    pub kind: Kind,
    /// The one text draft: megabytes for [`Kind::Memory`], the destination
    /// for [`Kind::Logging`].
    pub value: String,
    /// The acknowledgement of [`Kind::Forget`].
    pub acknowledged: bool,
    /// The reviewed operation. `Some` means the review is showing.
    pub review: Option<HostOperation>,
    pub error: Option<String>,
}

/// The refusal of every change whose data the inventory does not hold.
pub const MISSING: &str = "Required current host data is unavailable; refresh before editing";

/// Shown with a review that is no longer current.
pub const STALE: &str = "The connection, host or prerequisites changed. Close and reopen this \
    review before applying it.";

impl Editor {
    /// The editor a command would open for `object`, without opening it:
    /// the eligibility of the command asks it the same questions the open
    /// surface would. `None` when the object is not what the change applies
    /// to (a server; the pool for autostart) or its pool has no inventory.
    pub fn capture(m: &Model, object: &ObjectRef, kind: Kind) -> Option<Self> {
        let wanted = if kind == Kind::Autostart {
            ObjectKind::Pool
        } else {
            ObjectKind::Host
        };
        if object.kind != wanted {
            return None;
        }
        let pool = object.pool?;
        let target = resource_target(object)?;
        let initial = m.inventory_for(pool)?;
        let observed = m
            .snapshot_for(pool)
            .and_then(|snap| snap.observe_resource(target).ok());
        let mut editor = Self {
            initial,
            target,
            observed,
            kind,
            value: String::new(),
            acknowledged: false,
            review: None,
            error: None,
        };
        editor.select(kind);
        Some(editor)
    }

    /// Turns the editor to another change of the same target: the review,
    /// the error and the acknowledgement are dropped and the text draft is
    /// filled from the captured inventory. (The Rune `select`, which the
    /// chooser called; now the page list of the Properties dialog does.)
    pub fn select(&mut self, kind: Kind) {
        self.kind = kind;
        self.review = None;
        self.error = None;
        self.acknowledged = false;
        self.value = if let ResourceTarget::Host(id) = self.target {
            let host = self.initial.details.hosts.get(&id);
            match kind {
                Kind::Logging => host
                    .and_then(|h| h.syslog_destination.clone())
                    .unwrap_or_default(),
                Kind::Memory => host
                    .and_then(|h| h.control_domain)
                    .and_then(|id| self.initial.details.vms.get(&id))
                    .and_then(|v| v.memory_static_max)
                    .map(|n| (n / (1024 * 1024)).to_string())
                    .unwrap_or_default(),
                _ => String::new(),
            }
        } else {
            String::new()
        };
    }

    /// The pool the editor reads from and submits to.
    pub fn pool(&self) -> PoolId {
        self.initial.pool_id
    }

    /// The name of the server or pool, as captured.
    pub fn label(&self) -> &str {
        match self.target {
            ResourceTarget::Host(id) => self
                .initial
                .hosts
                .get(&id)
                .map(|h| h.name.as_str())
                .unwrap_or("Unknown host"),
            _ => &self.initial.pool_name,
        }
    }

    /// The change the Power On page starts on: the one mode the server is
    /// not in, since the other would change nothing.
    pub fn power_on_kind(&self) -> Kind {
        let ResourceTarget::Host(id) = self.target else {
            return Kind::WakeOnLan;
        };
        let mode = self
            .initial
            .details
            .hosts
            .get(&id)
            .and_then(|h| h.power_on_mode.as_deref());
        if mode == Some(HostPowerOnMode::WakeOnLan.as_str()) {
            Kind::DisablePowerOn
        } else {
            Kind::WakeOnLan
        }
    }
}

/// The typed operation of the draft, from the captured inventory.
pub fn operation(w: &Editor) -> Result<HostOperation, String> {
    let missing = || MISSING.to_string();
    if w.kind == Kind::Autostart {
        let expected = w
            .initial
            .details
            .pool
            .as_ref()
            .and_then(|p| p.auto_poweron)
            .ok_or_else(missing)?;
        return Ok(HostOperation::SetPoolAutostart {
            expected,
            enabled: !expected,
        });
    }
    let ResourceTarget::Host(id) = w.target else {
        return Err(missing());
    };
    let host = w.initial.hosts.get(&id).ok_or_else(missing)?;
    let d = w.initial.details.hosts.get(&id).ok_or_else(missing)?;
    Ok(match w.kind {
        Kind::Maintenance => HostOperation::EnterMaintenance {
            expected_enabled: host.enabled,
            resident_vms: w.initial.host_resident_guests(id),
        },
        Kind::Evacuate => HostOperation::Evacuate {
            resident_vms: w.initial.host_resident_guests(id),
        },
        Kind::Exit => HostOperation::ExitMaintenance,
        Kind::Reboot => HostOperation::Reboot {
            expected_boot_time: d.boot_time.clone().ok_or_else(missing)?,
        },
        Kind::Shutdown => HostOperation::Shutdown {
            expected_boot_time: d.boot_time.clone().ok_or_else(missing)?,
        },
        Kind::Restart => HostOperation::RestartToolstack {
            expected_agent_start_time: d.agent_start_time.clone().ok_or_else(missing)?,
        },
        Kind::PowerOn => HostOperation::PowerOn,
        Kind::WakeOnLan | Kind::DisablePowerOn => HostOperation::ConfigurePowerOn {
            expected_mode: d.power_on_mode.clone().ok_or_else(missing)?,
            mode: if w.kind == Kind::WakeOnLan {
                HostPowerOnMode::WakeOnLan
            } else {
                HostPowerOnMode::Disabled
            },
        },
        Kind::Memory => {
            let control_domain = d.control_domain.ok_or_else(missing)?;
            let vm = w
                .initial
                .details
                .vms
                .get(&control_domain)
                .ok_or_else(missing)?;
            let memory = w
                .value
                .parse::<u64>()
                .ok()
                .and_then(|n| n.checked_mul(1024 * 1024))
                .filter(|n| *n > 0 && *n <= i64::MAX as u64)
                .ok_or("Enter a positive whole MiB memory value")?;
            let total = d
                .metrics
                .and_then(|id| w.initial.details.host_metrics.get(&id))
                .and_then(|m| m.memory_total)
                .ok_or_else(missing)?;
            if memory > total {
                return Err("Memory exceeds the host's physical memory".into());
            }
            HostOperation::SetDom0Memory {
                control_domain,
                expected: MemoryLimits {
                    static_min: vm.memory_static_min.ok_or_else(missing)?,
                    dynamic_min: vm.memory_dynamic_min.ok_or_else(missing)?,
                    dynamic_max: vm.memory_dynamic_max.ok_or_else(missing)?,
                    static_max: vm.memory_static_max.ok_or_else(missing)?,
                },
                memory,
            }
        }
        Kind::Logging => {
            if !d.syslog_known {
                return Err(missing());
            }
            HostOperation::SetSyslog {
                expected_destination: d.syslog_destination.clone(),
                destination: (!w.value.trim().is_empty()).then(|| w.value.trim().into()),
            }
        }
        Kind::Multipath => {
            let expected = d.multipathing.ok_or_else(missing)?;
            HostOperation::SetMultipath {
                expected,
                enabled: !expected,
            }
        }
        Kind::Forget => HostOperation::ForgetDeadHost {
            verified_power_off: w.acknowledged,
        },
        // Handled above: autostart is the one change of a pool.
        Kind::Autostart => return Err("Choose a host operation".into()),
    })
}

/// The operation of the draft when the core finds it available against the
/// captured inventory, else the reason it cannot be reviewed.
fn reviewable(w: &Editor) -> Result<HostOperation, String> {
    operation(w).and_then(|op| {
        let a = op.availability(&w.initial, w.target);
        if a.is_available() {
            Ok(op)
        } else {
            Err(a.explanation())
        }
    })
}

/// Why the change cannot be reviewed **whatever the user enters**, if that
/// is so. A command is disabled with this reason, so a dialog never opens
/// on a change that cannot go ahead.
///
/// - A change without a draft is fully known now: this is the refusal its
///   review would give.
/// - Forgetting a server waits only for the acknowledgement: this is what
///   the core says once it is given.
/// - Memory and the log destination depend on the text: only missing host
///   data is certain before it is entered.
pub fn blocked(w: &Editor) -> Option<String> {
    match w.kind {
        Kind::Memory | Kind::Logging => operation(w).err().filter(|reason| reason == MISSING),
        Kind::Forget => operation(w).err().or_else(|| {
            let a = HostOperation::ForgetDeadHost {
                verified_power_off: true,
            }
            .availability(&w.initial, w.target);
            (!a.is_available()).then(|| a.explanation())
        }),
        _ => reviewable(w).err(),
    }
}

/// The Review button: the draft becomes the reviewed operation, or the
/// editor shows why it cannot.
pub fn review(m: &mut Model) {
    let Some(w) = &mut m.areas.host_admin.editor else {
        return;
    };
    match reviewable(w) {
        Ok(op) => {
            w.review = Some(op);
            w.error = None;
        }
        Err(e) => w.error = Some(e),
    }
}

/// The Back button of a review: to the draft, which is kept.
pub fn back(m: &mut Model) {
    if let Some(w) = &mut m.areas.host_admin.editor {
        w.review = None;
        w.error = None;
    }
}

/// Turns the open editor to another change of its target (the two modes
/// of the Power On page).
pub fn select(m: &mut Model, kind: Kind) {
    if let Some(w) = &mut m.areas.host_admin.editor {
        w.select(kind);
    }
}

/// Whether the editor's pool is a synthetic one, in which nothing can be
/// submitted. For the scope session this is `m.demo`.
pub fn synthetic(m: &Model, w: &Editor) -> bool {
    m.is_synthetic_pool(w.pool())
}

/// The Properties dialog tells the area about its pages. Entering a page
/// creates the editor of that page's change afresh, as choosing the change
/// in the Rune chooser did; leaving it drops that editor, as closing the
/// Rune dialog did. `Closed` has nothing left to drop: the editor belongs
/// to one page.
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) => {
            let Some(page) = Page::from_id(id) else {
                return;
            };
            let editor = target
                .single()
                .and_then(|object| Editor::capture(m, object, Kind::Multipath))
                .map(|mut w| {
                    let kind = match page {
                        Page::Multipathing => Kind::Multipath,
                        Page::LogDestination => Kind::Logging,
                        Page::PowerOn => w.power_on_kind(),
                    };
                    w.select(kind);
                    w
                });
            m.areas.host_admin.editor = editor;
        }
        PageEvent::Left(id) => {
            let editor = &mut m.areas.host_admin.editor;
            if editor
                .as_ref()
                .is_some_and(|w| w.kind.page().is_some_and(|page| page.id() == id))
            {
                *editor = None;
            }
        }
        PageEvent::Closed => {}
    }
}

/// Whether the review may be submitted now: a writable, current connection;
/// the captured observation still the one a fresh look gives (a reconnect
/// changes it); and the reviewed operation still available against the live
/// inventory.
pub fn current(m: &Model, w: &Editor) -> bool {
    let pool = w.pool();
    m.can_mutate_pool(pool)
        && w.observed.is_some()
        && w.observed
            == m.snapshot_for(pool)
                .and_then(|snapshot| snapshot.observe_resource(w.target).ok())
        && w.review.as_ref().is_some_and(|op| {
            m.inventory_for(pool)
                .is_some_and(|i| op.availability(&i, w.target).is_available())
        })
}

/// Why the review showing cannot be confirmed, when it is no longer
/// current. In a synthetic pool the preview notice says it instead.
pub fn stale(m: &Model, w: &Editor) -> Option<String> {
    (w.review.is_some() && !synthetic(m, w) && !current(m, w)).then(|| STALE.to_owned())
}

/// The Confirm button. On success the handle goes to the model and the
/// surface closes: the outcome is followed in Events, not here. On a
/// refusal the surface stays, with the reason and the review.
pub fn submit(m: &mut Model) {
    let Some(w) = &m.areas.host_admin.editor else {
        return;
    };
    match submission(m, w) {
        Ok(operation) => {
            m.track_resource(operation);
            super::close(m);
        }
        Err(e) => {
            if let Some(w) = &mut m.areas.host_admin.editor {
                w.error = Some(e);
            }
        }
    }
}

/// Checks again everything the Confirm button showed, since a handler can
/// run after the state has moved on, then submits what was reviewed.
fn submission(m: &Model, w: &Editor) -> Result<ResourceOperation, String> {
    let stale = || Error::Stale.to_string();
    if !current(m, w) {
        return Err(stale());
    }
    let operation = w.review.clone().ok_or_else(stale)?;
    // Rune disabled the button on a known denial and did not look again
    // here; the port does (MODEL-API 21). An unknown permission never
    // blocks: the host decides.
    let permission = permissions::assess_for(m, w.pool(), &operation.permission_requirements());
    if let Some(denial) = permissions::denial(&permission) {
        return Err(denial);
    }
    m.connection_for(w.pool())
        .ok_or_else(|| Error::Closed.to_string())?
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Host {
                target: w.observed.ok_or_else(stale)?,
                operation,
            },
        )
        .map_err(|e| e.to_string())
}

/// The closing line of every review.
// "Stop remaining steps" is the control of the record on the Events tab of
// the Notifications view, which also took over what the Rune text sent the
// reader to "Activity" and "Pool administration" for (decision 10).
const CLOSING: &str = "Changes are journaled individually. Stop remaining steps does not cancel \
    in-flight host work or undo completed changes. Use the Events tab of the \
    Notifications view for retained history and for unknown-outcome recovery.";

/// The review text of an operation, word for word from Rune: the pool, the
/// target, what the operation does and does not do, and the closing line.
pub fn lines(w: &Editor, op: &HostOperation) -> Vec<String> {
    let mut lines = vec![
        format!("Pool {}", w.initial.pool_id),
        format!("Target {}", w.target.uuid()),
    ];
    match op {
        HostOperation::DataSource { action, .. } => lines.push(action.label().into()),
        HostOperation::EnterMaintenance { resident_vms, .. }
        | HostOperation::Evacuate { resident_vms } => {
            lines.push(
                "Scheduling will be disabled for maintenance. The host selects compatible \
                 migration destinations after checking every reviewed guest. No guest shutdown, \
                 HA reduction or coordinator handover is automatic. A failure may leave \
                 scheduling disabled or some guests already moved."
                    .into(),
            );
            for id in resident_vms {
                lines.push(format!(
                    "Move guest: {} ({id})",
                    w.initial
                        .vms
                        .get(id)
                        .map(|v| v.name.as_str())
                        .unwrap_or("Unknown guest")
                ));
            }
        }
        HostOperation::ExitMaintenance => lines.push(
            "Enable scheduling, then clear the maintenance marker. No evacuated guest is \
             automatically moved back."
                .into(),
        ),
        HostOperation::Reboot { .. } | HostOperation::Shutdown { .. } => lines.push(
            "The host must be disabled and empty of running or paused guests. Multi-host pool \
             coordinators require separate handover. Connection loss leaves an uncertain outcome \
             until independent task and host evidence can establish it."
                .into(),
        ),
        HostOperation::RestartToolstack { .. } => lines.push(
            "Restart XAPI only with pool HA disabled and after checking that pool tasks have \
             finished. Management and console connections may disconnect; guests keep running. \
             A newer toolstack start marker is required to verify completion."
                .into(),
        ),
        HostOperation::PowerOn => lines.push(
            "Use the host's configured remote power-on method. Success requires a fresh report \
             that the host is live."
                .into(),
        ),
        HostOperation::ConfigurePowerOn {
            expected_mode,
            mode,
        } => lines.push(format!(
            "Power-on mode: {expected_mode:?} → {mode:?}. This editor accepts only disabled or \
             Wake-on-LAN modes with no stored BMC configuration."
        )),
        HostOperation::SetDom0Memory {
            control_domain,
            expected,
            memory,
        } => lines.push(format!(
            "Control domain {control_domain}: static maximum {} MiB → {} MiB; dynamic minimum \
             and maximum also become {} MiB. Static minimum remains {} MiB. The host decides \
             whether the requested allocation is possible.",
            expected.static_max / 1048576,
            memory / 1048576,
            memory / 1048576,
            expected.static_min / 1048576
        )),
        HostOperation::SetSyslog {
            expected_destination,
            destination,
        } => lines.push(format!(
            "Remote logging: {} → {}. Save the destination, then request logging \
             reconfiguration; failure of the second step leaves the saved setting visible as a \
             partial outcome.",
            expected_destination.as_deref().unwrap_or("Local only"),
            destination.as_deref().unwrap_or("Local only")
        )),
        HostOperation::SetMultipath { expected, enabled } => lines.push(format!(
            "Multipathing: {expected} → {enabled}. Requires an evacuated maintenance host with \
             every storage connection detached. Apply the standard dmp handler. Storage is not \
             automatically detached or reconnected."
        )),
        HostOperation::SetPoolAutostart { expected, enabled } => lines.push(format!(
            "Pool-wide VM autostart: {expected} → {enabled}. This setting applies to every host \
             in the pool, together with each VM's own auto-start flag. It does not start VMs now \
             or alter per-VM flags."
        )),
        HostOperation::ForgetDeadHost { .. } => lines.push(
            "Remove this dead host's pool record only. You have independently verified it is \
             powered off. It must be offline, disabled, empty, not the coordinator and HA must \
             be disabled. Storage repository records and data are retained; rejoining requires \
             separate recovery."
                .into(),
        ),
    }
    lines.push(CLOSING.into());
    lines
}
