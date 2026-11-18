//! Bulk properties: one property set to one value on up to 32 VMs or
//! custom templates of one pool, reviewed and submitted as one
//! `ResourceCommand::PropertiesBatch`.
//!
//! Carried over from the Rune `bulk_properties.rs`. The selection is the
//! target of the Properties dialog instead of the selected rows of the
//! Guests or Templates table; the pool is the target's pool instead of the
//! scope session's. The editor is the draft of the page
//! [`PropertyPage::BULK`]: it lives while that page is shown.

use xenadmin_core::{
    BatchVmProperties, Error, HostId, ObservedResource, OperationId, PoolId, PowerState,
    ResourceCommand, ResourceTarget, Vm, VmId, VmPropertiesBatch,
};

use super::property::{self, Property};
use crate::prelude::*;

pub const NO_INVENTORY: &str = "Inventory is unavailable";
pub const LIMIT: &str = "Select between 1 and 32 visible targets";
pub const MISSING: &str = "A selected row is no longer present in VM inventory";
pub const TYPE_CHANGED: &str = "A selected row changed type; reopen the selection";
/// A selection across pools: the batch is one command to one pool. (New
/// text: the Rune table showed one pool at a time.)
pub const ONE_POOL: &str = "Select targets of one pool to edit them together";
pub const STALE: &str = "Connection changed or inventory is stale. Reopen this selection.";
pub const NOTHING_TO_CHANGE: &str =
    "All selected targets already have the desired value. No changes to submit.";
pub const REVIEW_CHANGED: &str =
    "Reviewed values changed. Nothing will be sent; go Back or reopen the selection.";
/// Decision 10 of `AREAS.md`: "Operation history" is renamed in the one
/// sweep at the end, not here.
pub const SEQUENCE: &str = "Targets and setters run serially. A failure or uncertain response stops the remainder; completed changes remain. There is no automatic rollback. Operation history retains per-target steps, and recheck never sends pending changes.";
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale. Review is available; submission is disabled.";
/// Why "Apply changes" is disabled in a synthetic pool. (New text: Rune
/// disabled the button without saying why.)
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

pub struct Editor {
    pub pool: PoolId,
    pub pool_name: String,
    /// The pool as observed when the selection was captured: the token the
    /// batch is submitted with.
    pub observed: Option<ObservedResource>,
    /// The targets as they were when captured, in the order of the
    /// selection.
    pub targets: Vec<Vm>,
    /// The hosts of the pool, by name: the choices of a home host.
    pub hosts: Vec<(HostId, String)>,
    pub property: Property,
    pub values: [String; 4],
    pub host: Option<HostId>,
    pub toggle: bool,
    pub review: Option<Review>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    /// Only the targets that change.
    pub batch: VmPropertiesBatch,
    /// Every captured target.
    pub rows: Vec<ReviewRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRow {
    pub id: VmId,
    pub name: String,
    pub state: PowerState,
    pub changed: bool,
    pub summary: String,
}

fn edit(m: &mut Model) -> Option<&mut Editor> {
    m.areas.bulk_edit.properties.as_mut()
}

/// The VMs, or the custom templates, of a target in its pool's inventory,
/// in the target's order (the Rune `selected`), or why the selection is
/// refused. A target of templates takes the place of the Templates page.
pub(crate) fn selected(m: &Model, target: &Target) -> Result<(PoolId, Vec<Vm>), String> {
    let templates = target.all_of(ObjectKind::Template);
    let pool = target.pool().ok_or(ONE_POOL)?;
    let inventory = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    if !(1..=32).contains(&target.len()) {
        return Err(LIMIT.into());
    }
    let vms = target
        .objects
        .iter()
        .map(|object| {
            let vm = object
                .vm_id()
                .and_then(|id| inventory.vms.get(&id))
                .ok_or(MISSING)?;
            if vm.is_control_domain || vm.is_template != templates {
                return Err(TYPE_CHANGED.to_owned());
            }
            Ok(vm.clone())
        })
        .collect::<Result<_, String>>()?;
    Ok((pool, vms))
}

/// The editor a target would open (the Rune `open`): the description is
/// the property chosen first.
pub(crate) fn capture(m: &Model, target: &Target) -> Result<Editor, String> {
    let (pool, targets) = selected(m, target)?;
    let i = m.inventory_for(pool).ok_or(NO_INVENTORY)?;
    let mut hosts = i
        .hosts
        .values()
        .map(|h| (h.id, h.name.clone()))
        .collect::<Vec<_>>();
    hosts.sort_by_cached_key(|(_, name)| name.to_lowercase());
    let mut editor = Editor {
        pool,
        pool_name: i.pool_name.clone(),
        observed: m.snapshot_for(pool).and_then(|snapshot| {
            snapshot
                .observe_resource(ResourceTarget::Pool(pool))
                .ok()
        }),
        targets,
        hosts,
        property: Property::Description,
        values: Default::default(),
        host: None,
        toggle: false,
        review: None,
        error: None,
    };
    set_property(&mut editor, Property::Description);
    Ok(editor)
}

/// The draft of the bulk page follows the page: created when it is
/// entered, dropped when it is left or the dialog closes. A selection that
/// is refused has no draft; the page says why ([`selected`]).
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) if id == PropertyPage::BULK.id => {
            m.areas.bulk_edit.properties = capture(m, target).ok();
        }
        PageEvent::Left(id) if id == PropertyPage::BULK.id => {
            m.areas.bulk_edit.properties = None;
        }
        PageEvent::Closed => m.areas.bulk_edit.properties = None,
        _ => {}
    }
}

fn set_property(e: &mut Editor, property: Property) {
    e.property = property;
    e.error = None;
    e.values = Default::default();
    // A shared value is entered explicitly; do not silently use the first target's value.
    let defaults: &[&str] = match property {
        Property::Cpus => &["2", "2"],
        Property::LiveCpus => &["2"],
        Property::Memory => &["512 MiB", "1 GiB", "2 GiB", "2 GiB"],
        Property::DynamicMemory => &["1 GiB", "2 GiB"],
        Property::Topology => &["1"],
        Property::Weight => &["256"],
        Property::Shadow => &["1"],
        Property::BootOrder => &["cdn"],
        _ => &[],
    };
    for (field, value) in e.values.iter_mut().zip(defaults) {
        *field = (*value).to_owned();
    }
}

/// Another property: ignored while a review exists. The values start from
/// the defaults of the property; the chosen host and the toggle stay.
pub(crate) fn choose(m: &mut Model, property: Property) {
    if let Some(e) = edit(m).filter(|e| e.review.is_none()) {
        set_property(e, property);
    }
}

pub(crate) fn set_value(m: &mut Model, index: usize, text: &str) {
    if let Some(e) = edit(m).filter(|e| e.review.is_none())
        && let Some(value) = e.values.get_mut(index)
    {
        *value = text.to_owned();
        e.error = None;
    }
}

pub(crate) fn set_host(m: &mut Model, host: Option<HostId>) {
    if let Some(e) = edit(m).filter(|e| e.review.is_none()) {
        e.host = host;
    }
}

pub(crate) fn set_toggle(m: &mut Model, value: bool) {
    if let Some(e) = edit(m).filter(|e| e.review.is_none()) {
        e.toggle = value;
    }
}

/// The pool is still the one captured: any inventory of it in a synthetic
/// pool, else the observation the selection was captured with. Unlike the
/// other batches this does not ask for a writable connection: the review
/// is available on a read-only one, the submission is not.
pub(crate) fn current(m: &Model, e: &Editor) -> bool {
    if m.inventory_for(e.pool).is_none() {
        return false;
    }
    m.is_synthetic_pool(e.pool)
        || e.observed.is_some()
            && e.observed
                == m.snapshot_for(e.pool)
                    .and_then(|snap| snap.observe_resource(ResourceTarget::Pool(e.pool)).ok())
}

/// The batch that would be reviewed now: a row for every captured target,
/// a change for each whose value differs. One target that is gone, changed
/// or cannot take the property refuses the whole batch.
pub(crate) fn proposed(m: &Model, e: &Editor) -> Result<Review, String> {
    if !current(m, e) {
        return Err(STALE.into());
    }
    let i = m.inventory_for(e.pool).ok_or(NO_INVENTORY)?;
    let i = i.as_ref();
    let mut rows = Vec::new();
    let mut vms = Vec::new();
    let mut errors = Vec::new();
    for captured in &e.targets {
        let result = (|| {
            let now = i
                .vms
                .get(&captured.id)
                .ok_or("Target disappeared from inventory")?;
            if now.power_state != captured.power_state
                || now.is_template != captured.is_template
                || now.name != captured.name
                || now.is_control_domain
            {
                return Err(
                    "Target name, type or power state changed; reopen this selection".into(),
                );
            }
            let (patch, summary) = property::proposal(i, now, e)?;
            rows.push(ReviewRow {
                id: now.id,
                name: now.name.clone(),
                state: now.power_state.clone(),
                changed: patch.is_some(),
                summary,
            });
            if let Some(patch) = patch {
                vms.push(BatchVmProperties {
                    vm: now.id,
                    expected_state: now.power_state.clone(),
                    patch,
                });
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            errors.push(format!("{} ({}) · {error}", captured.name, captured.id));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Review {
        batch: VmPropertiesBatch { vms },
        rows,
    })
}

pub(crate) fn review(m: &mut Model) {
    let Some(e) = m.areas.bulk_edit.properties.as_ref() else {
        return;
    };
    let proposal = proposed(m, e);
    let Some(e) = edit(m) else { return };
    match proposal {
        Ok(r) if r.batch.vms.is_empty() => e.error = Some(NOTHING_TO_CHANGE.into()),
        Ok(r) => {
            e.review = Some(r);
            e.error = None;
        }
        Err(error) => e.error = Some(error),
    }
}

pub(crate) fn back(m: &mut Model) {
    if let Some(e) = edit(m) {
        e.review = None;
        e.error = None;
    }
}

/// Why "Apply changes" is disabled, the first that applies; `None` when
/// the reviewed batch can be submitted. The Rune button gave no reason:
/// each is the text the page shows for that state.
pub(crate) fn blocked(
    m: &Model,
    e: &Editor,
    proposal: &Result<Review, String>,
    denial: Option<String>,
) -> Option<String> {
    if m.is_synthetic_pool(e.pool) {
        return Some(DEMO.into());
    }
    if !m.can_mutate_pool(e.pool) {
        return Some(NOT_WRITABLE.into());
    }
    if m.resource_operations.len() >= 64 {
        return Some(Error::Busy.to_string());
    }
    match proposal {
        Err(reason) => return Some(reason.clone()),
        Ok(now) if Some(now) != e.review.as_ref() => return Some(REVIEW_CHANGED.into()),
        Ok(_) => {}
    }
    denial
}

/// Submits the reviewed batch. `Stale` in a synthetic pool, on a
/// connection that cannot change the pool, with 64 resource operations held
/// or when the review is not what would be reviewed now; a known permission
/// denial is `UnavailableAction`. Accepted, the batch is tracked and the
/// dialog closes; no history is opened (Rune opened none here).
pub(crate) fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.bulk_edit.properties.as_ref().ok_or(Error::Stale)?;
        let reviewed = e.review.as_ref().ok_or(Error::Stale)?;
        if m.is_synthetic_pool(e.pool)
            || !m.can_mutate_pool(e.pool)
            || m.resource_operations.len() >= 64
            || proposed(m, e).as_ref() != Ok(reviewed)
        {
            return Err(Error::Stale);
        }
        let assessment =
            permissions::assess_for(m, e.pool, &reviewed.batch.permission_requirements());
        if assessment.decision().is_denied() {
            return Err(Error::UnavailableAction);
        }
        m.connection_for(e.pool)
            .ok_or(Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::PropertiesBatch {
                    pool: e.observed.ok_or(Error::Stale)?,
                    selection: reviewed.batch.clone(),
                },
            )
    })();
    match result {
        Ok(operation) => {
            m.track_resource(operation);
            m.areas.bulk_edit.properties = None;
            m.close_properties();
        }
        Err(error) => {
            if let Some(e) = edit(m) {
                e.error = Some(error.to_string());
            }
        }
    }
}
