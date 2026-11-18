//! The tests of the area, in two parts.
//!
//! 1. **The Rune tests** of `network_service_ui.rs` (2),
//!    `network_topology_ui.rs` (4 of its 5), `network_fabric_ui.rs` (3) and
//!    `network_fabric_pool_ui.rs` (3), under their Rune names, and this
//!    area's share of
//!    `dialogs_outside_the_supported_core_carry_the_experimental_notice`
//!    (`main.rs`). Where the Rune test found `network-confirm` or
//!    `network-review` by its key and pressed it, the test here does the
//!    same through the harness; "inside the window at both sizes" is the
//!    front end's to show (`notes/progress/gpui-tests-wanted.md`).
//! 2. **What the port changed**: the buttons of the hub are commands, the
//!    fabric forms are wizards, the removal of a record is a dialog, and
//!    the changes with nothing to enter open the shared review.
//!
//! Not here, with the reason:
//!
//! - `topology_export_contains_only_public_created_and_removed_ids`
//!   (`network_topology_ui.rs`): the export is the `events` area's, which
//!   ported the test under its name.
//!
//! No fixture of these workflows has a current snapshot (the fixture
//! server serves no network operation), so no test submits successfully:
//! the refusals are tested.

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    Error, Inventory, NetworkId, NetworkServicePurpose, NetworkServiceReview, NetworkTopologyOperation,
    NetworkTunnel, PifId, PoolId, ResourceAction, ResourceTarget, TunnelId, Vlan,
};

use super::commands::{
    FABRIC_ADD, FABRIC_COMPLETE_POOL, FABRIC_REMOVE, FABRIC_REMOVE_POOL, SERVICE_PLAINTEXT,
    SERVICE_TLS, VLAN_CREATE_POOL, VLAN_RETAG_POOL,
};
use super::edit::{self, Draft, Editor, STALE};
use super::fabric::{self, Kind, TunnelProtocol};
use super::view::{FABRIC, FABRIC_POOL, FABRIC_REMOVE as REMOVE_DIALOG, REVIEW, TOPOLOGY};
use super::{scenes, service, topology};
use crate::Harness;
use crate::areas::network::{
    self, Workspace,
    commands::{NOT_WRITABLE, SELECT_NETWORK},
    view::{CHANGE, DEMO, PREVIEW},
};
use crate::prelude::*;

// -- Fixtures -----------------------------------------------------------------------

#[track_caller]
fn ed(h: &Harness) -> &Editor {
    h.model
        .areas
        .network_services
        .editor
        .as_ref()
        .expect("an editor")
}

/// The shared review dialog of the `network` area.
#[track_caller]
fn ws(h: &Harness) -> &Workspace {
    h.model.areas.network.workspace.as_ref().expect("a workspace")
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("an inventory")
}

fn first_network(h: &Harness) -> NetworkId {
    NetworkId::from(inventory(h).networks[0].uuid)
}

/// Replaces the inventory with an edited copy.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = h.model.inventory.take().expect("inventory").as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

#[track_caller]
fn topology_operation(h: &Harness) -> NetworkTopologyOperation {
    match &ed(h).review.as_ref().expect("a review").operation {
        ResourceAction::NetworkTopology(operation) => operation.clone(),
        other => panic!("a topology operation, not {other:?}"),
    }
}

/// The step a wizard shows.
#[track_caller]
fn step(h: &Harness) -> usize {
    match &h.surface().kind {
        SurfaceKind::Wizard { current, .. } => *current,
        _ => panic!("a wizard"),
    }
}

/// The Rune `retag_scene`: every uplink of the topology scene gets a VLAN
/// interface with tag 40 on the first network (the first plugged, the
/// others not), and the retagging is opened by its command.
fn retag_scene() -> Harness {
    let mut h = Harness::scene("network-topology");
    let network = ed(&h).network;
    let NetworkTopologyOperation::CreateVlans { uplinks, .. } = topology_operation(&h) else {
        panic!("the scene reviews a creation")
    };
    h.click("network-close");
    amend(&mut h, |i| {
        for (index, parent) in uplinks.iter().enumerate() {
            let mut child = i.details.pifs[&parent.pif].clone();
            child.id = Uuid::from_u128(600 + index as u128).into();
            child.network = Some(network);
            child.vlan = Some(40);
            child.physical = Some(false);
            child.currently_attached = Some(index == 0);
            let vlan = Uuid::from_u128(700 + index as u128).into();
            i.details.vlans.insert(
                vlan,
                Vlan {
                    id: vlan,
                    tagged_pif: Some(parent.pif),
                    untagged_pif: Some(child.id),
                    tag: Some(40),
                },
            );
            i.details.pifs.insert(child.id, child);
        }
    });
    h.invoke(VLAN_RETAG_POOL.0);
    assert_eq!(h.surface().id, TOPOLOGY);
    assert!(matches!(
        &ed(&h).draft,
        Draft::Topology(t) if matches!(&t.selection, topology::Selection::Retag(v) if v.len() == uplinks.len())
    ));
    h
}

/// A tunnel record over `input` with its access interface on `network`.
fn add_tunnel(
    i: &mut Inventory,
    network: NetworkId,
    input: PifId,
    protocol: TunnelProtocol,
    index: u128,
) -> TunnelId {
    let mut access = i.details.pifs[&input].clone();
    access.id = PifId::from(Uuid::from_u128(0x7800 + index));
    access.network = Some(network);
    access.physical = Some(false);
    access.device = format!("tunnel{index}");
    access.ip_configuration_mode = "None".into();
    access.ip.clear();
    access.netmask.clear();
    let id = TunnelId::from(Uuid::from_u128(0x7900 + index));
    i.details.network_fabric.tunnels.insert(
        id,
        NetworkTunnel {
            id,
            transport: Some(input),
            access: Some(access.id),
            protocol: Some(protocol),
            controller_active: Some(true),
            controller_error: Some(false),
        },
    );
    i.details.pifs.insert(access.id, access);
    id
}

// -- 1. The Rune tests ----------------------------------------------------------------

/// `network_service_ui.rs`. The review is opened by its command here.
#[test]
fn review_captures_plaintext_policy_and_interfaces_at_both_widths() {
    let mut h = Harness::scene("network-services");
    assert_eq!(h.surface().id, CHANGE);
    let i = inventory(&h);
    let network = first_network(&h);
    let review = NetworkServiceReview::capture(&i, network).expect("a service review");
    let lines = service::lines(
        &i,
        network,
        &review,
        NetworkServicePurpose::NbdPlaintext,
        true,
    );
    assert!(lines.iter().any(|s| s.contains("Plaintext NBD")));
    assert!(lines.iter().any(|s| s.contains("192.0.2.20")));
    h.click("network-close");
    h.invoke(SERVICE_PLAINTEXT.0);
    assert_eq!(h.surface().id, CHANGE);
    assert_eq!(ws(&h).review.as_ref().expect("a review").lines, lines);
    assert!(h.has_text("Plaintext NBD has no transport confidentiality"));
    // A review that its command opened has no draft to go back to.
    assert!(h.find("network-back").is_none());
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    network::workspace::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(ws(&h).review.is_some());
}

/// `network_service_ui.rs`.
#[test]
fn changed_interface_pool_or_service_policy_invalidates_captured_review() {
    let mut h = Harness::scene("network-services");
    assert!(service::unavailable(&h.model, ws(&h)).is_none());
    let before = h.model.inventory.clone();
    amend(&mut h, |i| {
        i.details.pifs.values_mut().next().expect("pif").ip = "192.0.2.99".into()
    });
    let reason = service::unavailable(&h.model, ws(&h)).expect("a changed interface");
    // The rule is the check of the shared review: it disables the confirm
    // button with its reason.
    assert_eq!(h.find("network-unavailable").and_then(|n| n.text), Some(reason.clone()));
    assert_eq!(h.try_click("network-confirm"), Err(reason));
    h.model.inventory = before.clone();
    amend(&mut h, |i| i.pool_id = PoolId::from(Uuid::from_u128(999)));
    assert!(service::unavailable(&h.model, ws(&h)).is_some());
    h.model.inventory = before;
    amend(&mut h, |i| {
        let id = NetworkId::from(i.networks[1].uuid);
        i.details.networks.get_mut(&id).expect("details").purposes =
            Some(["insecure_nbd".into()].into());
    });
    assert!(
        service::unavailable(&h.model, ws(&h))
            .expect("a conflicting policy")
            .contains("conflicts")
    );
}

/// `network_topology_ui.rs`.
#[test]
fn pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch() {
    for retag in [false, true] {
        let mut h = if retag {
            let mut h = retag_scene();
            h.click("network-review");
            h
        } else {
            Harness::scene("network-topology")
        };
        assert_eq!(h.surface().id, TOPOLOGY);
        let i = inventory(&h);
        let operation = topology_operation(&h);
        let review = ed(&h).review.as_ref().expect("a review");
        assert!(operation.availability(&i, ed(&h).network).is_available());
        assert!(review.lines.iter().any(|v| v.contains("never repeats")));
        assert!(h.has_text("never repeats"));
        if retag {
            let NetworkTopologyOperation::RetagVlans { interfaces, .. } = &operation else {
                panic!("a retag")
            };
            assert_eq!(interfaces.len(), i.hosts.len());
            assert!(review.lines.iter().any(|v| v.contains("old VLAN")));
        } else {
            assert!(matches!(
                &operation,
                NetworkTopologyOperation::CreateVlans { uplinks, tag: 100 } if uplinks.len() == i.hosts.len()
            ));
        }
        assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
        edit::submit(&mut h.model);
        assert!(h.model.resource_operations.is_empty());
        assert!(ed(&h).review.is_some());
    }
}

/// `network_topology_ui.rs`.
#[test]
fn topology_review_refuses_changed_pool_graph_and_keeps_captured_identities() {
    let mut h = Harness::scene("network-topology");
    assert!(edit::unavailable(&h.model, ed(&h)).is_none());
    let original = ed(&h).review.as_ref().expect("a review").operation.clone();
    let NetworkTopologyOperation::CreateVlans { uplinks, .. } = topology_operation(&h) else {
        panic!("a creation")
    };
    let pif = uplinks[0].pif;
    amend(&mut h, |i| {
        i.details.pifs.get_mut(&pif).expect("pif").mac = "02:00:00:00:ff:ff".into()
    });
    let reason = edit::unavailable(&h.model, ed(&h)).expect("a changed uplink");
    assert_eq!(ed(&h).review.as_ref().expect("a review").operation, original);
    assert_eq!(h.find("network-unavailable").and_then(|n| n.text), Some(reason.clone()));
    assert_eq!(h.try_click("network-confirm"), Err(reason));
    amend(&mut h, |i| i.pool_id = Uuid::new_v4().into());
    assert!(
        edit::unavailable(&h.model, ed(&h))
            .expect("another pool")
            .contains("pool changed")
    );
}

/// `network_topology_ui.rs`.
#[test]
fn retag_review_refuses_protection_consumers_missing_inventory_and_unchanged_tag() {
    for change in 0..5 {
        let mut h = retag_scene();
        let network = ed(&h).network;
        if change == 4 {
            h.type_text("network-topology-tag", "40");
        }
        amend(&mut h, |i| {
            let child = i
                .details
                .pifs
                .values()
                .find(|p| p.network == Some(network))
                .expect("a VLAN interface")
                .id;
            match change {
                0 => i.details.pifs.get_mut(&child).expect("pif").disallow_unplug = Some(true),
                1 => {
                    i.details.pifs.get_mut(&child).expect("pif").management_purpose =
                        Some("Storage".into())
                }
                2 => {
                    i.details.classes.remove("vlan");
                }
                3 => {
                    i.hosts.pop_first();
                }
                _ => {}
            }
        });
        // The form says why already (the live validation of the Rune form).
        assert!(h.find("network-topology-reason").is_some(), "change {change}");
        h.click("network-review");
        assert!(ed(&h).review.is_none(), "change {change}");
        assert!(ed(&h).error.is_some(), "change {change}");
        assert!(h.find("network-error").is_some(), "change {change}");
    }
}

/// `network_topology_ui.rs`.
#[test]
fn vlan_zero_is_explicit_and_invalid_tags_cannot_reach_review() {
    for tag in ["0", "4094", "4095", "-1", "garbage"] {
        let mut h = Harness::scene("network-topology");
        h.click("network-back");
        assert!(ed(&h).review.is_none());
        h.type_text("network-topology-tag", tag);
        h.click("network-review");
        assert_eq!(
            ed(&h).review.is_some(),
            matches!(tag, "0" | "4094"),
            "{tag}"
        );
    }
}

/// `network_fabric_ui.rs`.
#[test]
fn host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit() {
    let mut h = Harness::scene("network-fabric");
    assert_eq!(h.surface().id, FABRIC);
    assert_eq!(step(&h), 3);
    let lines = &ed(&h).review.as_ref().expect("a review").lines;
    assert!(lines.iter().any(|v| v.contains("host only")));
    assert!(
        lines
            .iter()
            .any(|v| v.contains("does not establish traffic"))
    );
    assert!(h.has_text("does not establish traffic"));
    assert!(h.find(REVIEW).is_some());
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    edit::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// `network_fabric_ui.rs`.
#[test]
fn review_fences_changed_controller_transport_and_pool() {
    let mut h = Harness::scene("network-fabric");
    assert!(edit::unavailable(&h.model, ed(&h)).is_none());
    amend(&mut h, |i| {
        i.details
            .network_fabric
            .controllers
            .values_mut()
            .next()
            .expect("a controller")
            .configuration = None
    });
    let reason = edit::unavailable(&h.model, ed(&h)).expect("a changed controller");
    assert_eq!(h.try_click("network-confirm"), Err(reason));
}

/// `network_fabric_ui.rs`.
#[test]
fn sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes() {
    let mut h = Harness::scene("network-sriov");
    assert_eq!(h.surface().id, FABRIC);
    assert!(edit::unavailable(&h.model, ed(&h)).is_none());
    let lines = &ed(&h).review.as_ref().expect("a review").lines;
    assert!(lines.iter().any(|v| v.contains("fixture_nic")));
    assert!(
        lines
            .iter()
            .any(|v| v.contains("reboot is never submitted"))
    );
    assert!(h.find("network-confirm").is_some());
    amend(&mut h, |i| {
        i.details
            .network_fabric
            .pci_devices
            .values_mut()
            .next()
            .expect("a PCI device")
            .driver = None
    });
    assert!(edit::unavailable(&h.model, ed(&h)).is_some());
}

/// `network_fabric_pool_ui.rs`.
#[test]
fn exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply() {
    let mut h = Harness::scene("network-fabric-pool");
    assert_eq!(h.surface().id, FABRIC_POOL);
    assert_eq!(step(&h), 3);
    let NetworkTopologyOperation::Fabric(op) = topology_operation(&h) else {
        panic!("pool review")
    };
    assert_eq!(op.operations.len(), 2);
    assert_eq!(
        op.operations[0].review.host,
        op.operations[0].review.coordinator
    );
    assert!(
        ed(&h)
            .review
            .as_ref()
            .expect("a review")
            .lines
            .iter()
            .any(|l| l.contains("never repeats a write"))
    );
    // A pool operation is a topology action: the topology rule fences it.
    assert!(edit::unavailable(&h.model, ed(&h)).is_none());
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    edit::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// `network_fabric_pool_ui.rs`.
#[test]
fn pool_controller_and_input_changes_invalidate_captured_review() {
    for change in 0..3 {
        let mut h = Harness::scene("network-fabric-pool");
        amend(&mut h, |i| match change {
            0 => {
                let id = *i.hosts.keys().next_back().expect("a host");
                i.hosts.remove(&id);
            }
            1 => {
                i.details
                    .network_fabric
                    .controllers
                    .values_mut()
                    .next()
                    .expect("a controller")
                    .configuration = None
            }
            _ => {
                i.details
                    .pifs
                    .values_mut()
                    .find(|p| p.ip == "192.0.2.21")
                    .expect("the second input")
                    .mac = "02:00:00:00:00:ff".into()
            }
        });
        let reason = edit::unavailable(&h.model, ed(&h));
        assert!(reason.is_some(), "change {change}");
        assert_eq!(
            h.try_click("network-confirm").err(),
            reason,
            "change {change}"
        );
    }
}

/// `network_fabric_pool_ui.rs`.
#[test]
fn input_editor_requires_all_missing_hosts_and_preserves_exact_selection() {
    let h = Harness::scene("network-fabric-pool");
    let i = inventory(&h);
    let network = ed(&h).network;
    let mut e = fabric::PoolEditor {
        kind: Some(Kind::Tunnel(TunnelProtocol::Vxlan)),
        inputs: BTreeMap::new(),
    };
    assert!(fabric::reviewed(network, &e, &i).is_err());
    for p in i
        .details
        .pifs
        .values()
        .filter(|p| p.network != Some(network) && p.ip_configuration_mode == "Static")
    {
        e.inputs.insert(p.host.expect("a host"), p.id);
    }
    assert_eq!(
        fabric::reviewed(network, &e, &i)
            .expect("both hosts")
            .0
            .operations
            .len(),
        2
    );
    let host = *e.inputs.keys().next().expect("an input");
    e.inputs.remove(&host);
    assert!(fabric::reviewed(network, &e, &i).is_err());

    let mut h = Harness::scene("network-fabric-pool-select");
    assert_eq!(h.surface().id, FABRIC_POOL);
    assert_eq!(step(&h), 2);
    assert!(ed(&h).review.is_none());
    let Draft::FabricPool(chosen) = &ed(&h).draft else {
        panic!("the pool draft")
    };
    let chosen = chosen.inputs.clone();
    assert_eq!(chosen.len(), 2);
    for (host, pif) in &chosen {
        // The chosen option of the host: the Rune check mark.
        let node = h.node(&format!("network-fabric-pool-input-{host}"));
        let mac = &inventory(&h).details.pifs[pif].mac;
        assert!(
            node.value.as_deref().is_some_and(|label| label.contains(mac)),
            "{node:?}"
        );
    }
    h.click("network-review");
    assert_eq!(step(&h), 3);
    let NetworkTopologyOperation::Fabric(op) = topology_operation(&h) else {
        panic!("a pool fabric operation")
    };
    assert_eq!(
        op.operations
            .iter()
            .map(|op| (op.review.host, op.review.input.id))
            .collect::<BTreeMap<_, _>>(),
        chosen
    );
}

/// The share of `dialogs_outside_the_supported_core_carry_the_experimental_notice`
/// (`main.rs`) that ran over the scenes of this area.
#[test]
fn every_network_services_surface_carries_the_experimental_notice() {
    for scene in [
        "network-services",
        "network-topology",
        "network-fabric",
        "network-sriov",
        "network-fabric-pool",
        "network-fabric-pool-select",
    ] {
        let mut h = Harness::scene(scene);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.model.experimental_notice(), "{scene}");
        assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some(), "{scene}");
        assert!(h.find("network-preview").is_some(), "{scene}");
        assert!(h.has_text(PREVIEW), "{scene}");
        h.assert_unique_keys();
        h.dismiss();
        assert!(h.surfaces().is_empty(), "{scene}");
        assert!(!h.model.experimental_notice(), "{scene}");
    }
}

// -- 2. What the port changed -----------------------------------------------------------

#[test]
fn the_buttons_of_the_hub_are_commands_on_a_network() {
    let mut h = Harness::scene("network-services");
    h.click("network-close");
    let network = first_network(&h);
    let pool = inventory(&h).pool_id;
    let row = |h: &Harness| h.local_row().iter().map(|item| item.id).collect::<Vec<_>>();
    let label = |h: &Harness, id: CommandId| {
        h.local_row()
            .into_iter()
            .find(|item| item.id == id)
            .map(|item| item.label.to_string())
    };
    for id in [
        VLAN_CREATE_POOL,
        VLAN_RETAG_POOL,
        SERVICE_TLS,
        SERVICE_PLAINTEXT,
        FABRIC_ADD,
        FABRIC_COMPLETE_POOL,
    ] {
        assert!(row(&h).contains(&id), "{id:?}");
        assert!(
            h.context_menu(&Target::one(ObjectRef::network(pool, network.uuid())))
                .iter()
                .any(|item| item.id == id),
            "{id:?}"
        );
    }
    // Rune drew a removal button per record, and none without one.
    assert!(!row(&h).contains(&FABRIC_REMOVE));
    assert!(!row(&h).contains(&FABRIC_REMOVE_POOL));
    // The two service buttons said what they would do.
    assert_eq!(label(&h, SERVICE_TLS).as_deref(), Some("Enable NBD with TLS"));
    assert_eq!(
        label(&h, SERVICE_PLAINTEXT).as_deref(),
        Some("Enable NBD without TLS")
    );
    amend(&mut h, |i| {
        i.details.networks.get_mut(&network).expect("details").purposes =
            Some([NetworkServicePurpose::NbdTls.as_str().into()].into());
    });
    assert_eq!(label(&h, SERVICE_TLS).as_deref(), Some("Remove NBD with TLS"));
    // The interface of the scene is no VLAN interface: the Rune error of
    // `open` is the reason of the command.
    assert_eq!(
        h.eligibility(VLAN_RETAG_POOL.0),
        Eligibility::disabled("Every interface on this network must be a VLAN interface")
    );
    // On the grid of a pool the commands wait for a network; for anything
    // else they do not exist.
    h.select(Target::one(ObjectRef::pool(pool)));
    h.open_tab(TabId::Networking);
    for id in [VLAN_CREATE_POOL, SERVICE_TLS, FABRIC_ADD, FABRIC_COMPLETE_POOL, FABRIC_REMOVE] {
        assert_eq!(h.eligibility(id.0), Eligibility::disabled(SELECT_NETWORK), "{id:?}");
    }
    assert_eq!(h.eligibility(FABRIC_REMOVE_POOL.0), Eligibility::Hidden);
    for id in super::commands::ALL {
        assert_eq!(h.model.eligibility(id, &Target::none()), Eligibility::Hidden);
        let unknown = Target::one(ObjectRef::network(pool, Uuid::from_u128(0xdead)));
        assert_eq!(h.model.eligibility(id, &unknown), Eligibility::Hidden);
    }
}

#[test]
fn creation_needs_an_uplink_network_and_lists_the_candidates() {
    let mut h = Harness::scene("network-topology");
    h.click("network-close");
    assert_eq!(h.eligibility(VLAN_CREATE_POOL.0), Eligibility::Enabled);
    h.invoke(VLAN_CREATE_POOL.0);
    assert_eq!(h.surface().id, TOPOLOGY);
    assert!(h.find("network-back").is_none());
    // Nothing chosen yet: the form says what the review would refuse.
    assert!(h.find("network-topology-reason").is_some());
    h.pick("network-topology-uplink", &scenes::underlay().to_string());
    assert!(h.find("network-topology-reason").is_none());
    h.submit("network-topology-tag");
    let NetworkTopologyOperation::CreateVlans { uplinks, tag } = topology_operation(&h) else {
        panic!("a creation")
    };
    assert_eq!((uplinks.len(), tag), (inventory(&h).hosts.len(), 100));
    // Without a network that has one managed untagged interface on every
    // host no draft can be completed: the command says so.
    h.click("network-close");
    amend(&mut h, |i| i.details.pifs.clear());
    assert_eq!(
        h.eligibility(VLAN_CREATE_POOL.0),
        Eligibility::disabled(topology::NO_UPLINK)
    );
}

#[test]
fn the_fabric_wizard_walks_type_name_settings_review() {
    let mut h = Harness::scene("network-fabric");
    let Draft::Fabric(scene) = &ed(&h).draft else {
        panic!("the fabric draft")
    };
    let input = scene.input.expect("an input");
    h.click("network-close");
    assert!(h.surfaces().is_empty());
    h.invoke(FABRIC_ADD.0);
    assert_eq!(h.surface().id, FABRIC);
    assert_eq!(step(&h), 0);
    assert!(h.find("network-back").is_none());
    assert_eq!(
        h.try_click("network-next"),
        Err(fabric::CHOOSE_TYPE.into())
    );
    h.pick("network-fabric-type", "gre");
    h.click("network-next");
    assert_eq!(step(&h), 1);
    assert!(h.has_text("Tunnel and SR-IOV coverage"));
    assert!(h.has_text("Tunnel records: available"));
    h.click("network-next");
    assert_eq!(step(&h), 2);
    // Nothing chosen: the Rune reason of the review.
    h.click("network-review");
    assert_eq!(ed(&h).error.as_deref(), Some("Select an eligible interface"));
    assert_eq!(step(&h), 2);
    h.pick("network-fabric-input", &input.to_string());
    h.click("network-review");
    assert_eq!(step(&h), 3);
    assert!(h.has_text("Create host tunnel interface on this host only."));
    // Back clears the review only.
    h.click("network-back");
    assert_eq!(step(&h), 2);
    assert!(matches!(&ed(&h).draft, Draft::Fabric(f) if f.input == Some(input)));
    // Another type has other inputs.
    h.click("network-back");
    h.click("network-back");
    assert_eq!(step(&h), 0);
    h.pick("network-fabric-type", "sriov");
    assert!(matches!(
        &ed(&h).draft,
        Draft::Fabric(f) if f.kind == Some(Kind::Sriov) && f.input.is_none()
    ));
    h.dismiss();
    assert!(h.model.areas.network_services.editor.is_none());
    // The pool wizard has the same steps and the three Rune buttons as types.
    h.invoke(FABRIC_COMPLETE_POOL.0);
    assert_eq!(h.surface().id, FABRIC_POOL);
    assert_eq!(step(&h), 0);
    assert!(h.has_text("Complete VXLAN coverage"));
}

#[test]
fn removal_of_a_record_is_chosen_then_reviewed() {
    let mut h = Harness::scene("network-fabric");
    let network = ed(&h).network;
    let Draft::Fabric(scene) = &ed(&h).draft else {
        panic!("the fabric draft")
    };
    let input = scene.input.expect("an input");
    h.click("network-close");
    assert_eq!(h.eligibility(FABRIC_REMOVE.0), Eligibility::Hidden);
    let mut tunnel = None;
    amend(&mut h, |i| {
        tunnel = Some(add_tunnel(i, network, input, TunnelProtocol::Gre, 1));
    });
    let tunnel = tunnel.expect("a tunnel");
    let i = inventory(&h);
    let records = fabric::records(&i, network);
    assert_eq!(records.len(), 1);
    assert!(
        matches!(records[0].removal, Some(Ok(_))),
        "{:?}",
        records[0].removal.as_ref().map(|r| r.as_ref().err())
    );
    assert_eq!(h.eligibility(FABRIC_REMOVE.0), Eligibility::Enabled);
    h.invoke(FABRIC_REMOVE.0);
    assert_eq!(h.surface().id, REMOVE_DIALOG);
    // The only record that can be removed is chosen already.
    assert!(matches!(
        &ed(&h).draft,
        Draft::Removal(removal) if removal.chosen == Some(fabric::Object::Tunnel(tunnel))
    ));
    assert!(
        h.node("network-fabric-record")
            .value
            .is_some_and(|line| line.contains(&tunnel.to_string()))
    );
    assert!(h.has_text(&format!("Tunnel {tunnel}")));
    h.click("network-review");
    let review = ed(&h).review.as_ref().expect("a review");
    assert_eq!(review.target, ResourceTarget::Network(network));
    assert!(
        review
            .lines
            .iter()
            .any(|l| l.contains(&format!("Remove exactly object {tunnel}")))
    );
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    // The record goes: the review is fenced by the fabric rule.
    amend(&mut h, |i| {
        i.details.network_fabric.tunnels.clear();
    });
    let reason = edit::unavailable(&h.model, ed(&h)).expect("the record is gone");
    assert_eq!(h.try_click("network-confirm"), Err(reason));
    h.click("network-back");
    h.click("network-review");
    assert_eq!(ed(&h).error.as_deref(), Some(fabric::RECORD_GONE));
}

#[test]
fn pool_removal_opens_the_shared_review_with_the_topology_rule() {
    let mut h = Harness::scene("network-fabric-pool");
    let network = ed(&h).network;
    let Draft::FabricPool(scene) = &ed(&h).draft else {
        panic!("the pool draft")
    };
    let inputs = scene.inputs.values().copied().collect::<Vec<_>>();
    h.click("network-close");
    assert_eq!(h.eligibility(FABRIC_REMOVE_POOL.0), Eligibility::Hidden);
    amend(&mut h, |i| {
        for (index, input) in inputs.iter().enumerate() {
            add_tunnel(i, network, *input, TunnelProtocol::Vxlan, index as u128 + 1);
        }
    });
    let i = inventory(&h);
    let (operation, lines) = fabric::pool_removal(&i, network).expect("a pool removal");
    assert_eq!(operation.operations.len(), 2);
    assert_eq!(h.eligibility(FABRIC_REMOVE_POOL.0), Eligibility::Enabled);
    assert_eq!(
        h.local_row()
            .into_iter()
            .find(|item| item.id == FABRIC_REMOVE_POOL)
            .map(|item| item.label.to_string())
            .as_deref(),
        Some(operation.label())
    );
    h.invoke(FABRIC_REMOVE_POOL.0);
    assert_eq!(h.surface().id, CHANGE);
    assert!(h.model.areas.network_services.editor.is_none());
    let review = ws(&h).review.as_ref().expect("a review");
    assert_eq!(review.lines, lines);
    assert!(matches!(
        &review.operation,
        ResourceAction::NetworkTopology(NetworkTopologyOperation::Fabric(_))
    ));
    assert!(h.find("network-back").is_none());
    assert!(topology::review_unavailable(&h.model, ws(&h)).is_none());
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    amend(&mut h, |i| {
        i.details.network_fabric.tunnels.pop_first();
    });
    let reason = topology::review_unavailable(&h.model, ws(&h)).expect("changed coverage");
    assert_eq!(h.try_click("network-confirm"), Err(reason));
}

#[test]
fn a_review_without_a_current_connection_cannot_be_confirmed() {
    let mut h = Harness::scene("network-topology");
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    h.model.demo = false;
    assert!(h.find("network-preview").is_none());
    assert_eq!(h.try_click("network-confirm"), Err(STALE.into()));
    assert!(h.find(&format!("{REVIEW}-stale")).is_some());
    edit::submit(&mut h.model);
    assert_eq!(ed(&h).error, Some(Error::Stale.to_string()));
    assert!(h.model.resource_operations.is_empty());
    h.click("network-back");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    assert_eq!(h.try_click("network-review"), Err(NOT_WRITABLE.into()));
    // The target cannot be observed: no review.
    edit::review(&mut h.model);
    assert!(ed(&h).review.is_none());
    assert_eq!(ed(&h).error, Some(Error::Stale.to_string()));
    assert_eq!(h.eligibility(VLAN_CREATE_POOL.0), Eligibility::disabled(NOT_WRITABLE));
}

#[test]
fn one_surface_of_the_two_areas_at_a_time_and_none_after_a_scope_change() {
    // A draft of this area gives way to the shared review, and the reverse.
    let mut h = Harness::scene("network-services");
    let network = first_network(&h);
    let pool = inventory(&h).pool_id;
    assert_eq!(h.surface().id, CHANGE);
    h.invoke(FABRIC_ADD.0);
    assert!(h.model.areas.network.workspace.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, FABRIC);
    service::open(&mut h.model, pool, network, NetworkServicePurpose::NbdTls);
    assert!(h.model.areas.network_services.editor.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, CHANGE);
    h.invoke(FABRIC_COMPLETE_POOL.0);
    assert_eq!(h.surface().id, FABRIC_POOL);
    // Another object in scope: the draft belonged to the previous one.
    h.select(Target::one(ObjectRef::pool(pool)));
    assert!(h.model.areas.network_services.editor.is_none());
    assert!(h.surfaces().is_empty());
}

#[test]
fn the_information_of_the_hub_is_offered_to_the_networking_tab() {
    let h = Harness::scene("network-fabric");
    let i = inventory(&h);
    let nodes = crate::surface::flatten_blocks(
        "tab",
        &super::networking_sections(&i, first_network(&h)),
    );
    let has = |text: &str| {
        nodes
            .iter()
            .any(|n| n.text.as_deref().is_some_and(|t| t.contains(text)))
    };
    assert!(has("NBD service policy: disabled"));
    assert!(has("Controller settings: available"));
    assert!(has("No interface reported on this network; this host is not covered."));
    assert!(crate::surface::duplicate_keys(&nodes).is_empty());
}
