//! The tests of the area, in three parts.
//!
//! 1. **The Rune tests.** The five of `host_admin.rs`, under names that say
//!    which they were, and this area's share of two tests of other Rune
//!    files that ran over its scenes. Each asserts what the Rune test
//!    asserted. Where the Rune test wrote a field or called a private
//!    function, the test here goes the way a user goes (a command, a key
//!    typed, a button pressed) and then asserts on the same state.
//! 2. **What the port changed**: the chooser became commands, three changes
//!    became pages, and the editor reads the pool of its target.
//! 3. **The hazards** the map lists for this module (section 11), one test
//!    each, so the next change of this file cannot lose them silently.
//!
//! Every test starts from the scene's server: the demo pool with the first
//! server given what its changes need (`scenes::prepare`). The plain demo
//! inventory has no details of any server, so there every change is refused
//! with [`MISSING`].

use std::sync::Arc;

use xenadmin_core::{
    Error, HostId, HostOperation, HostPowerOnMode, Inventory, PoolId, PowerState, ResourceTarget,
};

use super::change::{self, Editor, Kind, MISSING, STALE};
use super::commands::{self, NOT_WRITABLE};
use super::view;
use crate::Harness;
use crate::command::{self, MenuItem};
use crate::prelude::*;
use crate::surface::EXPERIMENTAL_NOTICE_KEY;

// -- Fixtures -------------------------------------------------------------------

const DEMO: &str = "Demo mode cannot change infrastructure.";
const INVALID: &str = "Choose valid host settings and a change to apply";
const PAGES: &str = "host-properties-pages";

/// The scene's server, selected, with nothing open.
fn server() -> Harness {
    let mut h = Harness::scene("host-administration");
    h.dismiss();
    assert!(h.surfaces().is_empty());
    h
}

/// The server every scene of the area acts on.
fn host(h: &Harness) -> HostId {
    let inventory = h.model.inventory.as_ref().expect("inventory");
    *inventory.hosts.keys().next().expect("a server")
}

fn host_target(h: &Harness) -> Target {
    let pool = h.model.inventory.as_ref().expect("inventory").pool_id;
    Target::one(ObjectRef::host(pool, host(h)))
}

fn pool_target(h: &Harness) -> Target {
    let pool = h.model.inventory.as_ref().expect("inventory").pool_id;
    Target::one(ObjectRef::pool(pool))
}

/// The open change.
#[track_caller]
fn editor(h: &Harness) -> &Editor {
    h.model.areas.host_admin.editor.as_ref().expect("a change")
}

/// The inventory moves on: what a poll does when the host reports a change.
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

/// The server as it is after entering maintenance and evacuation: disabled,
/// marked, and no guest running on it.
fn in_maintenance(h: &mut Harness) {
    let id = host(h);
    amend(h, |i| {
        i.hosts.get_mut(&id).expect("host").enabled = false;
        i.details
            .hosts
            .get_mut(&id)
            .expect("details")
            .maintenance_mode = Some(true);
        for vm in i.vms.values_mut().filter(|vm| vm.host == Some(id)) {
            vm.power_state = PowerState::Halted;
            vm.host = None;
        }
    });
}

/// The server as a dead member: not live, disabled, empty, and another
/// server is the coordinator.
fn dead(h: &mut Harness) {
    in_maintenance(h);
    let id = host(h);
    amend(h, |i| {
        i.coordinator = i.hosts.keys().copied().find(|other| *other != id);
        for metrics in i.details.host_metrics.values_mut() {
            metrics.live = Some(false);
        }
    });
}

/// The items of this area in the Server menu, in menu order.
fn server_menu(h: &Harness) -> Vec<MenuItem> {
    h.menus()
        .into_iter()
        .filter(|menu| menu.menu == Menu::Server)
        .flat_map(|menu| menu.items().cloned().collect::<Vec<_>>())
        .filter(|item| command::find(item.id).is_some_and(|spec| spec.area == "host_admin"))
        .collect()
}

// -- 1. The Rune tests ------------------------------------------------------------

/// Rune `maintenance_review_keeps_host_and_guest_identity_at_both_widths`:
/// after the live pool id changes, pressing `host-confirm` submits nothing
/// and the editor keeps its target and its review. (That the button is on
/// screen at 1440x900 and at 513x652 is for a GPUI interaction test.)
#[test]
fn maintenance_review_keeps_host_and_guest_identity() {
    let mut h = Harness::scene("host-administration");
    let target = editor(&h).target;
    let op = editor(&h).review.clone().expect("maintenance review");
    assert!(matches!(op, HostOperation::EnterMaintenance { .. }));
    amend(&mut h, |i| i.pool_id = PoolId::from(uuid::Uuid::new_v4()));
    // The button refuses, and says why.
    assert_eq!(h.try_click("host-confirm"), Err(STALE.into()));
    assert_eq!(editor(&h).target, target);
    assert_eq!(editor(&h).review, Some(op.clone()));
    assert!(h.model.resource_operations.is_empty());
    // A handler can run after the state has moved on: it refuses too, and
    // the review stays to be read.
    change::submit(&mut h.model);
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert_eq!(editor(&h).target, target);
    assert_eq!(editor(&h).review, Some(op));
    assert!(h.model.resource_operations.is_empty());
    // The next poll finds the server's pool gone and moves the selection
    // away, which closes the dialog (ARCHITECTURE.md, section 3).
    h.tick(0.1);
    assert!(h.model.areas.host_admin.editor.is_none());
    assert!(h.model.resource_operations.is_empty());
}

/// Rune `memory_editor_rejects_zero_overflow_and_physical_overcommit`.
#[test]
fn memory_editor_rejects_zero_overflow_and_physical_overcommit() {
    let mut h = Harness::scene("host-memory");
    const WHOLE: &str = "Enter a positive whole MiB memory value";
    for (invalid, reason) in [
        ("0", WHOLE),
        ("-1", WHOLE),
        ("18446744073709551615", WHOLE),
        ("999999", "Memory exceeds the host's physical memory"),
    ] {
        h.type_text("host-setting-value", invalid);
        h.click("host-review");
        assert!(editor(&h).review.is_none(), "{invalid}");
        assert_eq!(h.node("host-error").text.as_deref(), Some(reason));
    }
    h.type_text("host-setting-value", "3072");
    h.click("host-review");
    assert!(matches!(
        editor(&h).review,
        Some(HostOperation::SetDom0Memory {
            memory: 3221225472,
            ..
        })
    ));
    assert!(h.find("host-error").is_none());
}

/// Rune `logging_review_keeps_exact_old_and_new_destination`. Choosing
/// "Remote logging" in the chooser is turning to the Log Destination page.
#[test]
fn logging_review_keeps_exact_old_and_new_destination() {
    let mut h = server();
    h.invoke("server.properties");
    h.pick(PAGES, "log-destination");
    h.type_text("host-setting-value", "logs.example.test");
    h.click("host-review");
    let op = editor(&h).review.clone();
    h.pick(PAGES, "log-destination");
    h.type_text("host-setting-value", "bad name");
    h.click("host-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(
        op,
        Some(HostOperation::SetSyslog {
            expected_destination: None,
            destination: Some("logs.example.test".into())
        })
    );
}

/// Rune `autostart_is_explicitly_pool_scoped_and_unknown_is_not_false`.
#[test]
fn autostart_is_explicitly_pool_scoped_and_unknown_is_not_false() {
    let mut h = server();
    let pool = pool_target(&h);
    h.invoke_on("pool.vm-autostart", &pool);
    let w = editor(&h);
    assert_eq!(w.target, ResourceTarget::Pool(w.pool()));
    assert_eq!(
        w.review,
        Some(HostOperation::SetPoolAutostart {
            expected: false,
            enabled: true
        })
    );
    h.dismiss();
    amend(&mut h, |i| {
        i.details.pool.as_mut().expect("pool details").auto_poweron = None;
    });
    // The Rune steps, `pool(s); review(s)`: no review comes of an unknown value.
    h.model.areas.host_admin.editor =
        Editor::capture(&h.model, pool.single().expect("pool"), Kind::Autostart);
    change::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).error.as_deref(), Some(MISSING));
    h.dismiss();
    // Which is why the command is not offered: the dialog would open on it.
    assert_eq!(
        h.try_invoke_on("pool.vm-autostart", &pool),
        Err(MISSING.into())
    );
    assert!(h.surfaces().is_empty());
}

/// Rune `forget_requires_offline_disabled_noncoordinator_and_explicit_power_off_ack`.
#[test]
fn forget_requires_offline_disabled_noncoordinator_and_explicit_power_off_ack() {
    let mut h = server();
    let object = host_target(&h).single().expect("server").clone();
    // The Rune steps, `select(s, Kind::Forget); review(s)`.
    h.model.areas.host_admin.editor = Editor::capture(&h.model, &object, Kind::Forget);
    change::review(&mut h.model);
    assert!(editor(&h).review.is_none());
    let w = h.model.areas.host_admin.editor.as_mut().expect("a change");
    w.acknowledged = true;
    let availability = change::operation(w)
        .expect("operation")
        .availability(&w.initial, w.target);
    assert!(!availability.is_available());
    // The scene's server is live: the command says so instead of opening.
    h.dismiss();
    assert_eq!(
        h.eligibility("server.destroy"),
        Eligibility::disabled(availability.explanation())
    );
}

/// This area's share of Rune `main.rs`
/// `dialogs_outside_the_supported_core_carry_the_experimental_notice`:
/// "host maintenance" shows no notice, "control-domain memory" shows it.
#[test]
fn only_changes_outside_the_supported_part_carry_the_experimental_notice() {
    let h = Harness::scene("host-administration");
    assert!(!h.model.experimental_notice(), "host maintenance");
    assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_none());
    let h = Harness::scene("host-memory");
    assert!(h.model.experimental_notice(), "control-domain memory");
    assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_some());
}

/// This area's share of Rune `session_permissions_view.rs`
/// `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`:
/// the review explains the permission, and pressing confirm on the
/// synthetic pool submits nothing and raises no error.
#[test]
fn review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard() {
    let mut h = Harness::scene("host-administration");
    assert!(
        h.find(permissions::REVIEW_KEY).is_some(),
        "captured review must explain permission discovery"
    );
    assert_eq!(h.try_click("host-confirm"), Err(DEMO.into()));
    assert!(h.model.resource_operations.is_empty());
    assert!(h.model.operations.is_empty());
    assert!(h.model.error.is_none());
    // Before the review there is no operation to assess, so no line.
    let h = Harness::scene("host-memory");
    assert!(h.find(permissions::REVIEW_KEY).is_none());
}

// -- 2. What the port changed ---------------------------------------------------------

/// The Rune chooser listed thirteen changes as buttons. Each is now reached
/// from the Server menu and from the context menu of a server.
#[test]
fn the_changes_of_the_chooser_are_commands_of_the_server_menu() {
    let mut h = server();
    let labels: Vec<_> = server_menu(&h)
        .into_iter()
        .map(|item| (item.id.0, item.label))
        .collect();
    let expected = [
        ("server.reboot", "Reboot"),
        ("server.power-on", "Power On"),
        ("server.shutdown", "Shut Down"),
        ("server.restart-toolstack", "Restart Toolstack"),
        ("server.maintenance", "Enter Maintenance Mode"),
        ("server.evacuate", "Evacuate Guests"),
        ("server.control-domain-memory", "Control Domain Memory"),
        ("server.destroy", "Destroy"),
        ("server.properties", "Properties"),
    ];
    assert_eq!(
        labels,
        expected.map(|(id, label)| (id, label.to_owned())).to_vec()
    );
    let context = h.context_menu(&host_target(&h));
    for (id, _) in expected {
        assert!(context.iter().any(|item| item.id.0 == id), "{id}");
    }
    // Autostart is the pool's: in its context menu, not the server's.
    assert!(
        !context
            .iter()
            .any(|item| item.id == commands::POOL_AUTOSTART)
    );
    let context = h.context_menu(&pool_target(&h));
    assert!(
        context
            .iter()
            .any(|item| item.id == commands::POOL_AUTOSTART)
    );
    // None of the server's changes is offered for the pool. (The pool's menu
    // does hold `server.` commands of the connections area: Disconnect and
    // the like act on the pool's connection.)
    for (id, _) in expected {
        assert!(!context.iter().any(|item| item.id.0 == id), "{id}");
    }
    // The entry to memory settings on a server's Memory tab.
    h.open_tab(TabId::Memory);
    assert!(
        h.local_row()
            .iter()
            .any(|item| item.id == commands::CONTROL_DOMAIN_MEMORY)
    );
}

/// Wrong kind or count of objects: hidden. Right object that cannot change
/// now: disabled, with the reason the dialog would have given.
#[test]
fn a_command_is_hidden_for_other_objects_and_disabled_with_the_reason_of_its_review() {
    let mut h = server();
    let pool = pool_target(&h);
    let inventory = h.model.inventory.clone().expect("inventory");
    let vm = *inventory.vms.keys().next().expect("a guest");
    let vm = Target::one(ObjectRef::vm(inventory.pool_id, vm));
    let two = Target::many(
        inventory
            .hosts
            .keys()
            .take(2)
            .map(|id| ObjectRef::host(inventory.pool_id, *id)),
    );
    for spec in commands::commands() {
        let hidden: &[&Target] = if spec.id == commands::POOL_AUTOSTART {
            &[&vm, &two]
        } else {
            &[&vm, &two, &pool]
        };
        for target in hidden {
            assert_eq!(
                h.model.eligibility(spec.id, target),
                Eligibility::Hidden,
                "{} on {:?}",
                spec.id,
                target.kind()
            );
        }
    }
    // A server whose details the inventory does not hold: every change is
    // refused with the one text; Properties opens, and its pages say it.
    let bare = *inventory.hosts.keys().nth(1).expect("a second server");
    let bare = Target::one(ObjectRef::host(inventory.pool_id, bare));
    for spec in commands::commands() {
        if spec.id == commands::POOL_AUTOSTART {
            continue;
        }
        let expected = if spec.id == commands::PROPERTIES {
            Eligibility::Enabled
        } else {
            Eligibility::disabled(MISSING)
        };
        assert_eq!(h.model.eligibility(spec.id, &bare), expected, "{}", spec.id);
    }
    h.invoke_on("server.properties", &bare);
    assert_eq!(h.node("host-unavailable").text.as_deref(), Some(MISSING));
    h.dismiss();
    // The scene's server takes guests: a reboot is refused in the core's
    // words (the refusal the lab recorded on an enabled host).
    assert_eq!(
        h.try_invoke("server.reboot"),
        Err(
            "Disable scheduling and evacuate guests first; a multi-host pool coordinator must \
             be handed over separately"
                .into()
        )
    );
    assert!(h.surfaces().is_empty());
    // Rune kept "Pool VM autostart…" disabled on a connection that cannot write.
    h.model.demo = false;
    assert_eq!(
        h.model.eligibility(commands::POOL_AUTOSTART, &pool),
        Eligibility::disabled(NOT_WRITABLE)
    );
}

/// "Enter or Exit Maintenance Mode" is one item: whichever of the two the
/// server's state leaves possible.
#[test]
fn the_maintenance_item_enters_or_exits() {
    let mut h = server();
    let item = h.menu_item("server.maintenance").expect("item");
    assert_eq!(
        (item.label.as_str(), item.disabled),
        ("Enter Maintenance Mode", None)
    );
    h.invoke("server.maintenance");
    assert_eq!(h.surface().id, "host-enter-maintenance");
    assert!(matches!(
        editor(&h).review,
        Some(HostOperation::EnterMaintenance { .. })
    ));
    h.dismiss();
    in_maintenance(&mut h);
    let item = h.menu_item("server.maintenance").expect("item");
    assert_eq!(
        (item.label.as_str(), item.disabled),
        ("Exit Maintenance Mode", None)
    );
    h.invoke("server.maintenance");
    assert_eq!(h.surface().id, "host-exit-maintenance");
    assert_eq!(editor(&h).review, Some(HostOperation::ExitMaintenance));
}

/// Every change without a draft opens as a dialog on its review: its own
/// id and title, the notice exactly when it is outside the supported part,
/// the server named, and a confirm button the synthetic pool refuses.
#[test]
fn a_change_without_a_draft_opens_on_its_review() {
    let cases = [
        // command, dialog, title, a phrase of its review
        (
            "server.evacuate",
            "host-evacuate",
            "Evacuate guests",
            "Scheduling will be disabled for maintenance.",
        ),
        (
            "server.maintenance",
            "host-exit-maintenance",
            "Exit maintenance",
            "Enable scheduling, then clear the maintenance marker.",
        ),
        (
            "server.reboot",
            "host-reboot",
            "Reboot host",
            "The host must be disabled and empty of running or paused guests.",
        ),
        (
            "server.shutdown",
            "host-shutdown",
            "Shut down host",
            "The host must be disabled and empty of running or paused guests.",
        ),
        (
            "server.restart-toolstack",
            "host-restart-toolstack",
            "Restart toolstack",
            "Restart XAPI only with pool HA disabled",
        ),
    ];
    let mut h = server();
    in_maintenance(&mut h);
    // A single server may reboot; in a pool of several its coordinator may not.
    amend(&mut h, |i| i.coordinator = None);
    for (command, dialog, title, phrase) in cases {
        h.invoke(command);
        let surface = h.surface();
        assert_eq!(
            (surface.id.as_str(), surface.title.as_str()),
            (dialog, title)
        );
        assert!(matches!(surface.kind, SurfaceKind::Dialog), "{command}");
        assert!(!surface.experimental, "{command} is supported");
        assert_eq!(
            surface.subtitle.as_deref(),
            Some("xcp-lab-01 (20000000-0000-0000-0000-000000000001)")
        );
        assert!(editor(&h).review.is_some(), "{command}");
        assert!(h.has_text(phrase), "{command}");
        assert_eq!(h.try_click("host-confirm"), Err(DEMO.into()));
        // Nothing was entered, so there is nothing to go back to.
        assert!(h.find("host-back").is_none());
        assert!(h.find("host-preview").is_some());
        h.assert_unique_keys();
        h.click("host-close");
        assert!(h.surfaces().is_empty());
    }
    // Powering on is for a server that is off and has a way to be woken.
    dead(&mut h);
    let id = host(&h);
    amend(&mut h, |i| {
        i.details.hosts.get_mut(&id).expect("details").power_on_mode =
            Some(HostPowerOnMode::WakeOnLan.as_str().into());
    });
    h.invoke("server.power-on");
    let surface = h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        ("host-power-on", "Power on host")
    );
    assert!(surface.experimental);
    assert_eq!(editor(&h).review, Some(HostOperation::PowerOn));
    h.dismiss();
    // And the pool's one change.
    let pool = pool_target(&h);
    h.invoke_on("pool.vm-autostart", &pool);
    let surface = h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        ("pool-vm-autostart", "Pool VM autostart")
    );
    assert!(surface.experimental);
    assert!(h.has_text("Pool-wide VM autostart: false → true."));
    assert!(h.find("host-back").is_none());
    h.assert_unique_keys();
}

/// Forgetting a dead server opens on its acknowledgement, and no review
/// comes without it.
#[test]
fn a_dead_server_is_forgotten_only_after_the_acknowledgement() {
    let mut h = server();
    dead(&mut h);
    h.invoke("server.destroy");
    let surface = h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        ("host-forget", "Forget dead host")
    );
    assert!(surface.experimental);
    assert_eq!(h.node("host-forget-off").checked, Some(false));
    // The draft says already why a review would be refused.
    assert_eq!(h.node("host-unavailable").text.as_deref(), Some(INVALID));
    h.click("host-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.node("host-error").text.as_deref(), Some(INVALID));
    h.toggle("host-forget-off");
    assert!(h.find("host-error").is_none());
    assert!(h.find("host-unavailable").is_none());
    h.click("host-review");
    assert_eq!(
        editor(&h).review,
        Some(HostOperation::ForgetDeadHost {
            verified_power_off: true
        })
    );
    assert!(h.has_text("Remove this dead host's pool record only."));
    h.assert_unique_keys();
    h.click("host-back");
    assert_eq!(h.node("host-forget-off").checked, Some(true));
}

/// The three settings are pages of the server's Properties dialog. Turning
/// the page starts that page's change afresh, as choosing another change in
/// the Rune chooser did.
#[test]
fn the_settings_of_a_server_are_pages_of_its_properties() {
    let mut h = server();
    let id = host(&h);
    amend(&mut h, |i| {
        i.details
            .hosts
            .get_mut(&id)
            .expect("details")
            .syslog_destination = Some("old.example.test".into());
    });
    h.invoke("server.properties");
    let surface = h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        (view::PROPERTIES, "Properties")
    );
    let SurfaceKind::Properties { pages, current, .. } = &surface.kind else {
        panic!("not a properties dialog");
    };
    let labels: Vec<_> = pages.iter().map(|page| page.label.as_str()).collect();
    assert_eq!(labels, ["Multipathing", "Log Destination", "Power On"]);
    assert_eq!(current, "multipathing");
    assert!(surface.experimental);
    assert_eq!(editor(&h).kind, Kind::Multipath);
    h.assert_unique_keys();

    h.pick(PAGES, "log-destination");
    assert_eq!(editor(&h).kind, Kind::Logging);
    assert_eq!(
        h.node("host-setting-value").value.as_deref(),
        Some("old.example.test")
    );
    h.type_text("host-setting-value", "new.example.test");
    assert!(h.has_text("Remote logging: old.example.test → new.example.test."));
    h.submit("host-setting-value");
    assert_eq!(
        editor(&h).review,
        Some(HostOperation::SetSyslog {
            expected_destination: Some("old.example.test".into()),
            destination: Some("new.example.test".into())
        })
    );
    assert!(h.model.experimental_notice());
    h.assert_unique_keys();

    // The Power On page starts on the mode the server is not in.
    h.pick(PAGES, "power-on");
    assert_eq!(editor(&h).kind, Kind::WakeOnLan);
    assert!(editor(&h).review.is_none());
    assert!(h.find("host-unavailable").is_none());
    // The other mode is the one it is in: nothing to change.
    h.click("host-choice-Disable remote power-on");
    assert_eq!(editor(&h).kind, Kind::DisablePowerOn);
    assert_eq!(h.node("host-unavailable").text.as_deref(), Some(INVALID));
    h.pick("host-power-on-mode", "wake-on-lan");
    h.click("host-review");
    assert_eq!(
        editor(&h).review,
        Some(HostOperation::ConfigurePowerOn {
            expected_mode: String::new(),
            mode: HostPowerOnMode::WakeOnLan
        })
    );
    h.assert_unique_keys();

    // Back on the first page the draft of the second is gone.
    h.pick(PAGES, "log-destination");
    assert_eq!(
        h.node("host-setting-value").value.as_deref(),
        Some("old.example.test")
    );
    assert!(editor(&h).review.is_none());
}

/// Multipathing needs an evacuated server in maintenance with its storage
/// detached; the page says so until then, and reviews the toggle after.
#[test]
fn the_multipathing_page_reviews_the_toggle_once_the_server_is_ready() {
    const NOT_READY: &str = "Evacuate the host, enter maintenance and detach its storage \
        connections before changing multipathing";
    let mut h = server();
    h.invoke("server.properties");
    assert_eq!(h.node("host-unavailable").text.as_deref(), Some(NOT_READY));
    h.click("host-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.node("host-error").text.as_deref(), Some(NOT_READY));
    h.dismiss();
    in_maintenance(&mut h);
    let id = host(&h);
    amend(&mut h, |i| {
        for pbd in i.details.pbds.values_mut().filter(|p| p.host == Some(id)) {
            pbd.currently_attached = Some(false);
        }
    });
    h.invoke("server.properties");
    assert!(h.find("host-unavailable").is_none());
    h.click("host-review");
    assert_eq!(
        editor(&h).review,
        Some(HostOperation::SetMultipath {
            expected: false,
            enabled: true
        })
    );
    // A page is gone back to, also when nothing was typed on it.
    h.click("host-back");
    assert!(editor(&h).review.is_none());
    assert_eq!(editor(&h).kind, Kind::Multipath);
}

/// A page leads to the related workflow of another area by that area's
/// command, and shows the way exactly as that command allows it: absent
/// while hidden (or not yet registered), disabled with its reason.
#[test]
fn a_link_to_another_area_follows_that_areas_command() {
    let mut h = server();
    h.invoke("server.properties");
    let target = host_target(&h);
    for (page, key, command) in [
        (
            "multipathing",
            "host-multipath-workflow",
            commands::MULTIPATH_WORKFLOW,
        ),
        ("power-on", "host-bmc-settings", commands::POWER_ON_SETTINGS),
    ] {
        h.pick(PAGES, page);
        let node = h.find(key);
        match h.model.eligibility(command, &target) {
            Eligibility::Hidden => assert!(node.is_none(), "{key}"),
            Eligibility::Disabled(reason) => {
                assert_eq!(node.expect(key).disabled, Some(reason));
            }
            Eligibility::Enabled => assert_eq!(node.expect(key).disabled, None),
        }
    }
    // The rule itself, shown with commands that exist today.
    let w = editor(&h);
    let link = |command| view::link(&h.model, w, "k", "Label", command);
    assert!(link(CommandId("no.such-command")).is_none());
    assert!(link(commands::POOL_AUTOSTART).is_none(), "hidden");
    let reboot = link(commands::REBOOT).expect("shown");
    assert!(reboot.disabled.is_some(), "disabled with its reason");
    let memory = link(commands::CONTROL_DOMAIN_MEMORY).expect("shown");
    assert!(memory.disabled.is_none());
    // Pressing it runs the command on this server; this surface gives way.
    (memory.on_press)(&mut h.model);
    assert_eq!(h.surface().id, "host-dom0-memory");
    assert_eq!(h.surfaces().len(), 1);
    // A press that comes after the command stopped being possible is
    // refused, and this surface stays.
    h.dismiss();
    h.invoke("server.properties");
    let w = editor(&h);
    let memory = view::link(&h.model, w, "k", "Label", commands::CONTROL_DOMAIN_MEMORY);
    let id = host(&h);
    amend(&mut h, |i| {
        i.details.hosts.remove(&id);
    });
    (memory.expect("shown").on_press)(&mut h.model);
    assert_eq!(h.surface().id, view::PROPERTIES);
    assert_eq!(h.model.error.as_deref(), Some(MISSING));
}

/// A row of the Search tab can belong to another pool than the one the
/// tree shows. The editor reads and names the pool of its target.
#[test]
fn a_change_reads_the_pool_of_its_target_not_the_scope_sessions() {
    let mut h = server();
    let mut other = h
        .model
        .inventory
        .clone()
        .expect("inventory")
        .as_ref()
        .clone();
    other.pool_id = PoolId::parse("10000000-0000-0000-0000-000000000063").expect("pool id");
    other.pool_name = "Recovery lab".into();
    let id = host(&h);
    other.hosts.get_mut(&id).expect("host").name = "recovery-01".into();
    let pool = other.pool_id;
    h.model.add_synthetic_pool("Recovery lab", other);
    let scope = h.model.scope().clone();
    h.invoke_on(
        "server.maintenance",
        &Target::one(ObjectRef::host(pool, id)),
    );
    assert_eq!(h.model.scope(), &scope);
    assert_eq!(editor(&h).pool(), pool);
    assert_eq!(
        h.surface().subtitle.as_deref(),
        Some("recovery-01 (20000000-0000-0000-0000-000000000001)")
    );
    assert!(h.has_text("Pool 10000000-0000-0000-0000-000000000063"));
    // That pool is synthetic too, whatever the scope session is.
    h.model.demo = false;
    assert!(h.find("host-preview").is_some());
    assert_eq!(h.try_click("host-confirm"), Err(DEMO.into()));
}

/// The dialog belongs to the one server it was opened for.
#[test]
fn another_selection_or_another_session_closes_the_change() {
    let mut h = Harness::scene("host-administration");
    let pool = pool_target(&h);
    h.select(pool);
    assert!(h.model.areas.host_admin.editor.is_none());
    let mut h = Harness::scene("host-memory");
    h.model.load_demo(3, 48);
    assert!(h.model.areas.host_admin.editor.is_none());
    // Close, and Escape.
    let mut h = Harness::scene("host-memory");
    h.click("host-close");
    assert!(h.surfaces().is_empty());
    let mut h = Harness::scene("host-memory");
    h.dismiss();
    assert!(h.surfaces().is_empty());
}

// -- 3. The hazards -----------------------------------------------------------------

/// Hazards 2 and 4: the review is written from the inventory captured when
/// the dialog opened, and what is submitted is that review. The live
/// inventory moving on changes neither; it makes the review stale.
#[test]
fn the_review_is_of_the_captured_inventory() {
    let mut h = Harness::scene("host-administration");
    let id = host(&h);
    let captured = editor(&h).review.clone().expect("review");
    let HostOperation::EnterMaintenance { resident_vms, .. } = &captured else {
        panic!("not a maintenance review");
    };
    assert!(!resident_vms.is_empty());
    let first = resident_vms[0];
    let before = h.texts();
    // A guest leaves the server while the review is open.
    amend(&mut h, |i| {
        i.vms.get_mut(&first).expect("guest").host = None;
    });
    assert_eq!(editor(&h).review, Some(captured.clone()));
    assert_eq!(h.texts(), before);
    assert!(h.has_text(&format!("({first})")));
    // A dialog opened now reviews the guests there are now.
    h.dismiss();
    h.invoke("server.maintenance");
    let HostOperation::EnterMaintenance { resident_vms, .. } =
        editor(&h).review.clone().expect("review")
    else {
        panic!("not a maintenance review");
    };
    assert!(!resident_vms.contains(&first));
    let inventory = h.model.inventory.as_ref().expect("inventory");
    assert_eq!(resident_vms, inventory.host_resident_guests(id));
}

/// Hazard 3: a review that is no longer current can be read and not
/// confirmed. Outside the demo the fixture has no connection, which is the
/// state after a disconnect.
#[test]
fn a_stale_review_says_so_and_cannot_be_confirmed() {
    let mut h = Harness::scene("host-administration");
    h.model.demo = false;
    assert_eq!(
        h.node("host-change-review-stale").text.as_deref(),
        Some(STALE)
    );
    assert_eq!(h.try_click("host-confirm"), Err(STALE.into()));
    assert!(h.find("host-preview").is_none(), "not a synthetic pool now");
    let review = editor(&h).review.clone();
    change::submit(&mut h.model);
    assert_eq!(editor(&h).error, Some(Error::Stale.to_string()));
    assert_eq!(editor(&h).review, review, "never rebuilt, never dropped");
    assert!(h.model.resource_operations.is_empty());
}

/// Hazard 5: an unknown value refuses; it is never read as false.
#[test]
fn unknown_multipathing_is_not_off() {
    let mut h = server();
    let id = host(&h);
    amend(&mut h, |i| {
        i.details.hosts.get_mut(&id).expect("details").multipathing = None;
    });
    h.invoke("server.properties");
    assert_eq!(h.node("host-unavailable").text.as_deref(), Some(MISSING));
    h.click("host-review");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.node("host-error").text.as_deref(), Some(MISSING));
}

/// Hazard 6: the checks of the memory draft run in a fixed order. The text
/// is judged before the host's total memory is asked for.
#[test]
fn the_memory_draft_is_judged_before_missing_host_data() {
    let mut h = server();
    let id = host(&h);
    amend(&mut h, |i| {
        i.details.hosts.get_mut(&id).expect("details").metrics = None;
    });
    // The command would refuse (nothing typed can supply the total), so the
    // editor is opened the way the Rune test did.
    assert_eq!(
        h.eligibility("server.control-domain-memory"),
        Eligibility::disabled(MISSING)
    );
    let object = host_target(&h).single().expect("server").clone();
    h.model.areas.host_admin.editor = Editor::capture(&h.model, &object, Kind::Memory);
    h.type_text("host-setting-value", "0");
    h.click("host-review");
    assert_eq!(
        h.node("host-error").text.as_deref(),
        Some("Enter a positive whole MiB memory value")
    );
    h.type_text("host-setting-value", "3072");
    h.click("host-review");
    assert_eq!(h.node("host-error").text.as_deref(), Some(MISSING));
}

/// Hazard 7: Back leaves the review for the draft, which is kept.
#[test]
fn back_from_the_review_keeps_the_draft() {
    let mut h = Harness::scene("host-memory");
    assert_eq!(h.node("host-setting-value").value.as_deref(), Some("2048"));
    h.type_text("host-setting-value", "3072");
    h.submit("host-setting-value");
    assert!(editor(&h).review.is_some());
    assert!(h.find("host-setting-value").is_none());
    h.assert_unique_keys();
    h.click("host-back");
    assert!(editor(&h).review.is_none());
    assert_eq!(h.node("host-setting-value").value.as_deref(), Some("3072"));
}

/// Hazard 8: before Review is pressed the draft already shows the lines
/// the review will have and, if it would be refused, why.
#[test]
fn the_draft_previews_its_review_and_its_refusal() {
    let mut h = Harness::scene("host-memory");
    // 2048 MiB is what the control domain has: nothing to change.
    assert!(h.has_text("static maximum 2048 MiB → 2048 MiB"));
    assert_eq!(h.node("host-unavailable").text.as_deref(), Some(INVALID));
    h.type_text("host-setting-value", "3072");
    assert!(h.has_text("static maximum 2048 MiB → 3072 MiB"));
    assert!(h.find("host-unavailable").is_none());
    let preview = h.texts();
    h.click("host-review");
    // The review has the same lines, word for word.
    let w = editor(&h);
    for line in change::lines(w, w.review.as_ref().expect("review")) {
        assert!(preview.contains(&line), "{line}");
        assert!(h.has_text(&line), "{line}");
    }
}

/// Hazard 9: the closing line of every review is the Rune text. It names
/// "Activity" and "Pool administration", which the new information
/// architecture renames; it changes with every other module's, not here.
#[test]
fn the_closing_line_of_a_review_is_still_the_rune_text() {
    let h = Harness::scene("host-administration");
    let closing: Vec<_> = h
        .texts()
        .into_iter()
        .filter(|text| text.starts_with("Changes are journaled individually."))
        .collect();
    assert_eq!(
        closing,
        [
            "Changes are journaled individually. Stop remaining steps does not cancel in-flight \
             host work or undo completed changes. Use Activity for retained history and Pool \
             administration for unknown-outcome recovery."
        ]
    );
}
