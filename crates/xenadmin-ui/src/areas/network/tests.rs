//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests** of `network_admin.rs` (2), `network_interfaces.rs`
//!    (3) and of the generic editor in `resource_admin.rs` (3 of its 5),
//!    under their Rune names, and this area's share of two tests of other
//!    Rune files that ran over its scenes. Where the Rune test found a
//!    control by its key and pressed it, the test here does the same through
//!    the harness; "inside the window at both sizes" is the front end's to
//!    show (`notes/progress/gpui-tests-wanted.md`).
//! 2. **What the port changed**: the hub became commands, grids and pages.
//! 3. **Hazards** of the map (section 11) that no Rune test pinned.
//!
//! Not here, with the reason:
//!
//! - `joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations`
//!   and the second half of
//!   `used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens`
//!   (`resource_admin.rs`): the history and the acknowledgement are the
//!   `events` area's, which ported them.
//! - `compact_network_controls_remain_inside_the_window`
//!   (`resource_admin.rs`): retired. It measured where the buttons of the
//!   Rune page header (`manage-pool`, `create-network`, `add-connection`,
//!   `show-details`, `disconnect`) sat at two window sizes; that header does
//!   not exist any more.
//!
//! No fixture has a current snapshot, so no test submits successfully: the
//! refusals are tested.

use std::sync::Arc;

use uuid::Uuid;
use xenadmin_core::{
    BondId, BondMode, Error, Inventory, Ipv4Mode, Ipv6Mode, NetworkBond, NetworkId,
    NetworkOperation, PifId, PoolId, ResourceAction, ResourceTarget, SrId, Vlan, VlanId,
};

use super::commands::{
    ALREADY_DEFAULT, BOND_DELETE, DEFAULT_UNKNOWN, INTERFACE_MENU, NETWORK_USED, NEW, NO_IPV6,
    NOT_WRITABLE, REMOVE, SELECT_BOND, SELECT_INTERFACE, SELECT_NETWORK, SELECT_REPOSITORY,
    SET_DEFAULT, STORAGE_RESCAN, VLAN_ADD,
};
use super::editor::{self, Editor, Kind};
use super::view::{CHANGE, DEMO, PREVIEW, WIZARD};
use super::workspace::{self, Check, Form, STALE, Workspace};
use super::{scenes, selected_pif};
use crate::Harness;
use crate::prelude::*;

// -- Fixtures -----------------------------------------------------------------------

#[track_caller]
fn ws(h: &Harness) -> &Workspace {
    h.model.areas.network.workspace.as_ref().expect("a workspace")
}

#[track_caller]
fn ed(h: &Harness) -> &Editor {
    h.model.areas.network.editor.as_ref().expect("an editor")
}

fn inventory(h: &Harness) -> Arc<Inventory> {
    h.model.inventory.clone().expect("an inventory")
}

fn pool(h: &Harness) -> PoolId {
    inventory(h).pool_id
}

/// Replaces the inventory with an edited copy.
fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    let mut inventory = h.model.inventory.take().expect("inventory").as_ref().clone();
    change(&mut inventory);
    h.model.inventory = Some(Arc::new(inventory));
}

/// The two interfaces the scene `network-interfaces` adds.
fn uplink(index: u128) -> PifId {
    PifId::from(Uuid::from_u128(0x52000000000000000000000000000001 + index))
}

/// The scene inventory with no dialog open, the first server selected and
/// one of its tabs showing.
fn server_tab(tab: TabId) -> Harness {
    let mut h = Harness::scene("network-interfaces");
    h.dismiss();
    let i = inventory(&h);
    let host = *i.hosts.keys().next().expect("a host");
    h.select(Target::one(ObjectRef::host(i.pool_id, host)));
    h.open_tab(tab);
    h
}

/// A network of the demo that a VM interface is on, and one added without
/// any interface.
fn used_and_unused(h: &mut Harness) -> (NetworkId, NetworkId) {
    let used = inventory(h)
        .details
        .vifs
        .values()
        .find_map(|vif| vif.network)
        .expect("a network with a VM interface");
    let unused = NetworkId::from(Uuid::from_u128(0x53));
    amend(h, |i| {
        i.networks.push(xenadmin_core::Network {
            uuid: unused.uuid(),
            name: "Lab internal".into(),
            bridge: "xapi9".into(),
        });
        let mut details = i.details.networks[&used].clone();
        details.id = unused;
        i.details.networks.insert(unused, details);
    });
    (used, unused)
}

fn local_ids(h: &Harness) -> Vec<&'static str> {
    h.local_row().into_iter().map(|item| item.id.0).collect()
}

// -- 1. The Rune tests ----------------------------------------------------------------

/// `network_admin.rs`. The two window sizes are the front end's.
#[test]
fn compact_and_wide_network_reviews_remain_visible_and_preview_cannot_submit() {
    let mut h = Harness::scene("network-interfaces");
    workspace::set_form(
        &mut h.model,
        Form::Mtu {
            value: "9000".into(),
        },
    );
    h.click("network-review");
    assert!(matches!(
        ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetMtu { mtu: 9000, .. })
    ));
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    workspace::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert!(ws(&h).review.is_some());
    assert_eq!(ws(&h).error, Some(Error::Stale.to_string()));
}

/// `network_admin.rs`.
#[test]
fn mtu_review_captures_original_value_instead_of_chasing_inventory() {
    let mut h = Harness::scene("network-interfaces");
    workspace::set_form(
        &mut h.model,
        Form::Mtu {
            value: "9000".into(),
        },
    );
    workspace::form_review(&mut h.model);
    assert!(matches!(
        ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetMtu {
            expected: 1500,
            mtu: 9000,
            ..
        })
    ));
    let network = ws(&h).network.expect("network");
    amend(&mut h, |i| {
        i.details.networks.get_mut(&network).expect("details").mtu = Some(8000);
    });
    assert!(matches!(
        ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetMtu { expected: 1500, .. })
    ));
}

/// `network_interfaces.rs`. The two window sizes are the front end's.
#[test]
fn ipv6_review_is_visible_at_both_widths_and_keeps_captured_identity() {
    let mut h = Harness::scene("network-ipv6");
    let id = uplink(0);
    h.click("network-review");
    let review = ws(&h).review.as_ref().expect("review");
    assert_eq!(review.target, ResourceTarget::Pif(id));
    assert!(matches!(
        &review.operation,
        ResourceAction::Network(NetworkOperation::ConfigureIpv6 { configuration, .. })
            if configuration.addresses == ["2001:db8::20/64"]
    ));
    assert!(review.lines.iter().any(|s| s.contains("DNS is shared")));
    assert!(h.has_text("DNS is shared"));
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    workspace::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
}

/// `network_interfaces.rs`.
#[test]
fn interface_review_refuses_changed_identity_configuration_and_pool() {
    for change in 0..3 {
        let mut h = Harness::scene("network-ipv6");
        let id = uplink(0);
        amend(&mut h, |i| match change {
            0 => i.details.pifs.get_mut(&id).expect("pif").management = Some(true),
            1 => i.details.pifs.get_mut(&id).expect("pif").ipv6 = vec!["2001:db8::99/64".into()],
            _ => i.pool_id = PoolId::from(Uuid::from_u128(992)),
        });
        workspace::form_review(&mut h.model);
        assert!(ws(&h).review.is_none(), "change {change}");
        assert!(ws(&h).error.is_some(), "change {change}");
    }
}

/// `network_interfaces.rs`.
#[test]
fn purpose_label_is_separate_from_unplug_protection_and_invalid_ipv6_is_refused() {
    let mut h = Harness::scene("network-ipv6");
    let id = uplink(0);
    assert!(scenes::open_interface(&mut h.model, id, true));
    h.type_text("network-purpose", "Storage fabric");
    h.click("network-review");
    assert!(matches!(
        &ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetInterfacePurpose {
            before: None,
            purpose: Some(v),
            ..
        }) if v == "Storage fabric"
    ));
    assert_eq!(inventory(&h).details.pifs[&id].disallow_unplug, Some(false));
    assert!(scenes::open_interface(&mut h.model, id, false));
    h.pick("network-ipv6-mode", Ipv6Mode::Static.as_str());
    h.type_text("network-ipv6-address", "2001:db8::20/129");
    h.click("network-review");
    assert!(ws(&h).review.is_none());
    assert!(ws(&h).error.is_some());
}

/// `resource_admin.rs`. The dialog is the wizard New Network now; the two
/// window sizes are the front end's.
#[test]
fn network_review_captures_pool_name_mtu_and_never_submits_from_preview() {
    let mut h = Harness::scene("pool-administration");
    let pool = pool(&h);
    h.click("network.new");
    assert_eq!(h.surface().id, WIZARD);
    h.click("resource-next");
    h.type_text("resource-name", "Isolated test network");
    h.type_text("resource-description", "Synthetic network");
    h.click("resource-next");
    h.type_text("resource-mtu", "9000");
    h.click("resource-review");
    let editor = ed(&h);
    let reviewed = editor.review.as_ref().expect("review");
    assert_eq!(reviewed.name, "Isolated test network");
    assert_eq!(reviewed.mtu, 9000);
    assert_eq!(editor.target, ResourceTarget::Pool(pool));
    assert!(!editor::current(&h.model, editor));
    assert_eq!(h.try_click("resource-confirm"), Err(DEMO.into()));
    editor::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(ed(&h).error, Some(Error::Stale.to_string()));
}

/// `resource_admin.rs`. Rune kept the editor's target while the selection
/// behind the dialog changed; here a change of the tree selection closes
/// the editor (a dialog is modal, and the kernel closes a Properties
/// dialog with its object), so the target cannot be retargeted either.
#[test]
fn resource_review_keeps_original_selection_and_rejects_invalid_or_unchanged_values() {
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    let network = i.networks[0].uuid;
    h.select(Target::one(ObjectRef::network(i.pool_id, network)));
    h.invoke("network.properties");
    assert_eq!(h.surface().id, "network-properties");
    h.click("resource-review");
    assert_eq!(
        ed(&h).error.as_deref(),
        Some("No name or description changes to apply.")
    );
    let target = ed(&h).target;
    assert_eq!(target, ResourceTarget::Network(NetworkId::from(network)));
    h.type_text("resource-name", "Updated network");
    h.click("resource-review");
    assert!(ed(&h).review.is_some());
    assert_eq!(ed(&h).target, target);
    h.select(Target::one(ObjectRef::pool(i.pool_id)));
    assert!(h.model.areas.network.editor.is_none());
    assert!(h.model.properties.is_none());

    h.open_tab(TabId::Networking);
    h.click("network.new");
    for value in ["", "0", "1500.5", "65536", "text"] {
        let editor = h.model.areas.network.editor.as_mut().expect("editor");
        editor.name = "Network".into();
        editor.mtu = value.into();
        editor::review(&mut h.model);
        assert!(ed(&h).review.is_none(), "{value:?}");
        assert_eq!(
            ed(&h).error.as_deref(),
            Some("Enter an MTU between 576 and 65535")
        );
    }
}

/// `resource_admin.rs`, its first half: a network with interfaces cannot
/// be deleted, and says why. (Its MTU can be changed now: the Network
/// Settings page reviews the change the networking dialog had for such a
/// network, see `the_mtu_field_reviews_the_change_that_fits_the_network`.)
#[test]
fn used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens() {
    let mut h = Harness::scene("pool-administration");
    let (used, unused) = used_and_unused(&mut h);
    h.select_row("networking-networks-table", &used.uuid().to_string());
    assert_eq!(
        h.eligibility("network.remove"),
        Eligibility::disabled(NETWORK_USED)
    );
    assert_eq!(h.try_click("network.remove"), Err(NETWORK_USED.into()));
    assert!(h.model.areas.network.editor.is_none());

    // Without interfaces the command opens on its review, with no Back.
    h.select_row("networking-networks-table", &unused.uuid().to_string());
    h.click("network.remove");
    assert_eq!(h.surface().id, "network-remove");
    assert_eq!(h.surface().title, "Delete internal network");
    assert_eq!(ed(&h).target, ResourceTarget::Network(unused));
    assert!(h.has_text(
        "The internal network record will be deleted. The host must report no physical or VM interfaces on it."
    ));
    assert!(h.find("resource-back").is_none());
    assert_eq!(h.try_click("resource-confirm"), Err(DEMO.into()));
}

/// The share of `dialogs_outside_the_supported_core_carry_the_experimental_notice`
/// (`main.rs`): both Rune editors of this area had the notice.
#[test]
fn every_network_surface_carries_the_experimental_notice() {
    for scene in ["network-interfaces", "network-ipv6", "network-create"] {
        let h = Harness::scene(scene);
        assert!(h.surface().experimental, "{scene}");
        assert!(h.model.experimental_notice(), "{scene}");
    }
    // The hub scene has no surface, and a tab has no notice.
    let mut h = Harness::scene("pool-administration");
    assert!(h.surfaces().is_empty());
    assert!(!h.model.experimental_notice());
    // The pages.
    let i = inventory(&h);
    h.invoke("pool.properties");
    assert!(h.surface().experimental, "General of a pool");
    h.pick("pool-properties-pages", "network-options");
    assert!(h.surface().experimental, "Network Options");
    h.dismiss();
    h.select(Target::one(ObjectRef::network(i.pool_id, i.networks[0].uuid)));
    h.invoke("network.properties");
    h.pick("network-properties-pages", "network-settings");
    assert!(h.surface().experimental, "Network Settings");
}

/// The share of
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`
/// (`session_permissions_view.rs`, over `resource_admin::scene(s, false)`).
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    let mut h = Harness::scene("network-create");
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    assert_eq!(h.try_click("resource-confirm"), Err(DEMO.into()));
    h.click("resource-back");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    // The review of a network change has the line too.
    let mut h = Harness::scene("network-ipv6");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
    h.click("network-review");
    assert!(h.find(permissions::REVIEW_KEY).is_some());
}

// -- 2. What the port changed -----------------------------------------------------------

#[test]
fn the_scenes_show_what_the_rune_scenes_showed() {
    let h = Harness::scene("network-interfaces");
    assert_eq!(h.surface().id, CHANGE);
    assert_eq!(h.surface().title, "Add VLAN interface");
    assert_eq!(h.node("network-vlan-tag").value.as_deref(), Some("100"));
    let first = inventory(&h).networks[0].clone();
    assert_eq!(
        h.surface().subtitle,
        Some(format!(
            "Network: {} ({})",
            first.name,
            NetworkId::from(first.uuid)
        ))
    );
    // The two interfaces of the other network are the parents on offer.
    assert!(matches!(&ws(&h).form, Form::Vlan { parent: None, .. }));
    let h = Harness::scene("network-ipv6");
    assert_eq!(h.surface().title, "IPv6 configuration");
    assert_eq!(
        h.node("network-ipv6-address").value.as_deref(),
        Some("2001:db8::20/64")
    );
    let h = Harness::scene("network-create");
    assert_eq!(h.surface().id, WIZARD);
    assert!(matches!(
        h.surface().kind,
        SurfaceKind::Wizard { current: 3, .. }
    ));
    assert!(h.has_text("Name: Isolated lab network"));
    assert!(h.has_text("MTU: 1500"));
    assert!(h.has_text(PREVIEW));
    let h = Harness::scene("pool-administration");
    assert!(h.find("networking-networks-table").is_some());
}

#[test]
fn a_vlan_interface_is_reviewed_from_the_form() {
    let mut h = Harness::scene("network-interfaces");
    h.click("network-review");
    assert_eq!(
        ws(&h).error.as_deref(),
        Some("Select a physical interface")
    );
    h.pick("network-vlan-parent", &uplink(1).to_string());
    h.type_text("network-vlan-tag", "4095");
    h.click("network-review");
    assert_eq!(
        ws(&h).error.as_deref(),
        Some("Enter a VLAN tag from 0 to 4094")
    );
    h.type_text("network-vlan-tag", "17");
    h.click("network-review");
    assert!(matches!(
        ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::CreateVlan { tag: 17, parent, .. })
            if parent == uplink(1)
    ));
    assert!(h.has_text("VLAN tag: 17"));
    assert!(h.has_text(
        "Creates an interface on this host. Repeat on other hosts that need this network."
    ));
    // Back returns to the form as it was.
    h.click("network-back");
    assert_eq!(h.node("network-vlan-tag").value.as_deref(), Some("17"));
}

#[test]
fn the_local_rows_have_the_commands_of_the_plan_and_hide_them_elsewhere() {
    let mut h = Harness::scene("pool-administration");
    h.assert_unique_keys();
    let row = local_ids(&h);
    for id in [
        "network.new",
        "network.vlan.add",
        "network.remove",
        "nic.ipv4",
        "nic.ipv6",
        "nic.purpose",
    ] {
        assert!(row.contains(&id), "{id} in {row:?}");
    }
    // No network and no interface is selected yet.
    assert_eq!(h.eligibility("network.new"), Eligibility::Enabled);
    assert_eq!(
        h.eligibility("network.vlan.add"),
        Eligibility::disabled(SELECT_NETWORK)
    );
    assert_eq!(
        h.eligibility("network.remove"),
        Eligibility::disabled(SELECT_NETWORK)
    );
    assert_eq!(
        h.eligibility("nic.ipv4"),
        Eligibility::disabled(SELECT_INTERFACE)
    );
    // A VM's Networking tab is another area's: nothing of this one shows.
    let i = inventory(&h);
    let vm = *i.vms.keys().next().expect("a VM");
    h.select(Target::one(ObjectRef::vm(i.pool_id, vm)));
    h.open_tab(TabId::Networking);
    let row = local_ids(&h);
    assert!(
        row.iter()
            .all(|id| !id.starts_with("network.") && !id.starts_with("nic.")),
        "{row:?}"
    );
    assert!(
        super::tab::tab(&h.model, TabId::Networking, h.model.scope()).is_none(),
        "a VM's Networking tab is not this area's"
    );
    // A connection that cannot write: the Rune `enabled`.
    let mut h = Harness::scene("pool-administration");
    h.model.demo = false;
    assert_eq!(
        h.eligibility("network.new"),
        Eligibility::disabled(NOT_WRITABLE)
    );
}

#[test]
fn the_tabs_keep_the_keys_of_the_fallbacks_and_have_unique_keys() {
    let mut h = server_tab(TabId::Nics);
    h.assert_unique_keys();
    for key in ["nics-table", "nics-interfaces", "network-operation-history"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert!(h.find(&format!("pif-{}", uplink(0))).is_some());
    h.open_tab(TabId::Networking);
    h.assert_unique_keys();
    for key in ["networking-networks-table", "networking-uplinks"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert!(h.has_text("Physical uplinks"));
    let i = inventory(&h);
    let uplinks = i.networks.last().expect("network").uuid;
    h.select(Target::one(ObjectRef::network(i.pool_id, uplinks)));
    h.open_tab(TabId::Networking);
    h.assert_unique_keys();
    for key in ["network-uplinks-table", "networking-vifs-table"] {
        assert!(h.find(key).is_some(), "{key}");
    }
    assert!(h.has_text("Host uplinks"));
    assert!(h.has_text("VM interfaces"));
}

#[test]
fn an_interface_is_selected_in_the_grid_and_its_commands_are_its_row_menu() {
    let mut h = server_tab(TabId::Nics);
    let row = local_ids(&h);
    for id in ["server.bond.create", "server.bond.delete", "server.nics.rescan"] {
        assert!(row.contains(&id), "{id} in {row:?}");
    }
    assert_eq!(
        h.eligibility("server.bond.delete"),
        Eligibility::disabled(SELECT_BOND)
    );
    assert!(selected_pif(&h.model).is_none());
    let id = uplink(0);
    let items = h.open_row_menu("nics-table", &id.to_string());
    assert_eq!(selected_pif(&h.model).map(|found| found.2), Some(id));
    // The Rune card of the interface is under the grid.
    assert!(h.find("network-interface-card").is_some());
    assert!(h.has_text("Purpose: Not set · unplug protection: disabled"));
    assert!(h.has_text(NO_IPV6));
    let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Unplug Interface",
            "Configure IPv4",
            "Configure IPv6",
            "Interface Purpose",
            "Protect from Unplugging",
            // No bond and no VLAN on it: mode, hash and VLAN are hidden; the
            // plan's Delete Bond stays, disabled.
            "Delete Bond",
        ]
    );
    assert_eq!(h.eligibility("nic.ipv6"), Eligibility::disabled(NO_IPV6));
    assert_eq!(
        h.eligibility("server.bond.delete"),
        Eligibility::disabled(SELECT_BOND)
    );
    assert!(INTERFACE_MENU.contains(&BOND_DELETE));

    // A change with nothing to enter opens on its review and has no Back.
    h.invoke("nic.plug");
    assert_eq!(h.surface().id, CHANGE);
    assert!(h.has_text("Attached: true → false"));
    assert!(h.has_text(
        "Unplugging interrupts VM and storage traffic on this interface."
    ));
    assert!(h.find("network-back").is_none());
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    h.dismiss();
    h.invoke("nic.unplug-protection");
    assert!(h.has_text("Unplug protection: false → true"));
    h.dismiss();
    h.click("server.nics.rescan");
    assert!(h.has_text("Refresh physical interface discovery on this host."));
    assert!(matches!(
        ws(&h).review.as_ref().expect("review").target,
        ResourceTarget::Host(_)
    ));
    h.dismiss();
    // Another object: the selection does not follow.
    h.select(Target::one(ObjectRef::pool(pool(&h))));
    assert!(h.model.areas.network.pif.is_none());
}

#[test]
fn ipv4_of_a_secondary_interface_is_configured_from_the_networking_tab() {
    let mut h = server_tab(TabId::Networking);
    h.select_row("networking-uplinks", &uplink(1).to_string());
    h.click("nic.ipv4");
    assert_eq!(h.surface().title, "IPv4 configuration");
    assert!(h.find("network-ipv4-address").is_none(), "only for Static");
    h.click("network-review");
    assert_eq!(
        ws(&h).error.as_deref(),
        Some(
            "Enter valid changed IP settings, a contiguous netmask and comma-separated DNS addresses"
        )
    );
    h.pick("network-ipv4-mode", Ipv4Mode::Static.as_str());
    h.type_text("network-ipv4-address", "192.0.2.40");
    h.type_text("network-ipv4-netmask", "255.255.255.0");
    h.click("network-review");
    let review = ws(&h).review.as_ref().expect("review");
    assert_eq!(review.target, ResourceTarget::Pif(uplink(1)));
    assert!(matches!(
        &review.operation,
        ResourceAction::Network(NetworkOperation::ConfigureIpv4 { configuration, .. })
            if configuration.address == "192.0.2.40"
    ));
    assert!(h.has_text(
        "Storage or other traffic using this interface may be interrupted. This does not change the host management interface."
    ));
    // The management interface is not changed here.
    h.dismiss();
    amend(&mut h, |i| {
        i.details.pifs.get_mut(&uplink(1)).expect("pif").management = Some(true);
    });
    assert!(matches!(h.eligibility("nic.ipv4"), Eligibility::Disabled(_)));
    assert!(matches!(
        h.eligibility("nic.purpose"),
        Eligibility::Disabled(_)
    ));
}

#[test]
fn a_bond_is_created_from_the_nics_tab_with_its_destination_chosen_in_the_form() {
    let mut h = server_tab(TabId::Nics);
    h.click("server.bond.create");
    assert_eq!(h.surface().title, "Create bond");
    h.click("network-review");
    assert_eq!(
        ws(&h).error.as_deref(),
        Some("Select two to four physical interfaces on one host")
    );
    h.toggle(&format!("network-bond-member-{}", uplink(0)));
    h.toggle(&format!("network-bond-member-{}", uplink(1)));
    h.click("network-review");
    assert_eq!(
        ws(&h).error.as_deref(),
        Some("Choose the destination network")
    );
    let destination = inventory(&h).networks[0].uuid;
    h.pick("network-bond-network", &destination.to_string());
    h.pick("network-bond-mode", BondMode::Lacp.as_str());
    assert!(h.find("network-bond-hash").is_some(), "LACP has a hash");
    h.click("network-review");
    let review = ws(&h).review.as_ref().expect("review");
    assert_eq!(
        review.target,
        ResourceTarget::Network(NetworkId::from(destination))
    );
    assert!(matches!(
        &review.operation,
        ResourceAction::Network(NetworkOperation::CreateBond { members, mode, .. })
            if members.len() == 2 && *mode == BondMode::Lacp
    ));
    assert!(h.has_text(
        "The host will inherit the primary member's MAC. Active IP, VLAN and VM attachments must be moved separately before creating this bond."
    ));
}

#[test]
fn a_bond_and_a_vlan_interface_have_their_commands_in_the_row_menu() {
    let mut h = server_tab(TabId::Nics);
    let bond = BondId::from(Uuid::from_u128(0x54));
    let vlan = VlanId::from(Uuid::from_u128(0x55));
    amend(&mut h, |i| {
        i.details.bonds.insert(
            bond,
            NetworkBond {
                primary_slave: Some(uplink(1)),
                hashing_algorithm: Some("src_mac".into()),
                hashing_algorithm_known: true,
                id: bond,
                master: Some(uplink(0)),
                slaves: vec![uplink(1)],
                mode: "lacp".into(),
                links_up: None,
            },
        );
        i.details.vlans.insert(
            vlan,
            Vlan {
                id: vlan,
                tagged_pif: Some(uplink(1)),
                untagged_pif: Some(uplink(0)),
                tag: Some(7),
            },
        );
    });
    let items = h.open_row_menu("nics-table", &uplink(0).to_string());
    let labels: Vec<_> = items.iter().map(|item| item.label.as_str()).collect();
    for label in [
        "Delete VLAN Interface",
        "Change Bond Mode",
        "Change Bond Hash",
        "Delete Bond",
    ] {
        assert!(labels.contains(&label), "{label} in {labels:?}");
    }
    assert!(h.has_text(&format!("Bond {bond} · lacp · hash src_mac")));

    // The hash: one other hash, so it is the change.
    h.invoke("server.bond.hash");
    h.click("network-review");
    assert!(matches!(
        &ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetBondHash { expected, .. })
            if expected.as_deref() == Some("src_mac")
    ));
    assert!(h.has_text("Traffic distribution changes immediately."));
    h.dismiss();

    // The mode: the choice of the other modes is the draft.
    h.invoke("server.bond.mode");
    h.click("network-review");
    assert_eq!(ws(&h).error.as_deref(), Some("Choose a change to review"));
    h.pick("network-bond-mode", BondMode::ActiveBackup.as_str());
    h.click("network-review");
    assert!(matches!(
        &ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetBondMode { expected_mode, mode })
            if expected_mode == "lacp" && *mode == BondMode::ActiveBackup
    ));
    assert!(h.has_text(
        "Coordinate switch configuration first. Network traffic may be interrupted."
    ));
    h.dismiss();

    h.click("server.bond.delete");
    assert!(matches!(
        &ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::DeleteBond { master, primary, .. })
            if *master == uplink(0) && *primary == uplink(1)
    ));
    assert!(h.find("network-back").is_none());
    h.dismiss();

    h.invoke("nic.vlan.delete");
    let review = ws(&h).review.as_ref().expect("review");
    assert_eq!(review.target, ResourceTarget::Vlan(vlan));
    assert!(matches!(
        &review.operation,
        ResourceAction::Network(NetworkOperation::DeleteVlan { tag: 7, .. })
    ));
}

#[test]
fn the_mtu_field_reviews_the_change_that_fits_the_network() {
    let mut h = Harness::scene("pool-administration");
    let (used, unused) = used_and_unused(&mut h);
    let pool = pool(&h);
    // A network with interfaces: the change of the networking dialog.
    h.select(Target::one(ObjectRef::network(pool, used.uuid())));
    h.invoke("network.properties");
    // This area's two pages, among those other areas provide for a network.
    let labels: Vec<String> = match &h.surface().kind {
        SurfaceKind::Properties { pages, .. } => pages
            .iter()
            .map(|page| page.label.clone())
            .filter(|label| label == "General" || label == "Network Settings")
            .collect(),
        _ => panic!("not a properties dialog"),
    };
    assert_eq!(labels, ["General", "Network Settings"]);
    h.pick("network-properties-pages", "network-settings");
    assert_eq!(h.node("network-mtu").value.as_deref(), Some("1500"));
    h.click("network-review");
    assert_eq!(ws(&h).error.as_deref(), Some("Choose a different MTU"));
    h.type_text("network-mtu", "9000");
    h.click("network-review");
    assert!(matches!(
        ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetMtu {
            expected: 1500,
            mtu: 9000,
            ..
        })
    ));
    assert_eq!(h.try_click("network-confirm"), Err(DEMO.into()));
    h.click("network-back");
    assert_eq!(h.node("network-mtu").value.as_deref(), Some("9000"));
    h.dismiss();
    assert!(h.model.areas.network.workspace.is_none());

    // A network without interfaces: the internal change of the generic
    // editor, with its texts.
    h.select(Target::one(ObjectRef::network(pool, unused.uuid())));
    h.invoke("network.properties");
    h.pick("network-properties-pages", "network-settings");
    h.click("network-review");
    assert_eq!(ws(&h).error.as_deref(), Some("The MTU is unchanged."));
    h.type_text("network-mtu", "100");
    h.click("network-review");
    assert_eq!(
        ws(&h).error.as_deref(),
        Some("Enter an MTU between 576 and 65535")
    );
    h.type_text("network-mtu", "9000");
    h.click("network-review");
    let review = ws(&h).review.as_ref().expect("review");
    assert_eq!(review.target, ResourceTarget::Network(unused));
    assert!(matches!(
        review.operation,
        ResourceAction::ConfigureInternalNetwork {
            expected_mtu: 1500,
            mtu: 9000
        }
    ));
    assert!(h.has_text(
        "Changing MTU requires an unused internal network; attached physical and VM interfaces must be absent."
    ));
}

#[test]
fn automatic_attachment_and_igmp_snooping_are_reviewed_from_their_pages() {
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    h.invoke("pool.properties");
    h.pick("pool-properties-pages", "network-options");
    assert_eq!(
        h.node("network-igmp-state").text.as_deref(),
        Some("IGMP snooping: disabled")
    );
    assert_eq!(
        h.node("network-igmp").label.as_deref(),
        Some("Enable pool IGMP snooping")
    );
    h.click("network-igmp");
    let review = ws(&h).review.as_ref().expect("review");
    assert_eq!(review.target, ResourceTarget::Pool(i.pool_id));
    assert!(matches!(
        review.operation,
        ResourceAction::Network(NetworkOperation::SetIgmpSnooping {
            expected: false,
            enabled: true
        })
    ));
    assert!(h.has_text("IGMP snooping: false → true"));
    assert!(h.has_text(
        "This changes multicast forwarding behavior across the pool."
    ));
    // A page always has itself to go back to.
    h.click("network-back");
    assert!(h.find("network-igmp").is_some());
    h.click("network-close");
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.network.workspace.is_none());

    let network = NetworkId::from(i.networks[0].uuid);
    h.select(Target::one(ObjectRef::network(i.pool_id, network.uuid())));
    h.invoke("network.properties");
    h.pick("network-properties-pages", "network-settings");
    assert_eq!(
        h.node("network-automatic-state").text.as_deref(),
        Some("Automatic attachment to new VMs: enabled")
    );
    h.click("network-automatic");
    assert!(matches!(
        &ws(&h).review.as_ref().expect("review").operation,
        ResourceAction::Network(NetworkOperation::SetAutomaticAttachment {
            expected: None,
            enabled: false
        })
    ));
    assert!(h.has_text("Automatically attach this network to new VMs: false"));
    // A dialog of the area replaces the page and its Properties dialog.
    h.model.areas.network.pif = None;
    let target = h.model.target();
    assert!(h.model.invoke(VLAN_ADD, &target));
    assert!(h.model.properties.is_none());
    assert_eq!(h.surface().id, CHANGE);
}

#[test]
fn name_and_description_are_the_general_page_of_four_dialogs() {
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    let host = *i.hosts.keys().next().expect("host");
    let cases = [
        ("pool.properties", ObjectRef::pool(i.pool_id)),
        ("server.properties", ObjectRef::host(i.pool_id, host)),
        (
            "storage.properties",
            ObjectRef::storage(i.pool_id, i.storage[0].uuid),
        ),
        (
            "network.properties",
            ObjectRef::network(i.pool_id, i.networks[0].uuid),
        ),
    ];
    for (command, object) in cases {
        h.select(Target::one(object.clone()));
        assert!(h.model.open_properties(&Target::one(object), Some("general")), "{command}");
        assert_eq!(ed(&h).kind, Kind::Rename, "{command}");
        assert!(h.find("resource-name").is_some(), "{command}");
        h.type_text("resource-name", "Renamed");
        h.click("resource-review");
        assert!(h.has_text("Name: Renamed"), "{command}");
        assert_eq!(h.try_click("resource-confirm"), Err(DEMO.into()));
        h.click("resource-back");
        h.click("resource-close");
        assert!(h.model.properties.is_none(), "{command}");
        assert!(h.model.areas.network.editor.is_none(), "{command}");
    }
}

#[test]
fn scheduling_default_storage_and_rescan_open_on_their_review() {
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    let pool = i.pool_id;
    let (host, record) = i.hosts.iter().next().expect("host");
    let enabled = record.enabled;
    h.select(Target::one(ObjectRef::host(pool, *host)));
    let item = h.menu_item("server.scheduling").expect("in the Server menu");
    assert_eq!(
        item.label,
        if enabled {
            "Disable New VM Placement"
        } else {
            "Enable New VM Placement"
        }
    );
    h.invoke("server.scheduling");
    assert_eq!(h.surface().id, "server-scheduling");
    assert_eq!(ed(&h).kind, Kind::HostEnabled(!enabled));
    assert!(h.has_text(&format!(
        "Allow new VM placement on this host: {}",
        if enabled { "No" } else { "Yes" }
    )));
    assert!(h.has_text(
        "This controls scheduling. Running guests are not evacuated or shut down."
    ));
    assert!(h.find("resource-back").is_none());
    h.dismiss();

    // Default storage: the repository the command runs on is the choice.
    let repository = SrId::from(i.storage[0].uuid);
    let target = Target::one(ObjectRef::storage(pool, repository.uuid()));
    assert_eq!(
        h.model.eligibility(SET_DEFAULT, &target),
        Eligibility::disabled(ALREADY_DEFAULT)
    );
    assert_eq!(
        h.model
            .eligibility(SET_DEFAULT, &Target::one(ObjectRef::pool(pool))),
        Eligibility::disabled(SELECT_REPOSITORY)
    );
    let vm = *i.vms.keys().next().expect("vm");
    assert_eq!(
        h.model
            .eligibility(SET_DEFAULT, &Target::one(ObjectRef::vm(pool, vm))),
        Eligibility::Hidden
    );
    amend(&mut h, |i| {
        i.details.pool.as_mut().expect("pool").default_sr_known = false;
    });
    assert_eq!(
        h.model.eligibility(SET_DEFAULT, &target),
        Eligibility::disabled(DEFAULT_UNKNOWN)
    );
    amend(&mut h, |i| {
        let pool = i.details.pool.as_mut().expect("pool");
        pool.default_sr_known = true;
        pool.default_sr = None;
    });
    assert_eq!(h.model.eligibility(SET_DEFAULT, &target), Eligibility::Enabled);
    h.invoke_on("storage.set-default", &target);
    assert_eq!(h.surface().id, "storage-set-default");
    assert_eq!(ed(&h).target, ResourceTarget::Pool(pool));
    assert_eq!(ed(&h).selected_sr, Some(repository));
    assert!(h.has_text(&format!(
        "New default: {} ({repository})",
        i.storage[0].name
    )));
    assert!(h.find("resource-back").is_none());
    assert!(h.find(permissions::REVIEW_KEY).is_some());
    h.dismiss();
    // Only a repository for VM disks can be the default.
    amend(&mut h, |i| {
        i.details
            .storage
            .get_mut(&repository)
            .expect("details")
            .content_type = "iso".into();
    });
    assert_eq!(h.model.eligibility(SET_DEFAULT, &target), Eligibility::Hidden);

    // Rescan. The Rune text said "in history" (decision 10 of AREAS.md).
    assert_eq!(
        h.model.eligibility(STORAGE_RESCAN, &target),
        Eligibility::Enabled
    );
    h.invoke_on("storage.rescan", &target);
    assert_eq!(h.surface().id, "storage-rescan");
    assert_eq!(ed(&h).target, ResourceTarget::Storage(repository));
    assert!(h.has_text(
        "Ask the storage backend to refresh its disk inventory. The operation will appear on the Events tab of the Notifications view."
    ));
    assert_eq!(h.try_click("resource-confirm"), Err(DEMO.into()));
}

#[test]
fn the_latest_operation_and_the_way_to_the_history_are_under_the_grids() {
    let mut h = Harness::scene("pool-administration");
    let pool = pool(&h);
    assert!(h.find("network-latest-operation").is_none());
    h.model.resource_rows.push(
        serde_json::from_value(serde_json::json!({
            "id": xenadmin_core::OperationId::new(), "pool": pool, "target": {"Pool": pool},
            "action": {"Pool": {"Join": {
                "source_host": Uuid::new_v4(), "destination_pool": pool,
                "destination_coordinator": Uuid::new_v4(), "destination_address": "192.0.2.20"
            }}},
            "state": "OutcomeUnknown", "progress": null, "detail": "Fixture interrupted join",
            "created_at_ms": 0, "can_cancel": false, "cancel_requested": false, "steps": [],
            "created": null
        }))
        .expect("a record"),
    );
    let line = h.node("network-latest-operation").text.unwrap_or_default();
    assert!(line.starts_with("Latest operation: "), "{line}");
    assert!(line.ends_with("· OutcomeUnknown · Fixture interrupted join"), "{line}");
    h.click("network-operation-history");
    assert_eq!(h.model.active_tab(), TabId::Events);
}

// -- 3. Hazards ---------------------------------------------------------------------------

/// Hazard 43: a pool's own label may be empty while the inventory shows a
/// placeholder; the review must expect the real one.
#[test]
fn a_pool_rename_expects_the_label_the_host_holds() {
    let mut h = Harness::scene("pool-administration");
    amend(&mut h, |i| {
        i.details.pool.as_mut().expect("pool").name = String::new();
    });
    let shown = inventory(&h).pool_name.clone();
    assert!(!shown.is_empty());
    h.invoke("pool.properties");
    assert_eq!(ed(&h).expected_name, "");
    assert_eq!(ed(&h).label, shown);
    assert_eq!(h.node("resource-name").value.as_deref(), Some(""));
    // Without the host's record of the name there is nothing to compare
    // with: the page says so instead of offering the editor.
    h.dismiss();
    amend(&mut h, |i| i.details.pool = None);
    h.invoke("pool.properties");
    assert!(h.model.areas.network.editor.is_none());
    assert_eq!(
        h.node("network-page-error").text.as_deref(),
        Some("The host did not supply the current name and description. Refresh before editing.")
    );
    h.click("resource-close");
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.network.page_error.is_none());
}

/// Hazard 44: why an editor cannot open goes into the open editor when
/// there is one, else to the shared error line. Hazard 45: every kind
/// needs a current observation unless the pool is synthetic.
#[test]
fn an_editor_that_cannot_open_says_so_where_the_user_looks() {
    let mut h = Harness::scene("pool-administration");
    let pool = pool(&h);
    h.model.demo = false;
    let target = ResourceTarget::Pool(pool);
    assert!(!editor::open(&mut h.model, pool, target, Kind::CreateNetwork));
    assert!(h.model.areas.network.editor.is_none());
    assert_eq!(
        h.model.error.as_deref(),
        Some("The connection is not current")
    );
    h.model.demo = true;
    h.model.error = None;
    assert!(editor::open(&mut h.model, pool, target, Kind::CreateNetwork));
    h.model.demo = false;
    assert!(!editor::open(&mut h.model, pool, target, Kind::DefaultStorage));
    assert_eq!(ed(&h).kind, Kind::CreateNetwork);
    assert_eq!(
        ed(&h).error.as_deref(),
        Some("The connection is not current")
    );
    assert!(h.model.error.is_none());
}

/// A review made in the preview cannot be confirmed on a real connection
/// either: it has no observation. And without a current connection no
/// review is staged at all.
#[test]
fn a_review_without_a_current_observation_cannot_be_confirmed() {
    let mut h = Harness::scene("network-ipv6");
    h.click("network-review");
    h.model.demo = false;
    assert_eq!(h.try_click("network-confirm"), Err(STALE.into()));
    assert_eq!(
        h.node("network-change-review-stale").text.as_deref(),
        Some(STALE)
    );
    workspace::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(ws(&h).error, Some(Error::Stale.to_string()));
    h.click("network-back");
    assert_eq!(h.try_click("network-review"), Err(NOT_WRITABLE.into()));
    workspace::form_review(&mut h.model);
    assert!(ws(&h).review.is_none());
    assert_eq!(ws(&h).error, Some(Error::Stale.to_string()));

    let mut h = Harness::scene("network-create");
    h.model.demo = false;
    assert_eq!(h.try_click("resource-confirm"), Err(STALE.into()));
    editor::submit(&mut h.model);
    assert!(h.model.resource_operations.is_empty());
    assert_eq!(ed(&h).error, Some(Error::Stale.to_string()));
}

/// The review another area stages here keeps that area's staleness rule:
/// the Rune `service::unavailable` and its siblings.
#[test]
fn a_staged_review_of_another_area_is_gated_by_its_check() {
    fn gone(_: &Model, _: &Workspace) -> Option<String> {
        Some("The reviewed network changed".into())
    }
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    let network = NetworkId::from(i.networks[0].uuid);
    let (target, operation, lines) = workspace::automatic(&i, network).expect("staged");
    super::open_review(
        &mut h.model,
        i.pool_id,
        Some(network),
        None,
        target,
        ResourceAction::Network(operation),
        lines,
        Some(Check {
            reason: gone,
            conflict: true,
        }),
    );
    assert_eq!(h.surface().id, CHANGE);
    assert_eq!(
        h.node("network-unavailable").text.as_deref(),
        Some("The reviewed network changed")
    );
    assert_eq!(
        h.try_click("network-confirm"),
        Err("The reviewed network changed".into())
    );
    assert!(h.find("network-back").is_none());
    h.click("network-close");
    assert!(h.surfaces().is_empty());
}

/// Hazard 54: the two editors of the area are exclusive, as the Rune
/// dialogs were (`network_admin::open` cleared `resource_editor`).
#[test]
fn one_surface_of_the_area_at_a_time() {
    let mut h = Harness::scene("network-interfaces");
    assert!(h.model.areas.network.workspace.is_some());
    let target = Target::one(ObjectRef::pool(pool(&h)));
    assert!(h.model.invoke(NEW, &target));
    assert!(h.model.areas.network.workspace.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, WIZARD);
    let network = Target::one(ObjectRef::network(
        pool(&h),
        inventory(&h).networks[0].uuid,
    ));
    assert!(h.model.invoke(VLAN_ADD, &network));
    assert!(h.model.areas.network.editor.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(
        h.model.eligibility(REMOVE, &target),
        Eligibility::disabled(SELECT_NETWORK)
    );
}

// -- 5. With the `network_services` area ----------------------------------------------

/// The Type step of New Network lists the fabric wizards of
/// `network_services` under the labels of their commands. Without a
/// network as the target they say what is missing; with one, choosing a
/// type closes this wizard and opens that area's on the same network.
#[test]
fn the_type_step_hands_over_to_the_fabric_wizards() {
    use crate::areas::network_services::commands::{FABRIC_ADD, FABRIC_COMPLETE_POOL};
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    let network = NetworkId::from(i.networks[0].uuid);
    // Opened on the pool: the fabric types need a network.
    h.invoke(NEW.0);
    assert_eq!(h.surface().id, WIZARD);
    assert_eq!(
        h.node("network-new-type-fabric").label.as_deref(),
        Some("Add Tunnel or SR-IOV Interface")
    );
    assert_eq!(
        h.node("network-new-type-fabric-pool").label.as_deref(),
        Some("Complete Pool Coverage")
    );
    // The reason is the command's own for the pool the wizard is open on.
    let on_pool = h.model.target();
    for (key, command) in [
        ("network-new-type-fabric", FABRIC_ADD),
        ("network-new-type-fabric-pool", FABRIC_COMPLETE_POOL),
    ] {
        let reason = match h.model.eligibility(command, &on_pool) {
            Eligibility::Disabled(reason) => reason,
            Eligibility::Hidden => super::view::NEEDS_NETWORK.to_owned(),
            Eligibility::Enabled => panic!("a fabric command runs on a pool"),
        };
        assert_eq!(h.node(key).disabled, Some(reason));
    }
    // A refused hand-over leaves the wizard as it was.
    assert!(h.try_click("network-new-type-fabric").is_err());
    assert_eq!(h.surface().id, WIZARD);
    h.click("resource-close");

    // Opened on a network of the grid: each type opens its wizard there.
    for (option, command, surface) in [
        ("fabric", FABRIC_ADD, "network-fabric"),
        ("fabric-pool", FABRIC_COMPLETE_POOL, "network-fabric-pool"),
    ] {
        let target = Target::one(ObjectRef::network(i.pool_id, i.networks[0].uuid));
        h.model.select_in_grid(target.clone());
        h.invoke(NEW.0);
        assert_eq!(h.surface().id, WIZARD);
        let key = format!("network-new-type-{option}");
        match h.model.eligibility(command, &target) {
            Eligibility::Enabled => {
                assert_eq!(h.node(&key).disabled, None);
                h.pick("network-new-type", option);
                assert!(h.model.areas.network.editor.is_none());
                assert_eq!(h.surfaces().len(), 1);
                assert_eq!(h.surface().id, surface);
                let theirs = h.model.areas.network_services.editor.as_ref().expect("open");
                assert_eq!((theirs.pool, theirs.network), (i.pool_id, network));
                crate::areas::network_services::close(&mut h.model);
            }
            Eligibility::Disabled(reason) => {
                panic!("the demo offers {option}, but the command refuses: {reason}")
            }
            Eligibility::Hidden => panic!("{option} is hidden on a network"),
        }
    }
}

/// The Networking tab of a network shows the service state and the tunnel
/// and SR-IOV coverage that `network_services` describes.
#[test]
fn the_networking_tab_of_a_network_shows_services_and_coverage() {
    let mut h = Harness::scene("pool-administration");
    let i = inventory(&h);
    let network = NetworkId::from(i.networks[0].uuid);
    h.select(Target::one(ObjectRef::network(i.pool_id, i.networks[0].uuid)));
    h.open_tab(TabId::Networking);
    assert!(h.has_text("Network services"));
    assert!(h.has_text("Tunnel and SR-IOV coverage"));
    let theirs = crate::areas::network_services::networking_sections(&i, network);
    assert!(!theirs.is_empty());
    let blocks = h.tab().blocks();
    let nodes = crate::surface::flatten_blocks("tab", &blocks);
    assert_eq!(crate::surface::duplicate_keys(&nodes), Vec::<String>::new());
    for node in crate::surface::flatten_blocks("theirs", &theirs) {
        if let Some(text) = &node.text {
            assert!(h.has_text(text), "{text}");
        }
    }
}
