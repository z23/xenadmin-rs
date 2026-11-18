//! The life of the draft: captured when the first of this area's pages is
//! entered, kept while the Properties dialog is open, reviewed on the page
//! Review Changes, submitted as ONE `VmCommand::Configure`, dropped when the
//! dialog closes.
//!
//! The Rune functions `open`, `refresh_boot` and `submit` are here with what
//! the port changed: the target is the object the dialog is about (not the
//! table's cursor), every read goes to the pool of that object, and each
//! check a Rune button made by being disabled is made again by the handler.

use xenadmin_core::{ClassAvailability, Error, VmCommand, VmSettingsPatch};

use super::draft::{Editor, memory_text};
use crate::prelude::*;

/// The pages of the area, in the order of the page list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    CpuMemory,
    BootOptions,
    StartOptions,
    HomeServer,
    Advanced,
    /// Not in the plan's table: the review of the one change set, after the
    /// six pages that edit it.
    Review,
}

impl Page {
    pub const ALL: [Self; 7] = [
        Self::General,
        Self::CpuMemory,
        Self::BootOptions,
        Self::StartOptions,
        Self::HomeServer,
        Self::Advanced,
        Self::Review,
    ];

    /// The page id: the plan's (`PropertiesKind::Vm.planned()`), and
    /// `review`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::CpuMemory => "cpu-memory",
            Self::BootOptions => "boot-options",
            Self::StartOptions => "start-options",
            Self::HomeServer => "home-server",
            Self::Advanced => "advanced",
            Self::Review => "review",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.id() == id)
    }
}

/// The label of the review page in the page list.
pub const REVIEW_LABEL: &str = "Review Changes";
/// The place of the review page: last, where the kernel puts every page the
/// plan does not list (after Advanced, which is 80).
pub const REVIEW_ORDER: u16 = PropertyPage::UNPLANNED;

/// Rune caption of an editor whose target is not the one it captured.
pub const NOT_CURRENT: &str = "Connection or target changed; reopen properties to refresh.";
/// Rune status line of the same condition; here the stale line of the review
/// and the reason of a disabled Apply.
pub const STALE: &str = "The connection changed. Close this editor and reopen properties to refresh the target and values.";
/// The reason Rune's buttons were disabled without saying so: not the demo,
/// and no writable current connection. The words every area uses for it.
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";
/// From the Rune caption of a template's properties.
pub const BUILT_IN: &str = "Built-in templates are read-only; copy one first.";
/// The reason a synthetic pool gives a disabled confirm button.
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The one VM or template a target names, when the inventory of its pool
/// has it.
pub fn known<'a>(m: &Model, target: &'a Target) -> Option<&'a ObjectRef> {
    let object = target
        .single_of(ObjectKind::Vm)
        .or_else(|| target.single_of(ObjectKind::Template))?;
    let inventory = m.inventory_for(object.pool?)?;
    inventory
        .vms
        .contains_key(&object.vm_id()?)
        .then_some(object)
}

/// The Rune `open`: the draft of one VM or template, seeded from the
/// inventory of ITS pool. `target` is `None` in a synthetic pool. The boot
/// readiness query is issued here, as Rune issued it on opening.
pub fn capture(m: &Model, object: &ObjectRef) -> Option<Editor> {
    let pool = object.pool?;
    let id = object.vm_id()?;
    let inventory = m.inventory_for(pool)?;
    let original = inventory.vms.get(&id).cloned()?;
    let details = inventory.details.vms.get(&id).cloned().unwrap_or_default();
    let target = m
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(id).ok());
    let boot_query = target.and_then(|o| m.connection_for(pool)?.guest_boot_status(o).ok());
    if let (Some(query), Some(waker)) = (&boot_query, &m.waker) {
        query.notify_on_change(waker.clone());
    }
    Some(Editor {
        pool,
        boot_seen: false,
        back: Page::General.id(),
        target,
        id,
        name: original.name.clone(),
        description: original.description.clone(),
        max_cpus: details.vcpus_max.map(|v| v.to_string()).unwrap_or_default(),
        startup_cpus: details
            .vcpus_at_startup
            .map(|v| v.to_string())
            .unwrap_or_default(),
        static_min: memory_text(details.memory_static_min),
        dynamic_min: memory_text(details.memory_dynamic_min),
        dynamic_max: memory_text(details.memory_dynamic_max),
        static_max: memory_text(details.memory_static_max),
        boot_order: details.boot_order.clone().unwrap_or_default(),
        topology: details.cores_per_socket.clone().unwrap_or_default(),
        cpu_weight: details.cpu_weight.clone().unwrap_or_else(|| "256".into()),
        shadow: details.shadow_multiplier.clone().unwrap_or_default(),
        autostart: details.auto_power_on.unwrap_or(false),
        secure_boot: details.secure_boot.unwrap_or(false),
        firmware: details.firmware.clone(),
        empty_template: original.is_template
            && details.is_default_template == Some(false)
            && inventory.details.classes.get("vbd") == Some(&ClassAvailability::Available)
            && !inventory.details.vbds.values().any(|v| v.vm == Some(id)),
        boot_query,
        boot_error: None,
        home_host: details.affinity,
        hosts: {
            let mut hosts: Vec<_> = inventory
                .hosts
                .values()
                .map(|host| (host.id, host.name.clone()))
                .collect();
            hosts.sort_by_cached_key(|(_, name)| name.to_lowercase());
            hosts
        },
        original,
        details,
        review: None,
        error: None,
    })
}

/// What the dialog tells this area. One draft spans the pages: it is
/// captured when the first of them is entered and dropped only when the
/// dialog closes, so what was typed on one page is still there on the next
/// and all of it is one change set. Entering Review Changes builds the
/// review; leaving it is the Rune "Back".
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) => {
            let Some(page) = Page::from_id(id) else {
                return;
            };
            ensure(m, target);
            if page == Page::Review {
                build_review(m);
            } else if let Some(w) = &mut m.areas.vm_properties.editor {
                w.review = None;
                w.back = id;
            }
        }
        PageEvent::Left(id) => {
            if id == Page::Review.id()
                && let Some(w) = &mut m.areas.vm_properties.editor
            {
                w.review = None;
            }
        }
        PageEvent::Closed => m.areas.vm_properties.editor = None,
    }
}

/// The draft of this target, captured unless it is already there.
fn ensure(m: &mut Model, target: &Target) {
    let Some(object) = known(m, target).cloned() else {
        m.areas.vm_properties.editor = None;
        return;
    };
    let kept = m
        .areas
        .vm_properties
        .editor
        .as_ref()
        .is_some_and(|w| Some(w.pool) == object.pool && Some(w.id) == object.vm_id());
    if !kept {
        m.areas.vm_properties.editor = capture(m, &object);
    }
}

/// Whether the editor's pool is a synthetic one (the Rune `s.demo`).
pub fn synthetic(m: &Model, w: &Editor) -> bool {
    m.is_synthetic_pool(w.pool)
}

fn fresh(m: &Model, w: &Editor) -> Option<xenadmin_core::ObservedVm> {
    m.snapshot_for(w.pool)
        .and_then(|snapshot| snapshot.observe_vm(w.id).ok())
}

/// The Rune `current`: the demo, or the target captured at opening is still
/// what the pool's snapshot observes.
pub fn current(m: &Model, w: &Editor) -> bool {
    synthetic(m, w) || w.target.is_some() && w.target == fresh(m, w)
}

/// Why no review can be asked for, whatever the draft holds: the conditions
/// that disabled the Rune button "Review changes".
pub fn review_blocked(m: &Model, w: &Editor) -> Option<&'static str> {
    if !current(m, w) {
        Some(NOT_CURRENT)
    } else if !synthetic(m, w) && !m.can_mutate_pool(w.pool) {
        Some(NOT_WRITABLE)
    } else if w.original.is_template && w.details.is_default_template != Some(false) {
        Some(BUILT_IN)
    } else {
        None
    }
}

/// The review the draft would get now: refused for what blocks every review,
/// else the patch or the reason there is none.
fn reviewed(m: &Model, w: &Editor) -> Result<VmSettingsPatch, String> {
    match review_blocked(m, w) {
        Some(reason) => Err(reason.into()),
        None => w.patch(),
    }
}

/// The button "Review changes" of the six pages: a draft that cannot be
/// reviewed keeps its page and says why (as in Rune); one that can turns the
/// dialog to the page Review Changes, which builds the review on entering.
pub fn review(m: &mut Model) {
    let Some(result) = (m.areas.vm_properties.editor.as_ref()).map(|w| reviewed(m, w)) else {
        return;
    };
    match result {
        Ok(_) => {
            m.show_properties_page(Page::Review.id());
        }
        Err(error) => {
            if let Some(w) = &mut m.areas.vm_properties.editor {
                w.error = Some(error);
            }
        }
    }
}

/// On entering Review Changes, by the button or from the page list.
fn build_review(m: &mut Model) {
    let Some(result) = (m.areas.vm_properties.editor.as_ref()).map(|w| reviewed(m, w)) else {
        return;
    };
    let Some(w) = &mut m.areas.vm_properties.editor else {
        return;
    };
    match result {
        Ok(patch) => {
            w.review = Some(patch);
            w.error = None;
        }
        Err(error) => {
            w.review = None;
            w.error = Some(error);
        }
    }
}

/// "Back" on the review: to the page the review was asked from.
pub fn back(m: &mut Model) {
    let Some(page) = m.areas.vm_properties.editor.as_ref().map(|w| w.back) else {
        return;
    };
    m.show_properties_page(page);
}

/// "Cancel", Escape: the dialog closes and takes the draft with it
/// ([`page_event`], `Closed`).
pub fn cancel(m: &mut Model) {
    m.close_properties();
}

/// Why the review cannot be applied although it exists: the target is not
/// the observed one any more (never in a synthetic pool, which has no
/// observation and refuses by [`DEMO`] instead).
pub fn stale(m: &Model, w: &Editor) -> Option<&'static str> {
    (!synthetic(m, w) && (w.target.is_none() || w.target != fresh(m, w))).then_some(STALE)
}

/// What disables "Apply changes" besides a known permission denial, first
/// reason first.
pub fn apply_blocked(m: &Model, w: &Editor) -> Option<&'static str> {
    stale(m, w)
        .or_else(|| (!synthetic(m, w) && !m.can_mutate_pool(w.pool)).then_some(NOT_WRITABLE))
}

/// The Rune `submit`: the reviewed patch for the captured target, as one
/// `VmCommand::Configure` through the connection of the target's pool. Every
/// condition of the button is checked again. Accepted: the dialog closes and
/// the operation is followed in Events. Refused: the reason is on the page
/// and nothing was sent.
pub fn submit(m: &mut Model) {
    let Some(w) = &m.areas.vm_properties.editor else {
        return;
    };
    let Some(patch) = w.review.clone() else {
        return;
    };
    let pool = w.pool;
    let refusal = apply_blocked(m, w)
        .map(str::to_owned)
        .or_else(|| {
            let assessment = permissions::assess_for(m, pool, &patch.permission_requirements());
            permissions::denial(&assessment)
        })
        .or_else(|| synthetic(m, w).then(|| Error::Stale.to_string()));
    let result = match (refusal, w.target) {
        (Some(reason), _) => Err(reason),
        (None, None) => return,
        (None, Some(vm)) => {
            crate::areas::power::track_vm(m, pool, VmCommand::Configure { vm, patch })
                .map_err(|error| error.to_string())
        }
    };
    match result {
        Ok(_) => {
            m.error = None;
            m.close_properties();
            // A draft that was open without its dialog goes too.
            m.areas.vm_properties.editor = None;
        }
        Err(error) => {
            if let Some(w) = &mut m.areas.vm_properties.editor {
                w.error = Some(error);
            }
        }
    }
}

/// The Rune `refresh_boot`: asks the host again for the certificate
/// readiness of the captured target. A read; nothing is changed on the host.
pub fn refresh_boot(m: &mut Model) {
    let Some(w) = &m.areas.vm_properties.editor else {
        return;
    };
    let Some(target) = w.target else {
        return;
    };
    let result = m
        .connection_for(w.pool)
        .ok_or(Error::Closed)
        .and_then(|c| c.guest_boot_status(target));
    if let (Ok(query), Some(waker)) = (&result, &m.waker) {
        query.notify_on_change(waker.clone());
    }
    let Some(w) = &mut m.areas.vm_properties.editor else {
        return;
    };
    w.boot_seen = false;
    match result {
        Ok(query) => {
            w.boot_query = Some(query);
            w.boot_error = None;
        }
        Err(error) => {
            w.boot_query = None;
            w.boot_error = Some(error.to_string());
        }
    }
}

/// Rune read the readiness answer while drawing and the waker only caused a
/// redraw. Here the description reads it too; `poll` says when it arrived,
/// so the front end describes again.
pub fn poll(m: &mut Model, _clock: f64) -> bool {
    let Some(w) = &mut m.areas.vm_properties.editor else {
        return false;
    };
    let seen = w.boot_query.as_ref().is_some_and(|q| q.latest().is_some());
    let changed = seen != w.boot_seen;
    w.boot_seen = seen;
    changed
}

/// A scope change or a session change: the kernel closes the dialog, which
/// drops the draft; this drops one that was left without a dialog.
pub fn drop_draft(m: &mut Model) {
    m.areas.vm_properties.editor = None;
}
