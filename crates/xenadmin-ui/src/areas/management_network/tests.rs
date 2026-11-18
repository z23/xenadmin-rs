//! The tests of the three Rune modules, accounted for.
//!
//! A Rune test that built a `rune::Runtime` clicked `network-confirm` at
//! two window sizes and asserted that nothing was submitted in the demo.
//! Its model half is here (the review, the refused confirm button, no
//! operation); the half about the window (the button inside it at 513x652
//! and 1440x900) is listed in `notes/progress/gpui-tests-wanted.md`.
//!
//! The Rune form stayed open behind its review, and several tests went on
//! editing it. A form is closed by its review here, so those tests open
//! the form again where Rune only cleared the review.
//!
//! `management_network_ui.rs`:
//!
//! | Rune test | Here |
//! | --- | --- |
//! | `vlan_over_bond_review_captures_full_topology_at_both_widths` | `vlan_over_bond_review_captures_full_topology` (ported; M+G) |
//! | `topology_review_tracks_membership_protection_and_metadata_but_not_link_counters` | same name (moved) |
//! | `bond_members_and_nested_vlan_targets_do_not_form_a_management_review` | same name (moved) |
//! | `automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo` | same name (ported; M+G) |
//! | `automatic_review_rejects_wrong_family_hostnames_noop_and_changed_lease` | same name (ported: the entries go through the form) |
//! | `ipv6_dhcp_and_autoconf_review_require_same_family_and_capture_stable_recovery` | same name (moved) |
//! | `management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch` | `management_review_captures_recovery_settings_without_demo_dispatch` (ported; M+G) |
//! | `management_review_refuses_changed_source_pool_ha_and_nonstandard_mtu` | same name (moved) |
//! | `management_move_review_captures_the_destination_and_retains_old_address` | same name (ported) |
//! | `management_ipv6_review_keeps_installed_family_and_rejects_invalid_recovery_address` | same name (ported: the address is typed) |
//!
//! `management_bond_ui.rs`:
//!
//! | Rune test | Here |
//! | --- | --- |
//! | `management_guest_query_result_is_fenced_to_exact_form_and_pool` | same name (moved) |
//! | `management_guest_impact_review_lists_moved_retained_and_no_eligible_guests` | same name (ported; M+G: reaching the guest lines by scrolling is the renderer's) |
//! | `management_guest_impact_becomes_stale_when_consumers_or_guest_placement_change` | same name (moved) |
//! | `management_bond_review_has_exact_members_and_recovery_at_both_widths` | `management_bond_review_has_exact_members_and_recovery` (ported; M+G) |
//! | `removal_review_captures_the_existing_primary_member_and_refuses_changed_bond` | same name (ported: through the command) |
//! | `management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths` | `management_vlan_bond_review_lists_recreated_ids_and_captures_parent` (ported; M+G) |
//! | `changed_member_or_added_consumer_invalidates_captured_bond_review` | same name (moved) |
//!
//! `management_ipv6_bond_ui.rs`:
//!
//! | Rune test | Here |
//! | --- | --- |
//! | `ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled` | `ipv6_stage_review_keeps_demo_apply_disabled` (ported; M+G) |
//! | `ipv6_review_refuses_changed_temporary_or_member_identity` | same name (moved) |
//! | `ipv6_continuation_retains_original_review_and_targets_exact_predecessor` | same name (ported: through `handovers` and `resume`) |
//!
//! Twenty Rune tests: 8 moved, 12 ported, none retired. New (9), for what
//! the port added: the commands and their places, the forms driven through
//! the harness, the refusals without a connection, the experimental notice
//! of every scene, closing.
//!
//! Not tested: an answer of the guest-impact query. The query needs a live
//! connection, and the fixture server of `crate::testing` serves neither
//! `review_management_bond` nor `review_management_ipv6_bond`; the fence of
//! the answer (`inputs_current`) and the refusals are tested.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    BondMode, Inventory, ManagementBondOperation as Bond, ManagementDynamicMode,
    ManagementInterface, ManagementNetworkOperation as Operation, OperationId, OperationState,
    PifId, PoolId, PowerState, ResourceAction, ResourceTarget,
};

use super::commands::{
    BOND_CREATE, BOND_IPV6, BOND_REMOVE, CONFIGURE, IPV6_CONTINUE, IPV6_LEAVE, MOVE,
};
use super::scenes::{self, BondFixture};
use super::view::SURFACE;
use super::{bond, ipv6, management};
use crate::Harness;
use crate::areas::network::commands::NOT_WRITABLE;
use crate::areas::network::view::{CHANGE, DEMO};
use crate::areas::network::workspace::{self, Review, Workspace};
use crate::prelude::*;

// -- Fixtures -----------------------------------------------------------------------

/// A harness over a fixture model set up by `prepare`.
fn fixture<T>(prepare: impl FnOnce(&mut Model) -> T) -> (Harness, T) {
    let mut model = Model::fixture();
    let made = prepare(&mut model);
    (Harness::with(model), made)
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("an inventory")
}

/// Replaces the inventory with an edited copy.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = h
        .model
        .inventory
        .take()
        .expect("inventory")
        .as_ref()
        .clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

fn pif(n: u128) -> PifId {
    Uuid::from_u128(n).into()
}

#[track_caller]
fn ws(h: &Harness) -> &Workspace {
    h.model
        .areas
        .network
        .workspace
        .as_ref()
        .expect("the review dialog")
}

#[track_caller]
fn review(h: &Harness) -> &Review {
    ws(h).review.as_ref().expect("a review")
}

fn reviewed(h: &Harness) -> bool {
    h.model
        .areas
        .network
        .workspace
        .as_ref()
        .is_some_and(|w| w.review.is_some())
}

#[track_caller]
fn management_op(h: &Harness) -> &Operation {
    let ResourceAction::ManagementNetwork(op) = &review(h).operation else {
        panic!("a management review")
    };
    op
}

#[track_caller]
fn ipv6_op(h: &Harness) -> &xenadmin_core::ManagementIpv6Bond {
    let ResourceAction::ManagementIpv6Bond(op) = &review(h).operation else {
        panic!("an IPv6 bond review")
    };
    op
}

fn line(h: &Harness, part: &str) -> bool {
    review(h).lines.iter().any(|v| v.contains(part))
}

fn unavailable(h: &Harness) -> Option<String> {
    management::review_unavailable(&h.model, ws(h))
}

fn error(h: &Harness) -> Option<String> {
    h.model
        .areas
        .management_network
        .editor
        .as_ref()
        .and_then(|e| e.error.clone())
}

/// The model half of every Rune "click `network-confirm` in the demo":
/// the review is the front surface, its confirm button refuses, and
/// neither the button nor the handler submits anything.
#[track_caller]
fn assert_demo_cannot_submit(h: &mut Harness) {
    assert_eq!(h.surface().id, CHANGE);
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    workspace::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(reviewed(h), "the refused review stays open");
}

/// Makes the management interface of the fixture a static IPv6 one.
fn make_ipv6(h: &mut Harness, source: PifId, clear_ipv4: bool) {
    amend(h, |i| {
        let p = i.details.pifs.get_mut(&source).unwrap();
        p.primary_address_type = Some("IPv6".into());
        p.ipv6_configuration_mode = "Static".into();
        p.ipv6 = vec!["2001:db8::10/64".into()];
        if clear_ipv4 {
            p.ip_configuration_mode = "None".into();
            p.ip.clear();
            p.netmask.clear();
            p.gateway.clear();
        }
    });
}

// -- management_network_ui.rs ------------------------------------------------------------

#[test]
fn vlan_over_bond_review_captures_full_topology() {
    let mut h = Harness::scene("management-vlan");
    let topology = management_op(&h).source().topology.as_ref().unwrap();
    assert_eq!(topology.pifs.len(), 4);
    assert_eq!(topology.bonds.len(), 1);
    assert_eq!(topology.vlans.len(), 1);
    assert!(line(&h, "bond0"));
    assert!(line(&h, "tag 17"));
    assert_demo_cannot_submit(&mut h);
}

#[test]
fn topology_review_tracks_membership_protection_and_metadata_but_not_link_counters() {
    let mut h = Harness::scene("management-vlan");
    let before = inventory(&h);
    amend(&mut h, |i| {
        i.details.bonds.values_mut().next().unwrap().links_up = Some(0);
        for p in i.details.pifs.values_mut() {
            p.capabilities = Some(vec!["runtime-capability".into()]);
        }
    });
    assert!(unavailable(&h).is_none());
    for change in ["primary", "mac", "protected", "tag", "class"] {
        h.model.inventory = Some(before.clone());
        amend(&mut h, |i| match change {
            "primary" => {
                let b = i.details.bonds.values_mut().next().unwrap();
                b.primary_slave = Some(b.slaves[1]);
            }
            "mac" => i.details.pifs.get_mut(&pif(0xf002)).unwrap().mac = "02:00:00:00:ff:ff".into(),
            "protected" => {
                i.details.pifs.get_mut(&pif(0xf002)).unwrap().disallow_unplug = Some(true)
            }
            "tag" => i.details.vlans.values_mut().next().unwrap().tag = Some(18),
            "class" => {
                i.details.classes.insert(
                    "bond".into(),
                    xenadmin_core::ClassAvailability::PermissionDenied,
                );
            }
            _ => unreachable!(),
        });
        assert!(unavailable(&h).is_some(), "{change}");
        // The reason is on the review and disables its confirm button
        // before the demo does.
        let reason = unavailable(&h).unwrap();
        assert_eq!(h.try_click("network-confirm"), Err(reason));
    }
}

#[test]
fn bond_members_and_nested_vlan_targets_do_not_form_a_management_review() {
    let mut h = Harness::scene("management-vlan");
    assert!(ManagementInterface::capture(&inventory(&h), pif(0xf002)).is_none());
    let source = management_op(&h).source().pif;
    amend(&mut h, |i| {
        let parent = pif(0xf001);
        let grandparent = pif(0xf002);
        i.details.bonds.clear();
        i.details.pifs.get_mut(&parent).unwrap().vlan = Some(18);
        let id = Uuid::from_u128(0xf202).into();
        i.details.vlans.insert(
            id,
            xenadmin_core::Vlan {
                id,
                tagged_pif: Some(grandparent),
                untagged_pif: Some(parent),
                tag: Some(18),
            },
        );
    });
    assert!(ManagementInterface::capture(&inventory(&h), source).is_none());
    // The commands say so instead of opening a form.
    h.dismiss();
    assert_eq!(
        h.eligibility(CONFIGURE.0),
        Eligibility::disabled(management::INCOMPLETE)
    );
    assert_eq!(
        h.eligibility(MOVE.0),
        Eligibility::disabled(management::INCOMPLETE)
    );
}

#[test]
fn automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo() {
    let mut h = Harness::scene("management-dhcp");
    assert!(matches!(
        management_op(&h),
        Operation::ConfigureDynamic { mode: ManagementDynamicMode::Ipv4Dhcp, recovery, .. }
            if recovery.to_string() == "192.0.2.30"
    ));
    assert!(line(&h, "does not create a reservation"));
    assert!(line(&h, "remains unresolved"));
    assert_demo_cannot_submit(&mut h);
}

#[test]
fn automatic_review_rejects_wrong_family_hostnames_noop_and_changed_lease() {
    let (mut h, (pool, source)) = fixture(|m| {
        let found = scenes::management_fixture(m).unwrap();
        scenes::open_dynamic(m, found.0, found.1);
        found
    });
    assert_eq!(h.surface().id, SURFACE);
    for value in [
        "2001:db8::30",
        "recovery.example.test",
        "192.0.2.30/24",
        "0.0.0.0",
    ] {
        h.type_text("management-recovery", value);
        // The form says why while it is edited, and the review refuses.
        assert!(h.find("management-validation").is_some(), "{value}");
        h.click("network-review");
        assert!(!reviewed(&h), "{value}");
        assert!(error(&h).is_some(), "{value}");
        assert_eq!(h.surface().id, SURFACE, "{value}");
    }
    h.type_text("management-recovery", "192.0.2.30");
    assert!(h.find("management-validation").is_none());
    h.click("network-review");
    assert!(reviewed(&h));
    assert!(h.model.areas.management_network.editor.is_none());
    amend(&mut h, |i| {
        i.details.pifs.get_mut(&source).unwrap().ip = "192.0.2.99".into()
    });
    assert!(unavailable(&h).is_some());
    // A lease that is already dynamic: the same mode is no change.
    amend(&mut h, |i| {
        i.details
            .pifs
            .get_mut(&source)
            .unwrap()
            .ip_configuration_mode = "DHCP".into()
    });
    management::open(&mut h.model, pool, source, false);
    management::editor(&mut h.model)
        .expect("the form of a DHCP management interface")
        .automatic = Some(ManagementDynamicMode::Ipv4Dhcp);
    management::prepare(&mut h.model);
    assert!(!reviewed(&h));

    let (mut h, _) = fixture(|m| {
        let (pool, source) = scenes::management_fixture(m).unwrap();
        management::open(m, pool, source, false);
    });
    h.type_text("management-dns", "192.0.2.53,2001:db8::99");
    h.click("network-review");
    assert!(
        !reviewed(&h),
        "IPv4 editing must not count ignored IPv6 DNS as a change"
    );
}

#[test]
fn ipv6_dhcp_and_autoconf_review_require_same_family_and_capture_stable_recovery() {
    for mode in [
        ManagementDynamicMode::Ipv6Dhcp,
        ManagementDynamicMode::Ipv6Autoconf,
    ] {
        let (mut h, (pool, source)) = fixture(|m| scenes::management_fixture(m).unwrap());
        make_ipv6(&mut h, source, false);
        let open = |h: &mut Harness, recovery: &str| {
            management::open(&mut h.model, pool, source, false);
            let v = management::editor(&mut h.model).unwrap();
            v.automatic = Some(mode);
            v.recovery = recovery.into();
            management::prepare(&mut h.model);
        };
        open(&mut h, "2001:db8::30");
        assert!(matches!(
            management_op(&h),
            Operation::ConfigureDynamic { mode: m, .. } if *m == mode
        ));
        assert!(line(
            &h,
            if mode == ManagementDynamicMode::Ipv6Dhcp {
                "incomplete DHCPv6"
            } else {
                "replace manually configured DNS"
            }
        ));
        open(&mut h, "fe80::30");
        assert!(!reviewed(&h));
    }
}

#[test]
fn management_review_captures_recovery_settings_without_demo_dispatch() {
    let mut h = Harness::scene("management-network");
    assert!(matches!(
        review(&h).operation,
        ResourceAction::ManagementNetwork(_)
    ));
    assert!(line(&h, "192.0.2.20"));
    assert!(line(&h, "no trust is copied"));
    assert_demo_cannot_submit(&mut h);
}

#[test]
fn management_review_refuses_changed_source_pool_ha_and_nonstandard_mtu() {
    for changed in ["ip", "pool", "ha", "mtu"] {
        let mut h = Harness::scene("management-network");
        amend(&mut h, |i| {
            let p = *i.details.pifs.keys().next().unwrap();
            match changed {
                "ip" => i.details.pifs.get_mut(&p).unwrap().ip = "192.0.2.99".into(),
                "pool" => i.pool_id = Uuid::new_v4().into(),
                "ha" => i.details.pool.as_mut().unwrap().ha_enabled = Some(true),
                "mtu" => i.details.pifs.get_mut(&p).unwrap().mtu = Some(9000),
                _ => unreachable!(),
            };
        });
        assert!(unavailable(&h).is_some(), "{changed}");
        if changed == "pool" {
            // AREAS.md decision 10: the Rune text said "reopen networking".
            assert_eq!(
                unavailable(&h).as_deref(),
                Some("The pool changed. Close and reopen this dialog.")
            );
        }
    }
}

#[test]
fn management_move_review_captures_the_destination_and_retains_old_address() {
    let (mut h, (pool, source)) = fixture(|m| scenes::management_fixture(m).unwrap());
    let mut destination = inventory(&h).details.pifs[&source].clone();
    destination.id = Uuid::new_v4().into();
    destination.network = Some(Uuid::new_v4().into());
    destination.management = Some(false);
    destination.ip = "192.0.2.30".into();
    let capture = ManagementInterface::from_inventory(&destination).unwrap();
    let id = destination.id;
    amend(&mut h, |i| {
        i.details.pifs.insert(id, destination);
    });
    management::open(&mut h.model, pool, source, true);
    assert_eq!(h.surface().title, "Move host management");
    // Nothing chosen yet: the form says what is missing.
    assert_eq!(
        h.node("management-validation").text.as_deref(),
        Some("Choose a configured destination interface")
    );
    assert!(h.find(&format!("management-destination-{id}")).is_some());
    management::editor(&mut h.model).unwrap().destination = Some(capture.clone());
    h.click("network-review");
    assert!(matches!(
        management_op(&h),
        Operation::Move { destination, .. } if destination == &capture
    ));
    assert!(line(&h, "retains its IP"));
    amend(&mut h, |i| {
        i.details.pifs.get_mut(&capture.pif).unwrap().disallow_unplug = Some(true)
    });
    assert!(unavailable(&h).is_some());
}

#[test]
fn management_ipv6_review_keeps_installed_family_and_rejects_invalid_recovery_address() {
    let (mut h, (pool, source)) = fixture(|m| scenes::management_fixture(m).unwrap());
    make_ipv6(&mut h, source, true);
    management::open(&mut h.model, pool, source, false);
    // The installed family decides the fields: no netmask for IPv6.
    assert_eq!(
        h.node("management-address").label.as_deref(),
        Some("IPv6 address/prefix")
    );
    assert!(h.find("management-netmask").is_none());
    h.type_text("management-address", "2001:db8::20/64");
    h.click("network-review");
    assert!(reviewed(&h));
    management::open(&mut h.model, pool, source, false);
    h.type_text("management-address", "fe80::20/64");
    h.click("network-review");
    assert!(!reviewed(&h));
    assert!(error(&h).is_some());
}

// -- management_bond_ui.rs ---------------------------------------------------------------

/// The Rune `management_bond::scene` before its review: the form, filled.
fn bond_form() -> (Harness, BondFixture) {
    fixture(|m| {
        let f = scenes::bond_fixture(m).unwrap();
        scenes::fill_bond(m, &f, f.source).unwrap();
        f
    })
}

#[test]
fn management_guest_query_result_is_fenced_to_exact_form_and_pool() {
    let (mut h, _) = bond_form();
    let requested = bond::operation(bond::editor(&mut h.model).unwrap()).unwrap();
    assert!(bond::inputs_current(&h.model, &requested));
    bond::editor(&mut h.model).unwrap().mode = BondMode::Lacp;
    assert!(!bond::inputs_current(&h.model, &requested));
    bond::editor(&mut h.model).unwrap().mode = BondMode::ActiveBackup;
    assert!(bond::inputs_current(&h.model, &requested));
    amend(&mut h, |i| i.pool_id = Uuid::new_v4().into());
    assert!(!bond::inputs_current(&h.model, &requested));
    h.model.areas.management_network.editor = None;
    assert!(!bond::inputs_current(&h.model, &requested));
}

#[test]
fn management_guest_impact_review_lists_moved_retained_and_no_eligible_guests() {
    let mut h = Harness::scene("management-bond-guests");
    assert!(line(&h, "Move network:"));
    assert!(line(&h, "Retain network:"));
    assert!(line(
        &h,
        "no eligible host (authoritative server response"
    ));
    assert!(line(&h, "3 reviewed interfaces"));
    assert!(
        review(&h)
            .lines
            .iter()
            .any(|v| v.starts_with("Move network:"))
    );
    assert_demo_cannot_submit(&mut h);
}

#[test]
fn management_guest_impact_becomes_stale_when_consumers_or_guest_placement_change() {
    for change in 0..3 {
        let mut h = Harness::scene("management-bond-guests");
        amend(&mut h, |i| {
            let (id, v) = i
                .details
                .vifs
                .first_key_value()
                .map(|(id, v)| (*id, v.clone()))
                .unwrap();
            match change {
                0 => {
                    i.details.vifs.remove(&id);
                }
                1 => i.details.vifs.get_mut(&id).unwrap().mac = "02:00:00:00:00:ff".into(),
                _ => i.vms.get_mut(&v.vm.unwrap()).unwrap().power_state = PowerState::Paused,
            }
        });
        assert!(unavailable(&h).is_some(), "{change}");
    }
}

#[test]
fn management_bond_review_has_exact_members_and_recovery() {
    let mut h = Harness::scene("management-bond");
    let Operation::Bond {
        operation: Bond::Create { members, .. },
        ..
    } = management_op(&h)
    else {
        panic!()
    };
    assert_eq!(members.len(), 2);
    assert!(line(&h, "192.0.2.10"));
    assert!(line(&h, "Member:"));
    assert_demo_cannot_submit(&mut h);
}

/// Puts a bond (`bond0`, `master`) over `members` into the inventory, as
/// the two Rune removal tests did.
fn add_bond(
    i: &mut Inventory,
    like: PifId,
    master: PifId,
    bond: u128,
    members: &[PifId],
    primary: PifId,
    network: xenadmin_core::NetworkId,
) {
    let mut p = i.details.pifs[&like].clone();
    p.id = master;
    p.network = Some(network);
    p.physical = Some(false);
    p.device = "bond0".into();
    i.details.pifs.insert(master, p);
    let bond = Uuid::from_u128(bond).into();
    i.details.bonds.insert(
        bond,
        xenadmin_core::NetworkBond {
            id: bond,
            master: Some(master),
            slaves: members.to_vec(),
            primary_slave: Some(primary),
            mode: "active-backup".into(),
            hashing_algorithm: None,
            hashing_algorithm_known: true,
            links_up: Some(2),
        },
    );
}

#[test]
fn removal_review_captures_the_existing_primary_member_and_refuses_changed_bond() {
    let (mut h, f) = bond_form();
    let members = [f.source, f.second];
    h.dismiss();
    amend(&mut h, |i| {
        add_bond(i, f.source, pif(0xb004), 0xb005, &members, f.source, f.target);
        for m in &members {
            let p = i.details.pifs.get_mut(m).unwrap();
            p.management = Some(false);
            p.ip_configuration_mode = "None".into();
            p.ip.clear();
            p.netmask.clear();
            p.gateway.clear();
            p.dns.clear();
        }
    });
    // The management interface of the server is the bond master now:
    // Remove is offered, Create is not.
    let ids: Vec<_> = h.local_row().into_iter().map(|item| item.id).collect();
    assert!(ids.contains(&BOND_REMOVE), "{ids:?}");
    assert!(!ids.contains(&BOND_CREATE), "{ids:?}");
    h.invoke(BOND_REMOVE.0);
    assert!(
        matches!(management_op(&h), Operation::Bond { operation: Bond::Destroy { primary, .. }, .. } if *primary == f.source)
    );
    assert!(line(&h, "network objects are retained"));
    amend(&mut h, |i| {
        i.details
            .bonds
            .get_mut(&Uuid::from_u128(0xb005).into())
            .unwrap()
            .primary_slave = Some(f.second)
    });
    assert!(unavailable(&h).is_some());
}

#[test]
fn management_vlan_bond_review_lists_recreated_ids_and_captures_parent() {
    for remove_bond in [false, true] {
        let (mut h, (f, source)) = fixture(|m| {
            let (f, source) = scenes::vlan_bond_fixture(m).unwrap();
            scenes::fill_bond(m, &f, source).unwrap();
            (f, source)
        });
        let v = bond::editor(&mut h.model).unwrap();
        let primary = v.members[0].pif;
        let members: Vec<PifId> = v.members.iter().map(|m| m.pif).collect();
        assert_eq!(v.source.pif, source);
        if remove_bond {
            amend(&mut h, |i| {
                add_bond(i, primary, pif(0xb050), 0xb051, &members, primary, f.target);
                for v in i.details.vlans.values_mut() {
                    v.tagged_pif = Some(pif(0xb050));
                }
            });
            bond::remove(&mut h.model, f.pool, source);
        } else {
            h.click("network-review");
        }
        assert_eq!(
            review(&h)
                .lines
                .iter()
                .filter(|v| v.starts_with("Recreate VLAN"))
                .count(),
            2,
            "{remove_bond}"
        );
        assert!(line(&h, "tag 17"));
        assert!(line(&h, "attached false"));
        assert_eq!(management_op(&h).source().pif, source);
        if !remove_bond {
            assert!(
                matches!(management_op(&h), Operation::Bond { operation: Bond::Create { members, .. }, .. } if members[0].pif == primary)
            );
        }
        assert_demo_cannot_submit(&mut h);
    }
}

#[test]
fn changed_member_or_added_consumer_invalidates_captured_bond_review() {
    let mut h = Harness::scene("management-bond");
    let before = inventory(&h);
    let op = management_op(&h).clone();
    for fault in ["mac", "ip", "protected", "class", "pool"] {
        h.model.inventory = Some(before.clone());
        amend(&mut h, |i| {
            let p = i.details.pifs.get_mut(&pif(0xb001)).unwrap();
            match fault {
                "mac" => p.mac = "02:00:00:00:ff:ff".into(),
                "ip" => p.ip = "192.0.2.99".into(),
                "protected" => p.disallow_unplug = Some(true),
                "class" => {
                    i.details.classes.remove("vif");
                }
                "pool" => i.pool_id = Uuid::new_v4().into(),
                _ => unreachable!(),
            };
        });
        assert!(unavailable(&h).is_some(), "{fault}");
    }
    h.model.inventory = Some(before);
    assert!(op.availability(&inventory(&h)).is_available());
}

// -- management_ipv6_bond_ui.rs ----------------------------------------------------------

#[test]
fn ipv6_stage_review_keeps_demo_apply_disabled() {
    let mut h = Harness::scene("management-ipv6-bond");
    assert!(line(&h, "stage 1 of 5"));
    assert!(line(&h, "temporary interface remains configured"));
    let op = ipv6_op(&h);
    assert_ne!(op.source.pif, op.temporary.pif);
    assert_demo_cannot_submit(&mut h);
}

#[test]
fn ipv6_review_refuses_changed_temporary_or_member_identity() {
    for temporary in [false, true] {
        let mut h = Harness::scene("management-ipv6-bond");
        let op = ipv6_op(&h);
        let id = if temporary {
            op.temporary.pif
        } else {
            let Bond::Create { members, .. } = &op.operation else {
                panic!()
            };
            members[1].pif
        };
        amend(&mut h, |i| {
            i.details.pifs.get_mut(&id).unwrap().mac = "02:00:00:00:ff:ff".into()
        });
        assert!(ipv6::unavailable(&h.model, ws(&h)).is_some());
    }
}

/// The scene `management-ipv6-bond` with the journal record of its first
/// stage, verified and waiting for the second.
fn handover() -> (Harness, PoolId, OperationId) {
    let mut h = Harness::scene("management-ipv6-bond");
    let pool = inventory(&h).pool_id;
    let action = review(&h).operation.clone();
    let host = ipv6_op(&h).source.identity.host;
    let id = OperationId::new();
    let evidence = xenadmin_core::ManagementIpv6BondEvidence {
        left_partial: false,
        completed_stages: 1,
        retains_reservation: true,
        temporary_endpoint: xenadmin_core::Endpoint::parse("https://[2001:db8:2::10]").unwrap(),
        target_endpoint: xenadmin_core::Endpoint::parse("https://[2001:db8:1::10]").unwrap(),
        predecessor: None,
        successor: None,
        created_bond: None,
        moved_vlans: vec![],
    };
    let record: xenadmin_core::ResourceOperationRecord = serde_json::from_value(serde_json::json!({"id":id,"pool":pool,"target":ResourceTarget::Host(host),"action":action,"state":OperationState::PartiallyCompleted,"progress":0.2,"detail":"Stage verified","created_at_ms":0,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"management_ipv6_bond":evidence})).unwrap();
    h.model.resource_rows.push(record);
    (h, pool, id)
}

#[test]
fn ipv6_continuation_retains_original_review_and_targets_exact_predecessor() {
    let (mut h, pool, id) = handover();
    let original = ipv6_op(&h).clone();
    let found = ipv6::handovers(&h.model, pool);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].record, found[0].stage), (id, 1));
    assert!(found[0].caption.starts_with(
        "IPv6 handover: 1/5 stages verified. Pool reserved. Temporary recovery: "
    ));
    assert!(found[0].caption.ends_with("Stage verified"));
    ipv6::resume(&mut h.model, pool, id, false);
    let next = ipv6_op(&h);
    assert_eq!(next.predecessor, Some(id));
    assert_eq!(next.source, original.source);
    assert!(line(&h, "NEXT: 2."));
    ipv6::resume(&mut h.model, pool, id, true);
    assert_eq!(ipv6_op(&h).predecessor, Some(id));
    assert!(line(&h, "No XAPI calls"));
    let r = review(&h);
    assert_eq!(
        r.operation
            .permission_requirements(r.target)
            .methods()
            .count(),
        0
    );
}

// -- New: commands, forms, notice, closing ----------------------------------------------

#[test]
fn ipv6_continuation_commands_show_only_for_an_unfinished_handover_and_need_a_connection() {
    let mut h = Harness::scene("management-ipv6-bond");
    h.dismiss();
    // No handover: neither command is anywhere.
    assert_eq!(h.eligibility(IPV6_CONTINUE.0), Eligibility::Hidden);
    assert_eq!(h.eligibility(IPV6_LEAVE.0), Eligibility::Hidden);
    let (mut h, pool, id) = handover();
    h.dismiss();
    // Rune: disabled by `!can_mutate()`, also in the demo.
    assert_eq!(
        h.eligibility(IPV6_CONTINUE.0),
        Eligibility::disabled(NOT_WRITABLE)
    );
    assert_eq!(
        h.eligibility(IPV6_LEAVE.0),
        Eligibility::disabled(NOT_WRITABLE)
    );
    let item = h
        .local_row()
        .into_iter()
        .find(|item| item.id == IPV6_CONTINUE)
        .expect("the continuation in the local row of NICs");
    assert_eq!(item.label, "Review IPv6 Handover Stage 2");
    // The tab says what is unfinished, beside those commands.
    let note = h.node(&format!("ipv6-bond-handover-note-{id}"));
    let note = note.text.as_deref().expect("a caption");
    assert!(
        note.starts_with("IPv6 handover: 1/5 stages verified. Pool reserved."),
        "{note}"
    );
    // The panel is in the dialog of a management change, with the Rune
    // keys and the same rule.
    h.invoke(BOND_IPV6.0);
    assert_eq!(h.surface().id, SURFACE);
    assert!(h.has_text("IPv6 handover: 1/5 stages verified."));
    assert_eq!(
        h.node(&format!("ipv6-bond-continue-{id}"))
            .label
            .as_deref(),
        Some("Review stage 2…")
    );
    assert_eq!(
        h.try_click(&format!("ipv6-bond-leave-{id}")),
        Err(NOT_WRITABLE.into())
    );
    assert_eq!(super::continuation(&h.model, pool).len(), 2);
    // A record that has a successor, or released the reservation, is no
    // handover any more.
    h.model.resource_rows.clear();
    assert!(ipv6::handovers(&h.model, pool).is_empty());
}

#[test]
fn commands_are_on_the_tabs_of_the_server_and_follow_the_management_interface() {
    let (mut h, f) = fixture(|m| scenes::bond_fixture(m).unwrap());
    let host = h.model.scope().clone();
    let ids = |h: &Harness| -> Vec<CommandId> {
        h.local_row().into_iter().map(|item| item.id).collect()
    };
    // NICs: the bond of the installed family only.
    let nics = ids(&h);
    assert!(nics.contains(&BOND_CREATE), "{nics:?}");
    assert!(!nics.contains(&BOND_REMOVE), "{nics:?}");
    assert!(!nics.contains(&BOND_IPV6), "{nics:?}");
    assert!(!nics.contains(&CONFIGURE), "{nics:?}");
    // Networking: the IP settings and the move.
    h.open_tab(TabId::Networking);
    let networking = ids(&h);
    assert!(networking.contains(&CONFIGURE), "{networking:?}");
    assert!(networking.contains(&MOVE), "{networking:?}");
    assert!(!networking.contains(&BOND_CREATE), "{networking:?}");
    // The context menu of the server has them all.
    let menu: Vec<CommandId> = h
        .context_menu(&host)
        .into_iter()
        .map(|item| item.id)
        .collect();
    for id in [CONFIGURE, MOVE, BOND_CREATE] {
        assert!(menu.contains(&id), "{id:?} in {menu:?}");
    }
    // IPv6 management: the staged handover replaces both bond commands.
    amend(&mut h, |i| {
        i.details
            .pifs
            .get_mut(&f.source)
            .unwrap()
            .primary_address_type = Some("IPv6".into())
    });
    assert_eq!(h.eligibility(BOND_CREATE.0), Eligibility::Hidden);
    assert_eq!(h.eligibility(BOND_REMOVE.0), Eligibility::Hidden);
    assert_ne!(h.eligibility(BOND_IPV6.0), Eligibility::Hidden);
    // A pool has no management interface of its own: the command asks for
    // the selection; a VM has none of this.
    let pool = Target::one(ObjectRef::pool(f.pool));
    assert_eq!(
        h.model.eligibility(CONFIGURE, &pool),
        Eligibility::disabled(super::commands::SELECT_MANAGEMENT)
    );
    // A network selected in the grid of the server's tab stands for the
    // server.
    let network = Target::one(ObjectRef::network(f.pool, f.target.uuid()));
    assert_eq!(h.model.eligibility(CONFIGURE, &network), Eligibility::Enabled);
    h.select(pool.clone());
    assert_eq!(
        h.model.eligibility(CONFIGURE, &pool),
        Eligibility::disabled(super::commands::SELECT_MANAGEMENT)
    );
    let inventory = inventory(&h);
    let vm = *inventory.vms.keys().next().unwrap();
    let vm = Target::one(ObjectRef::vm(f.pool, vm));
    for id in [CONFIGURE, MOVE, BOND_CREATE, BOND_REMOVE, BOND_IPV6] {
        assert_eq!(h.model.eligibility(id, &vm), Eligibility::Hidden);
    }
}

#[test]
fn the_bond_form_is_filled_through_its_controls_and_reviewed() {
    let (mut h, f) = fixture(|m| scenes::bond_fixture(m).unwrap());
    h.invoke(BOND_CREATE.0);
    let surface = h.surface();
    assert_eq!(surface.id, SURFACE);
    assert_eq!(surface.title, "Create management bond");
    assert!(surface.experimental);
    h.assert_unique_keys();
    // The management parent is a member and stays one.
    let parent = h.node(&format!("management-bond-member-{}", f.source));
    assert_eq!(parent.checked, Some(true));
    assert!(parent.disabled.is_some());
    // Nothing chosen: the review says what is missing and the form stays.
    h.click("network-review");
    assert_eq!(
        error(&h).as_deref(),
        Some("Choose an empty destination network")
    );
    h.toggle(&format!("management-bond-member-{}", f.second));
    h.pick("management-bond-network", &f.target.to_string());
    h.pick("management-bond-mode", "lacp");
    assert!(h.find("management-bond-hash").is_some());
    h.pick("management-bond-hash", "tcpudp_ports");
    h.click("network-review");
    let Operation::Bond {
        operation:
            Bond::Create {
                members,
                network,
                mode,
                hash,
            },
        ..
    } = management_op(&h)
    else {
        panic!()
    };
    assert_eq!(
        members.iter().map(|m| m.pif).collect::<Vec<_>>(),
        [f.source, f.second]
    );
    assert_eq!(*network, f.target);
    assert_eq!(*mode, BondMode::Lacp);
    assert_eq!(*hash, Some(xenadmin_core::BondHash::TcpUdpPorts));
    // The hand-over closed the form; the review is the network area's.
    assert!(h.model.areas.management_network.editor.is_none());
    assert_eq!(h.surface().id, CHANGE);
    assert!(h.surface().experimental);
}

#[test]
fn the_ipv6_form_is_filled_through_its_controls_and_reviewed() {
    let (mut h, _) = fixture(|m| scenes::open_ipv6(m).unwrap());
    assert_eq!(h.surface().title, "Staged IPv6 management bond");
    h.assert_unique_keys();
    let temporary = pif(0x6b001);
    assert!(h.find(&format!("ipv6-bond-temporary-{temporary}")).is_some());
    // Without the temporary interface there is no stage to review.
    ipv6::editor(&mut h.model).unwrap().temporary = None;
    h.click("network-review");
    assert_eq!(
        error(&h).as_deref(),
        Some("Choose the independent static IPv6 recovery interface")
    );
    h.pick("ipv6-bond-temporary", &temporary.to_string());
    h.pick("ipv6-bond-mode", "lacp");
    h.click("network-review");
    let op = ipv6_op(&h);
    assert_eq!(op.temporary.pif, temporary);
    assert!(matches!(
        &op.operation,
        Bond::Create { mode: BondMode::Lacp, hash: Some(xenadmin_core::BondHash::SourceMac), .. }
    ));
    assert!(line(&h, "NEXT: 1."));
}

#[test]
fn the_addressing_choice_switches_between_static_and_the_modes_of_the_installed_family() {
    let (mut h, _) = fixture(|m| {
        let (pool, source) = scenes::management_fixture(m).unwrap();
        management::open(m, pool, source, false);
    });
    assert_eq!(h.surface().title, "Management IP settings");
    assert!(h.surface().experimental);
    h.assert_unique_keys();
    assert!(h.find("management-address").is_some());
    assert!(h.find("management-netmask").is_some());
    assert!(h.find("management-recovery").is_none());
    h.pick("management-mode", "ipv4-dhcp");
    assert_eq!(
        management::editor(&mut h.model).unwrap().automatic,
        Some(ManagementDynamicMode::Ipv4Dhcp)
    );
    assert!(h.find("management-address").is_none());
    assert_eq!(
        h.node("management-recovery").value.as_deref(),
        Some("192.0.2.10")
    );
    h.pick("management-mode", "static");
    assert_eq!(management::editor(&mut h.model).unwrap().automatic, None);
}

#[test]
fn every_scene_ends_on_an_experimental_review() {
    // The share of the Rune `experimental_dialog_open` test: the
    // networking dialog always carried the notice.
    for spec in scenes::scenes() {
        let h = Harness::scene(spec.name);
        let surface = h.surface();
        assert_eq!(surface.id, CHANGE, "{}", spec.name);
        assert!(surface.experimental, "{}", spec.name);
        assert!(reviewed(&h), "{}", spec.name);
        assert!(
            h.model.areas.management_network.editor.is_none(),
            "{}",
            spec.name
        );
        h.assert_unique_keys();
    }
}

#[test]
fn the_form_closes_on_escape_and_with_its_scope() {
    let (mut h, f) = bond_form();
    assert_eq!(h.surface().id, SURFACE);
    h.dismiss();
    assert!(h.model.areas.management_network.editor.is_none());
    scenes::fill_bond(&mut h.model, &f, f.source).unwrap();
    assert!(h.model.areas.management_network.editor.is_some());
    h.select(Target::one(ObjectRef::pool(f.pool)));
    assert!(h.model.areas.management_network.editor.is_none());
}

#[test]
fn a_quiet_model_has_nothing_to_poll() {
    let (mut h, _) = bond_form();
    assert!(!bond::poll(&mut h.model));
    assert!(!ipv6::poll(&mut h.model));
}

#[test]
fn outside_the_demo_a_review_needs_a_connection_and_the_form_stays() {
    // Bond: the guest impact cannot be asked for.
    let (mut h, f) = bond_form();
    h.model.demo = false;
    assert_eq!(h.try_click("network-review"), Err(NOT_WRITABLE.into()));
    bond::prepare(&mut h.model);
    assert_eq!(
        error(&h).as_deref(),
        Some("Connect before reviewing guest interfaces")
    );
    let e = h.model.areas.management_network.editor.as_ref().unwrap();
    assert!(e.pending.is_none());
    assert!(!reviewed(&h));
    // IP settings: the review is complete, but its target cannot be
    // observed (the Rune `prepare_action`): the error is on the form.
    management::open(&mut h.model, f.pool, f.source, false);
    management::editor(&mut h.model).unwrap().address = "192.0.2.20".into();
    management::prepare(&mut h.model);
    assert_eq!(
        error(&h),
        Some(xenadmin_core::Error::Stale.to_string())
    );
    assert_eq!(h.surface().id, SURFACE);
    assert!(!reviewed(&h));
    // The staged IPv6 bond likewise.
    let (mut h, _) = fixture(|m| scenes::open_ipv6(m).unwrap());
    h.model.demo = false;
    ipv6::prepare(&mut h.model);
    assert_eq!(
        error(&h),
        Some(xenadmin_core::Error::Stale.to_string())
    );
    assert!(!reviewed(&h));
}

// -- Back from a handed-over review ------------------------------------------------------

/// Rune: "Back" of the review cleared the review and showed the form again.
#[test]
fn back_from_a_handed_over_review_shows_the_form_with_what_was_entered() {
    let mut h = Harness::scene("management-network");
    assert_eq!(h.surface().id, CHANGE);
    assert_eq!(h.surfaces().len(), 1, "the form is not on screen");
    assert!(h.model.areas.management_network.editor.is_none());
    assert!(h.model.areas.management_network.reviewed.is_some());
    let operation = review(&h).operation.clone();

    h.click("network-back");
    assert_eq!(h.surface().id, SURFACE);
    assert_eq!(h.surfaces().len(), 1);
    assert!(h.model.areas.network.workspace.is_none());
    assert!(h.model.areas.management_network.reviewed.is_none());
    assert_eq!(h.node("management-address").value.as_deref(), Some("192.0.2.20"));
    assert_eq!(h.node("management-gateway").value.as_deref(), Some("192.0.2.1"));
    assert!(error(&h).is_none());

    // The same entries give the same review; a changed entry a changed one.
    h.click("network-review");
    assert_eq!(h.surface().id, CHANGE);
    assert_eq!(review(&h).operation, operation);
    h.click("network-back");
    h.type_text("management-address", "192.0.2.21");
    h.click("network-review");
    assert!(line(&h, "192.0.2.21"));
    assert_ne!(review(&h).operation, operation);

    // Closing the review drops the form that was kept for its Back.
    h.click("network-close");
    assert!(h.surfaces().is_empty());
    assert!(!h.tick(0.1), "nothing shown changed");
    assert!(h.model.areas.management_network.reviewed.is_none());
    assert!(h.model.areas.management_network.editor.is_none());
}

/// A bond review comes after the guest-impact query: Back still leads to
/// the form, without the finished query.
#[test]
fn back_from_a_bond_review_shows_the_bond_form() {
    let mut h = Harness::scene("management-bond");
    assert_eq!(h.surface().id, CHANGE);
    h.click("network-back");
    assert_eq!(h.surface().id, SURFACE);
    assert_eq!(h.surface().title, "Create management bond");
    let editor = h.model.areas.management_network.editor.as_ref().unwrap();
    assert!(editor.pending.is_none());
    assert!(matches!(editor.form, super::Form::Bond(_)));
    assert!(h.find("management-pending").is_none());
}

/// The removal of a bond has nothing to enter: its review has no Back, as
/// a review that `network` opens from a command.
#[test]
fn a_review_without_a_form_has_no_back() {
    let (mut h, f) = bond_form();
    let members = [f.source, f.second];
    h.dismiss();
    amend(&mut h, |i| {
        add_bond(i, f.source, pif(0xb004), 0xb005, &members, f.source, f.target);
        for m in &members {
            let p = i.details.pifs.get_mut(m).unwrap();
            p.management = Some(false);
            p.ip_configuration_mode = "None".into();
            p.ip.clear();
            p.netmask.clear();
            p.gateway.clear();
            p.dns.clear();
        }
    });
    h.invoke(BOND_REMOVE.0);
    assert!(reviewed(&h));
    assert_eq!(h.surface().id, CHANGE);
    assert!(h.find("network-back").is_none());
    assert!(h.model.areas.management_network.reviewed.is_none());
    assert_eq!(crate::areas::network::handed_over_by(&h.model), None);
}
