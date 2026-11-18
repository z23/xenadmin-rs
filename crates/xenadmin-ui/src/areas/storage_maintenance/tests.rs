//! The tests of the area.
//!
//! The thirteen Rune tests, each under a name that says which it was:
//!
//! | Rune test | Here |
//! | --- | --- |
//! | `storage_maintenance_view_tests.rs` | |
//! | `repair_review_captures_targets_and_remains_reachable_at_compact_and_wide_sizes` | `repair_review_captures_targets_and_stays_through_a_tab_change` (harness; the positions at two sizes are the GPUI half) |
//! | `repair_requires_selection_and_rejects_changed_attachment_graph` | moved, same name |
//! | `read_cache_review_explains_policy_and_preserves_target_at_both_widths` | `read_cache_review_explains_policy_and_preserves_target` (harness) |
//! | `cache_unknown_or_changed_current_policy_cannot_be_reviewed_or_submitted` | moved, same name |
//! | `selecting_a_host_attachment_reviews_only_that_exact_target_at_both_widths` | `selecting_a_host_attachment_reviews_only_that_exact_target` (harness) |
//! | `storage_graph_view_tests.rs` | |
//! | `reviewed_storage_graph_is_captured_reachable_and_never_submits_demo` | `reviewed_storage_graph_is_captured_and_never_submits_demo` (harness; positions are the GPUI half) |
//! | `missing_repair_requires_selection_and_rejects_new_pair_before_review` | moved, same name |
//! | `changed_multipath_attachment_and_pool_disable_captured_review` | moved, same name |
//! | `multipath_refuses_unknown_handler_and_nonmaintenance_host` | moved, same name |
//! | `edit_host_selection_then_review_keeps_exact_uuid` | harness, same name |
//! | `partial_outcome_restore_is_a_new_review_and_unknown_cannot_offer_it` | harness, same name |
//! | `restore_cannot_turn_into_detach_after_state_changes` | moved, same name (it calls the storage area's function) |
//!
//! None retired. Then what the port added: the commands, the Read Caching
//! page, the way from the Repair dialog and from a server's Multipathing
//! page into the workflow, closing, and the texts that name Rune places.
//!
//! Not tested, because the fixture server serves neither storage discovery
//! nor a storage write: a discovery that answers, a submission that is
//! accepted (the workflow staying open on "Submitted. …", the Repair dialog
//! closing), and a current observation going stale.

use std::sync::Arc;

use xenadmin_core::{
    Error, Inventory, OperationId, OperationState, PbdId, PoolId, ResourceAction,
    ResourceOperationRecord, ResourceTarget, StorageGraph, StorageMaintenance,
};

use super::{commands, graph, maintenance, scenes, view};
use crate::{Harness, prelude::*};

#[track_caller]
fn editor(h: &Harness) -> &maintenance::Editor {
    let editor = h.model.areas.storage_maintenance.editor.as_ref();
    editor.expect("a repair or read-cache editor")
}

#[track_caller]
fn editor_mut(h: &mut Harness) -> &mut maintenance::Editor {
    let editor = h.model.areas.storage_maintenance.editor.as_mut();
    editor.expect("a repair or read-cache editor")
}

#[track_caller]
fn workflow(h: &Harness) -> &graph::Editor {
    let editor = h.model.areas.storage_maintenance.graph.as_ref();
    editor.expect("a storage workflow")
}

#[track_caller]
fn workflow_mut(h: &mut Harness) -> &mut graph::Editor {
    let editor = h.model.areas.storage_maintenance.graph.as_mut();
    editor.expect("a storage workflow")
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = inventory(h).as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

fn first_pbd(h: &Harness) -> PbdId {
    *inventory(h).details.pbds.keys().next().expect("a PBD")
}

fn disabled(h: &Harness, key: &str) -> Option<String> {
    h.node(key).disabled
}

// -- storage_maintenance_view_tests.rs ------------------------------------------------

/// Rune: `repair_review_captures_targets_and_remains_reachable_at_compact_and_wide_sizes`.
#[test]
fn repair_review_captures_targets_and_stays_through_a_tab_change() {
    let mut h = Harness::scene("storage-repair");
    assert!(editor(&h).review.is_some(), "{:?}", editor(&h).error);
    let captured = editor(&h).review.clone();
    assert_eq!(h.surface().id, view::REPAIR);
    for key in ["storage-maintenance-confirm", "storage-close"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    // Rune changed the page under the dialog; the tab is what changes here.
    h.model.set_active_tab(TabId::General);
    assert_eq!(editor(&h).review, captured);
    assert_eq!(
        h.try_click("storage-maintenance-confirm"),
        Err(maintenance::DEMO.into())
    );
    // The handler refuses by itself, whatever the button said.
    maintenance::apply(&mut h.model);
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(editor(&h).review, captured);
    h.model.demo = false;
    assert!(h.has_text("captured connection changed"));
    assert_eq!(
        h.try_click("storage-maintenance-confirm"),
        Err(maintenance::STALE_REVIEW.into())
    );
    maintenance::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn repair_requires_selection_and_rejects_changed_attachment_graph() {
    let mut h = Harness::scene("storage-repair");
    let e = editor_mut(&mut h);
    e.review = None;
    e.selection.clear();
    // The button says what the review would.
    assert_eq!(
        h.try_click("storage-repair-review"),
        Err(maintenance::NOTHING_TO_DO.into())
    );
    maintenance::stage(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).error.as_deref(), Some(maintenance::NOTHING_TO_DO));
    let id = first_pbd(&h);
    amend(&mut h, |i| {
        i.details.pbds.get_mut(&id).unwrap().currently_attached = Some(true);
    });
    editor_mut(&mut h).selection.insert(id);
    maintenance::stage(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert_eq!(
        editor(&h).error.as_deref(),
        Some("A selected attachment changed; select it again")
    );
}

/// Rune: `read_cache_review_explains_policy_and_preserves_target_at_both_widths`.
#[test]
fn read_cache_review_explains_policy_and_preserves_target() {
    let mut h = Harness::scene("storage-cache");
    assert!(matches!(
        editor(&h).review,
        Some(StorageMaintenance::ReadCaching { enabled: false, .. })
    ));
    // The review is on the Read Caching page of the kernel's dialog.
    assert_eq!(h.surface().id, PropertiesKind::Storage.surface_id());
    assert!(h.has_text("does not establish effective caching"));
    h.click("storage-maintenance-back");
    assert!(editor(&h).review.is_none());
    h.click("storage-cache-review");
    assert!(editor(&h).review.is_some());
    assert_eq!(
        h.try_click("storage-maintenance-confirm"),
        Err(maintenance::DEMO.into())
    );
    maintenance::apply(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn cache_unknown_or_changed_current_policy_cannot_be_reviewed_or_submitted() {
    let mut h = Harness::scene("storage-cache");
    let sr = editor(&h).sr;
    let op = editor(&h).review.clone().unwrap();
    amend(&mut h, |i| {
        i.details.storage.get_mut(&sr).unwrap().read_cache_override = Some("true".into());
    });
    assert!(!op.availability(&inventory(&h), sr).is_available());
    // The review says so and its Confirm button carries the reason.
    let explanation = op.availability(&inventory(&h), sr).explanation();
    assert_eq!(
        h.node("storage-maintenance-unavailable").text,
        Some(explanation.clone())
    );
    assert_eq!(
        h.try_click("storage-maintenance-confirm"),
        Err(explanation)
    );
    editor_mut(&mut h).review = None;
    amend(&mut h, |i| {
        i.details
            .storage
            .get_mut(&sr)
            .unwrap()
            .read_cache_policy_known = false;
    });
    maintenance::stage(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert_eq!(
        editor(&h).error.as_deref(),
        Some(maintenance::POLICY_UNKNOWN)
    );
    assert_eq!(
        h.try_click("storage-cache-review"),
        Err(maintenance::POLICY_UNKNOWN.into())
    );
    assert_eq!(
        h.node("storage-cache-policy").text.as_deref(),
        Some("Read-cache policy: Unknown")
    );
}

/// Rune: `selecting_a_host_attachment_reviews_only_that_exact_target_at_both_widths`.
#[test]
fn selecting_a_host_attachment_reviews_only_that_exact_target() {
    let mut h = Harness::scene("storage-maintenance");
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).selection.len(), 2);
    editor_mut(&mut h).selection.clear();
    let id = first_pbd(&h);
    h.click(&format!("repair-pbd-{id}"));
    assert_eq!(editor(&h).selection.len(), 1);
    h.click("storage-repair-review");
    let Some(StorageMaintenance::Repair { attachments, .. }) = &editor(&h).review else {
        panic!("review missing")
    };
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].attachment, id);
}

// -- storage_graph_view_tests.rs ------------------------------------------------------

/// Rune: `reviewed_storage_graph_is_captured_reachable_and_never_submits_demo`.
#[test]
fn reviewed_storage_graph_is_captured_and_never_submits_demo() {
    for scene in ["storage-graph", "storage-multipath"] {
        let mut h = Harness::scene(scene);
        assert!(workflow(&h).review.is_some(), "{:?}", workflow(&h).error);
        let captured = workflow(&h).review.clone();
        assert_eq!(h.surface().id, view::WORKFLOW);
        for key in ["storage-graph-close", "storage-graph-submit"] {
            assert!(h.find(key).is_some(), "{scene}: {key}");
        }
        h.model.set_active_tab(TabId::General);
        assert_eq!(workflow(&h).review, captured);
        assert_eq!(
            h.try_click("storage-graph-submit"),
            Err(graph::DEMO.into()),
            "{scene}"
        );
        graph::submit(&mut h.model);
        assert_eq!(workflow(&h).error, Some(Error::Stale.to_string()));
        assert_eq!(workflow(&h).review, captured);
        assert!(h.model.resource_operations.is_empty());
    }
}

#[test]
fn missing_repair_requires_selection_and_rejects_new_pair_before_review() {
    let mut h = Harness::scene("storage-graph");
    let e = workflow_mut(&mut h);
    e.review = None;
    let selected = *e.hosts.first().unwrap();
    let source = e.source.unwrap();
    e.hosts.clear();
    graph::stage(&mut h.model);
    assert!(workflow(&h).review.is_none());
    // The button already says what the proposal lacks.
    assert_eq!(
        disabled(&h, "storage-graph-review"),
        graph::proposal(workflow(&h)).err()
    );
    workflow_mut(&mut h).hosts.insert(selected);
    amend(&mut h, |i| {
        let mut new = i.details.pbds[&source].clone();
        new.id = PbdId::from(uuid::Uuid::from_u128(9951));
        new.host = Some(selected);
        new.currently_attached = Some(false);
        i.details.pbds.insert(new.id, new);
    });
    graph::stage(&mut h.model);
    assert!(workflow(&h).review.is_none());
    assert!(workflow(&h).error.as_ref().unwrap().contains("changed"));
}

#[test]
fn changed_multipath_attachment_and_pool_disable_captured_review() {
    let mut h = Harness::scene("storage-multipath");
    let id = first_pbd(&h);
    let op = workflow(&h).review.clone().unwrap();
    assert!(graph::available(&h.model, workflow(&h), &op));
    assert!(h.find("storage-graph-changed").is_none());
    amend(&mut h, |i| {
        i.details.pbds.get_mut(&id).unwrap().currently_attached = Some(false);
    });
    assert!(!graph::available(&h.model, workflow(&h), &op));
    assert_eq!(
        h.node("storage-graph-changed").text.as_deref(),
        Some(graph::CHANGED)
    );
    assert_eq!(
        h.try_click("storage-graph-submit"),
        Err(graph::CHANGED.into())
    );
    amend(&mut h, |i| {
        i.pool_id = PoolId::from(uuid::Uuid::from_u128(999));
    });
    assert!(!graph::current(&h.model, workflow(&h)));
    assert_eq!(
        h.try_click("storage-graph-submit"),
        Err(graph::STALE.into())
    );
    graph::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

#[test]
fn multipath_refuses_unknown_handler_and_nonmaintenance_host() {
    for changed in ["handler", "maintenance", "ha"] {
        let mut h = Harness::scene("storage-multipath");
        let mut i = inventory(&h).as_ref().clone();
        let ResourceTarget::Host(host) = workflow(&h).target else {
            panic!()
        };
        match changed {
            "handler" => {
                i.details
                    .hosts
                    .get_mut(&host)
                    .unwrap()
                    .standard_multipath_handler = None
            }
            "maintenance" => i.details.hosts.get_mut(&host).unwrap().maintenance_mode = Some(false),
            _ => i.details.pool.as_mut().unwrap().ha_enabled = Some(true),
        }
        workflow_mut(&mut h).initial = Arc::new(i);
        assert!(graph::proposal(workflow(&h)).is_err(), "{changed}");
    }
}

#[test]
fn edit_host_selection_then_review_keeps_exact_uuid() {
    let mut h = Harness::scene("storage-graph");
    let host = *workflow(&h).hosts.first().unwrap();
    h.click("storage-graph-back");
    h.click(&format!("storage-host-{host}"));
    assert!(workflow(&h).hosts.is_empty());
    h.click(&format!("storage-host-{host}"));
    h.click("storage-graph-review");
    let Some(StorageGraph::Repair { hosts, .. }) = &workflow(&h).review else {
        panic!()
    };
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].host, host);
    assert!(hosts[0].attachment.is_none());
}

#[test]
fn partial_outcome_restore_is_a_new_review_and_unknown_cannot_offer_it() {
    for state in [
        OperationState::PartiallyCompleted,
        OperationState::OutcomeUnknown,
    ] {
        let mut h = Harness::scene("storage-multipath");
        let e = workflow(&h);
        let Some(StorageGraph::Multipath { attachments, .. }) = e.review.as_ref() else {
            panic!()
        };
        let p = attachments[0].clone();
        let record: ResourceOperationRecord = serde_json::from_value(serde_json::json!({
            "id": OperationId::new(),
            "pool": e.pool,
            "target": e.target,
            "action": ResourceAction::StorageGraph(e.review.clone().unwrap()),
            "state": state,
            "progress": 0.25,
            "detail": "Fixture stopped after disconnect",
            "created_at_ms": 1,
            "can_cancel": false,
            "cancel_requested": false,
            "steps": [],
            "created": null,
            "storage_graph": [{
                "host": p.host, "sr": p.sr, "pbd": p.pbd,
                "created": false, "attached": false, "verified": true
            }]
        }))
        .unwrap();
        h.model.resource_rows.push(record);
        amend(&mut h, |i| {
            i.details.pbds.get_mut(&p.pbd).unwrap().currently_attached = Some(false);
        });
        // The record is in the history of this target either way.
        assert!(h.has_text("Fixture stopped after disconnect"));
        assert!(h.has_text("verified detached"));
        let key = format!("storage-restore-{}", p.pbd);
        if state == OperationState::PartiallyCompleted {
            h.click(&key);
            assert!(h.model.areas.storage_maintenance.graph.is_none());
            // The storage area's review, which can only attach.
            assert_eq!(h.surface().id, "storage-change");
            assert!(h.model.resource_operations.is_empty());
            assert!(h.has_text("Review storage change"));
        } else {
            assert!(h.find(&key).is_none());
        }
    }
}

/// A Restore whose attachment is not the detached one any more is offered
/// disabled, with the storage area's refusal.
#[test]
fn restore_of_a_changed_attachment_is_disabled_with_the_reason() {
    let mut h = Harness::scene("storage-multipath");
    let e = workflow(&h);
    let Some(StorageGraph::Multipath { attachments, .. }) = e.review.as_ref() else {
        panic!()
    };
    let p = attachments[0].clone();
    let record: ResourceOperationRecord = serde_json::from_value(serde_json::json!({
        "id": OperationId::new(), "pool": e.pool, "target": e.target,
        "action": ResourceAction::StorageGraph(e.review.clone().unwrap()),
        "state": OperationState::PartiallyCompleted, "progress": 0.25, "detail": "Stopped",
        "created_at_ms": 1, "can_cancel": false, "cancel_requested": false, "steps": [],
        "created": null,
        "storage_graph": [{
            "host": p.host, "sr": p.sr, "pbd": p.pbd,
            "created": false, "attached": false, "verified": true
        }]
    }))
    .unwrap();
    h.model.resource_rows.push(record);
    // The inventory still has it attached.
    let reason = disabled(&h, &format!("storage-restore-{}", p.pbd));
    assert_eq!(
        reason.as_deref(),
        Some("The detached attachment changed; reopen its storage view")
    );
}

#[test]
fn restore_cannot_turn_into_detach_after_state_changes() {
    let mut h = Harness::scene("storage-multipath");
    let p = inventory(&h).details.pbds.values().next().unwrap().clone();
    let pool = pool(&h);
    crate::areas::storage::review_restore(
        &mut h.model,
        pool,
        p.sr.unwrap(),
        p.id,
        p.host.unwrap(),
    );
    // No storage review opened: the workflow is still the one surface.
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, view::WORKFLOW);
    assert!(h.model.error.as_ref().unwrap().contains("changed"));
}

// -- Commands -------------------------------------------------------------------------

/// The demo pool of the repair scenes with nothing open: the pool is the
/// scope, its Storage tab is shown.
fn prepared() -> (Harness, Target) {
    let mut h = Harness::scene("storage-maintenance");
    let sr = editor(&h).sr;
    h.dismiss();
    assert!(h.surfaces().is_empty());
    let repository = Target::one(ObjectRef::storage(pool(&h), sr.uuid()));
    (h, repository)
}

#[test]
fn repair_is_a_command_on_a_repository() {
    let (mut h, repository) = prepared();
    let pool = pool(&h);
    let eligibility = |h: &Harness, target: &Target| h.model.eligibility(commands::REPAIR, target);
    assert_eq!(eligibility(&h, &repository), Eligibility::Enabled);
    // The grid's owner without a selected row: the storage area's words.
    assert_eq!(commands::SELECT, crate::areas::storage::commands::SELECT);
    let host = *inventory(&h).hosts.keys().next().unwrap();
    for owner in [ObjectRef::pool(pool), ObjectRef::host(pool, host)] {
        assert_eq!(
            eligibility(&h, &Target::one(owner)),
            Eligibility::disabled(commands::SELECT)
        );
    }
    // Nothing to do with repositories, or not one the inventory has.
    let vm = *inventory(&h).vms.keys().next().unwrap();
    let unknown = ObjectRef::storage(pool, uuid::Uuid::from_u128(0xdead));
    for other in [
        Target::one(ObjectRef::vm(pool, vm)),
        Target::one(unknown),
        Target::none(),
    ] {
        assert_eq!(eligibility(&h, &other), Eligibility::Hidden);
    }
    // Its places: the Storage menu, the repository's context menu, the
    // local row of the Storage tab.
    assert_eq!(h.menu_item("storage.repair").unwrap().label, "Repair");
    let in_context = h.context_menu(&repository);
    assert!(in_context.iter().any(|item| item.id == commands::REPAIR));
    let item = |h: &Harness| {
        let row = h.local_row();
        row.into_iter().find(|item| item.id == commands::REPAIR)
    };
    assert_eq!(
        item(&h).expect("in the local row").disabled.as_deref(),
        Some(commands::SELECT)
    );
    let row = repository.single().unwrap().uuid().unwrap().to_string();
    h.select_row("storage-repositories-table", &row);
    assert_eq!(item(&h).expect("in the local row").disabled, None);
    h.click("storage.repair");
    assert_eq!(h.surface().id, view::REPAIR);
    assert!(h.surface().experimental);
    assert_eq!(editor(&h).kind, maintenance::Kind::Repair);
    assert!(editor(&h).selection.is_empty());
    assert!(editor(&h).review.is_none());
    h.assert_unique_keys();
}

/// The Rune hub's button into the workflow is in the Repair dialog, and
/// the workflow opens on the same repository.
#[test]
fn the_repair_dialog_leads_to_the_workflow_on_its_repository() {
    let mut h = Harness::scene("storage-maintenance");
    let sr = editor(&h).sr;
    assert_eq!(
        h.node("storage-graph-open").label.as_deref(),
        Some("Repair missing or detached host connections…")
    );
    h.click("storage-graph-open");
    assert!(h.model.areas.storage_maintenance.editor.is_none());
    assert_eq!(h.surface().id, view::WORKFLOW);
    assert!(h.surface().experimental);
    let e = workflow(&h);
    assert_eq!(e.target, ResourceTarget::Storage(sr));
    assert!(e.review.is_none() && e.hosts.is_empty());
    // The first shared connection is the source; choosing another empties
    // the chosen hosts.
    let sources: Vec<PbdId> = inventory(&h).details.pbds.keys().copied().collect();
    assert_eq!(e.source, Some(sources[0]));
    for id in &sources {
        assert!(h.find(&format!("storage-source-{id}")).is_some());
    }
    let third = *inventory(&h).hosts.keys().nth(2).unwrap();
    h.click(&format!("storage-host-{third}"));
    assert_eq!(workflow(&h).hosts.len(), 1);
    h.pick("storage-source", &sources[1].to_string());
    assert_eq!(workflow(&h).source, Some(sources[1]));
    assert!(workflow(&h).hosts.is_empty());
    assert!(h.find("storage-graph-back").is_none());
    h.assert_unique_keys();
}

#[test]
fn the_multipath_workflow_is_the_way_from_the_multipathing_page() {
    let mut h = Harness::scene("storage-multipath");
    let ResourceTarget::Host(host) = workflow(&h).target else {
        panic!()
    };
    let pool = pool(&h);
    let server = Target::one(ObjectRef::host(pool, host));
    h.dismiss();
    assert!(h.surfaces().is_empty());
    let eligibility =
        |h: &Harness, target: &Target| h.model.eligibility(commands::MULTIPATH_WORKFLOW, target);
    assert_eq!(eligibility(&h, &server), Eligibility::Enabled);
    for other in [Target::one(ObjectRef::pool(pool)), Target::none()] {
        assert_eq!(eligibility(&h, &other), Eligibility::Hidden);
    }
    // No place of its own: the page of the server's Properties dialog.
    assert!(h.menu_item("storage.multipath-workflow").is_none());
    assert!(
        !h.context_menu(&server)
            .iter()
            .any(|item| item.id == commands::MULTIPATH_WORKFLOW)
    );
    h.invoke("server.properties");
    h.pick("host-properties-pages", "multipathing");
    h.click("host-multipath-workflow");
    assert!(h.model.properties.is_none());
    assert_eq!(h.surface().id, view::WORKFLOW);
    // Nothing to enter: on the review, without a way back to a draft.
    assert!(matches!(
        workflow(&h).review,
        Some(StorageGraph::Multipath { enabled: true, .. })
    ));
    assert!(h.find("storage-graph-back").is_none());
    assert!(h.find("storage-graph-review").is_none());
    h.assert_unique_keys();
}

/// A server that is not in the state the change needs still opens the
/// workflow: with the reason, and with the history and its controls.
#[test]
fn a_server_that_cannot_be_reviewed_opens_with_the_reason_and_the_history() {
    let mut h = Harness::scene("storage-multipath");
    let ResourceTarget::Host(host) = workflow(&h).target else {
        panic!()
    };
    let server = Target::one(ObjectRef::host(pool(&h), host));
    h.dismiss();
    amend(&mut h, |i| {
        i.details.hosts.get_mut(&host).unwrap().maintenance_mode = Some(false);
    });
    assert!(h.model.invoke(commands::MULTIPATH_WORKFLOW, &server));
    let reason = "First disable HA, evacuate guests and enter maintenance for this host";
    assert!(workflow(&h).review.is_none());
    assert_eq!(workflow(&h).error, None);
    assert_eq!(
        h.node("storage-graph-unavailable").text.as_deref(),
        Some(reason)
    );
    assert_eq!(h.try_click("storage-graph-review"), Err(reason.into()));
    assert!(h.has_text("Recent storage workflow results"));
    assert!(h.find("storage-graph-submit").is_none());
}

// -- The Read Caching page ------------------------------------------------------------

#[test]
fn read_caching_is_a_page_of_the_repository_properties() {
    let (mut h, repository) = prepared();
    let pages = h.model.property_pages(&repository);
    let (area, page) = pages
        .iter()
        .find(|(_, page)| page.id == maintenance::PAGE)
        .expect("the page");
    assert_eq!(*area, "storage_maintenance");
    assert_eq!(page.label, "Read Caching");
    assert!(page.experimental);
    // Only for a repository the inventory has.
    let pool_target = Target::one(ObjectRef::pool(pool(&h)));
    assert!(view::pages(&h.model, &pool_target).is_empty());
    let unknown = ObjectRef::storage(pool(&h), uuid::Uuid::from_u128(0xdead));
    assert!(view::pages(&h.model, &Target::one(unknown)).is_empty());

    assert!(
        h.model
            .open_properties(&repository, Some(maintenance::PAGE))
    );
    assert_eq!(h.surface().id, PropertiesKind::Storage.surface_id());
    assert!(h.surface().experimental);
    assert_eq!(editor(&h).kind, maintenance::Kind::Cache);
    // No driver capabilities yet: nothing to change, a way to ask again.
    // In a synthetic pool the question has no one to go to and no error.
    assert!(h.find("storage-cache-review").is_none());
    h.click("storage-capabilities-refresh");
    assert_eq!(editor(&h).error, None);
    assert!(editor(&h).discovery.is_none());
    assert!(!maintenance::poll(&mut h.model, 0.0));

    editor_mut(&mut h).backends = vec![scenes::nfs_backend()];
    assert!(h.has_text("Driver: NFS · Thin provisioning: advertised"));
    assert_eq!(
        h.node("storage-cache-policy").text.as_deref(),
        Some("Read-cache policy: Permitted")
    );
    assert_eq!(
        h.node("storage-cache-review").label.as_deref(),
        Some("Review disabling read caching…")
    );
    h.assert_unique_keys();

    // The host's override "true" means disabled: the change enables.
    let sr = editor(&h).sr;
    amend(&mut h, |i| {
        i.details.storage.get_mut(&sr).unwrap().read_cache_override = Some("true".into());
    });
    assert_eq!(
        h.node("storage-cache-policy").text.as_deref(),
        Some("Read-cache policy: Disabled")
    );
    assert_eq!(
        h.node("storage-cache-review").label.as_deref(),
        Some("Review enabling read caching…")
    );
    h.click("storage-cache-review");
    assert!(matches!(
        editor(&h).review,
        Some(StorageMaintenance::ReadCaching { enabled: true, .. })
    ));
    assert!(h.has_text("Permit read caching by removing the repository override."));
    h.click("storage-maintenance-back");

    // A driver that does not advertise read caching offers no change.
    editor_mut(&mut h).backends[0].features.clear();
    assert!(h.has_text("This driver does not advertise supported read caching."));
    assert!(h.has_text("Thin provisioning: not advertised"));
    assert!(h.find("storage-cache-review").is_none());

    // Close takes the dialog and the editor.
    h.click("storage-close");
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.storage_maintenance.editor.is_none());
    assert!(h.surfaces().is_empty());
}

#[test]
fn leaving_the_page_or_the_dialog_drops_the_read_cache_editor() {
    let mut h = Harness::scene("storage-cache");
    assert!(h.model.properties.is_some());
    h.dismiss();
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.storage_maintenance.editor.is_none());

    // The Repair command on the repository takes the dialog's place.
    let mut h = Harness::scene("storage-cache");
    let sr = editor(&h).sr;
    let pool = pool(&h);
    assert!(maintenance::open_repair(&mut h.model, pool, sr));
    assert!(h.model.properties.is_none());
    assert_eq!(h.surface().id, view::REPAIR);
    assert_eq!(editor(&h).kind, maintenance::Kind::Repair);
}

// -- Closing, order, texts ------------------------------------------------------------

#[test]
fn closing_and_a_scope_change_keep_nothing() {
    let mut h = Harness::scene("storage-repair");
    h.click("storage-close");
    assert!(h.model.areas.storage_maintenance.editor.is_none());

    let mut h = Harness::scene("storage-graph");
    h.click("storage-graph-close");
    assert!(h.model.areas.storage_maintenance.graph.is_none());

    for scene in ["storage-repair", "storage-graph", "storage-cache"] {
        let mut h = Harness::scene(scene);
        let host = *inventory(&h).hosts.keys().nth(2).unwrap();
        let other = Target::one(ObjectRef::host(pool(&h), host));
        h.select(other);
        let state = &h.model.areas.storage_maintenance;
        assert!(state.editor.is_none() && state.graph.is_none(), "{scene}");
        assert!(h.surfaces().is_empty(), "{scene}");
    }
}

/// Map hazard 17: the review lists a repair in the order it runs, the
/// coordinator first, whatever the order of the ids.
#[test]
fn a_repair_is_reviewed_coordinator_first() {
    let mut h = Harness::scene("storage-repair");
    let i = inventory(&h);
    let hosts: Vec<_> = i.hosts.keys().copied().take(2).collect();
    let name = |n: usize| i.hosts[&hosts[n]].name.clone();
    for (coordinator, order) in [(0, [0, 1]), (1, [1, 0])] {
        amend(&mut h, |i| i.coordinator = Some(hosts[coordinator]));
        editor_mut(&mut h).review = None;
        maintenance::stage(&mut h.model);
        let e = editor(&h);
        let lines = maintenance::lines(&h.model, e, e.review.as_ref().expect("a review"));
        let at = |line: String| lines.iter().position(|l| *l == line).expect("the line");
        let first = at(format!("1. {}", name(order[0])));
        let second = at(format!("2. {}", name(order[1])));
        assert!(first < second);
        assert_eq!(
            lines[first + 1],
            format!("Host {} · coordinator", hosts[coordinator])
        );
        assert_eq!(lines[second + 1], format!("Host {}", hosts[1 - coordinator]));
    }
}

/// Every surface of the area carries the experimental notice, every scene
/// has unique keys, and the reviews keep the Rune words that name Rune
/// places (they are renamed in one sweep, decision 10).
#[test]
fn scenes_are_experimental_and_the_rune_texts_are_kept() {
    for scene in [
        "storage-maintenance",
        "storage-repair",
        "storage-cache",
        "storage-graph",
        "storage-multipath",
    ] {
        let h = Harness::scene(scene);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
        h.assert_unique_keys();
    }
    assert_eq!(
        maintenance::STALE_REVIEW,
        "The captured connection changed; close and reopen storage administration before \
         submitting."
    );
    assert_eq!(
        maintenance::STALE_STAGE,
        "The captured connection changed; reopen storage administration"
    );
    let h = Harness::scene("storage-repair");
    assert!(h.has_text("Stop remaining steps does not cancel an in-flight host task."));
    assert!(h.has_text("Repair selected host attachments"));

    let mut h = Harness::scene("storage-graph");
    assert!(h.has_text("Review storage workflow"));
    assert!(h.has_text(
        "After a known partial result, use the separately reviewed Restore connection controls \
         below."
    ));
    assert!(h.has_text("Synthetic preview · no changes can be submitted"));
    assert_eq!(
        h.node("storage-graph-activity").label.as_deref(),
        Some("Open Activity history")
    );
    // Rune's Activity is the Events tab of the Notifications view.
    h.click("storage-graph-activity");
    assert!(h.model.areas.storage_maintenance.graph.is_none());
    assert_eq!(h.model.nav.view, NavView::Notifications);
    assert_eq!(h.model.active_tab(), TabId::Events);

    let h = Harness::scene("storage-multipath");
    assert!(h.has_text("Multipathing: false → true"));
    assert!(h.has_text("Handler: absent → dmp"));
    assert!(h.has_text("Disconnect, then reconnect after the policy change"));
}
