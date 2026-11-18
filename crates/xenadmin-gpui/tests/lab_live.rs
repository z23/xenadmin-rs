//! Live scenarios of the GPUI front end against the explicitly designated
//! disposable lab: the real shell, store, intents and console windows over
//! the real model and core, on the Kit's headless platform.
//!
//! Every test is ignored by default. Run one at a time, each on a fresh
//! state directory:
//!
//! ```sh
//! XENADMIN_LAB_MANIFEST=/var/lib/libvirt/images/xenadmin-rs-lab/lab.json \
//! XENADMIN_LAB_PASSWORD_FILE=/path/to/lab-root-password \
//! XENADMIN_STATE_DIR=$(mktemp -d) \
//! cargo test -p xenadmin-gpui --features scenes --test lab_live -- \
//!     --ignored --exact NAME --nocapture --test-threads=1
//! ```
//!
//! Targets come only from the harness manifest, never from a saved profile
//! of this workstation: the state directory is `XENADMIN_STATE_DIR`, which
//! must be a temporary directory. Evidence (PNG frames, element trees and a
//! summary per scenario) is written under the manifest directory
//! (`evidence/gpui/`), outside the repository.
//!
//! The scenarios act as a user can: presses and typing on elements found by
//! their keys, commands through the menu bar, the toolbar, context menus
//! and local rows. What they read of the model is only for assertions.
//! What this does not establish is in `docs/reports/2026-11-19-gpui-live-headless.md`.
#![cfg(feature = "scenes")]

mod common;
mod lab_support;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lab_support::{Client, Evidence, Lab, Outside, Platform};
use serde_json::json;
use xenadmin_core::{
    OperationId, OperationState, PowerAction, PowerState, VmCommand, VmId, VmSettingEdit,
    VmSettingsPatch,
};
use xenadmin_gpui::shell::status;
use xenadmin_ui::command::Menu;
use xenadmin_ui::nav::NavView;
use xenadmin_ui::object::ObjectRef;
use xenadmin_ui::tabs::TabId;

const PLAIN_GUEST: &str = "lab-guest-";
const TOOLS_GUEST: &str = "lab-tools-";

fn seconds(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10.).round() / 10.
}

fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The guest of the scope session whose name starts so.
fn guest(client: &mut Client, prefix: &str) -> (ObjectRef, VmId, String) {
    client.model(|m| {
        let inventory = m.inventory.as_ref().expect("an inventory");
        let vm = inventory
            .guests()
            .find(|vm| vm.name.starts_with(prefix) && !vm.is_template)
            .unwrap_or_else(|| panic!("the pool has no guest {prefix}*"));
        (
            ObjectRef::vm(inventory.pool_id, vm.id),
            vm.id,
            vm.name.clone(),
        )
    })
}

/// What the status bar says of the connection.
fn connection_status(client: &mut Client) -> String {
    client
        .chrome(status::CONNECTION_KEY)
        .and_then(|element| element.label().map(str::to_owned))
        .unwrap_or_default()
}

/// Add Server from the Home tab, to the coordinator: the form, the
/// certificate inspection and its review, the pin and the connection.
/// Returns the fingerprint the review showed.
fn add_server(client: &mut Client, lab: &Lab, evidence: &mut Evidence) -> String {
    let inspected = lab_support::inspect(&lab.endpoint(0));
    let fingerprint = inspected.details.fingerprint.to_string();
    evidence.step(
        "inspected the coordinator's certificate through the core",
        json!({ "sha256": fingerprint }),
    );

    assert_eq!(client.model(|m| m.active_tab()), TabId::Home);
    client
        .try_local("server.add")
        .expect("Add Server is on the Home tab");
    assert_eq!(client.front().as_deref(), Some("connect-dialog"));
    client.type_into("host-endpoint", &lab.address(0));
    client.type_into("username", "root");
    lab_support::type_password(client, lab, "password");
    client.snap(evidence, "connect-form");
    let shown = client.node("password").expect("the password field");
    assert!(
        shown
            .value
            .as_deref()
            .is_none_or(|value| value.is_empty() || value.chars().all(|c| c == '•' || c == '*')),
        "the password field describes its text"
    );

    // The host's certificate is self-signed: it is inspected and reviewed
    // before anything is sent to the host.
    client.press("certificate-options");
    client.press("inspect-certificate");
    let took = client.wait(
        "the certificate review",
        Duration::from_secs(30),
        |client| client.element("certificate-review").is_some(),
    );
    assert!(
        client.describes(&fingerprint),
        "the review shows the fingerprint the core inspected"
    );
    assert!(client.says(&fingerprint), "the fingerprint is in the frame");
    client.snap(evidence, "certificate-review");
    evidence.step(
        "the certificate review shows the inspected fingerprint",
        json!({ "seconds": seconds(took) }),
    );
    let refused = client
        .try_press("accept-certificate")
        .expect_err("the pin needs the endpoint and the confirmation");
    evidence.step("Trust certificate is refused first", json!(refused));
    let endpoint = client.model(|m| {
        m.workspace
            .trust
            .observation
            .as_ref()
            .map(|observation| observation.endpoint.to_string())
            .expect("the observation")
    });
    client.type_into("confirm-certificate-endpoint", &endpoint);
    client.press("certificate-verified");
    client.press("accept-certificate");
    assert!(
        client.element("certificate-review").is_none(),
        "the form is back"
    );
    client.snap(evidence, "certificate-pinned");

    client.press("connect");
    let took = client.wait(
        "the connection is current",
        Duration::from_secs(120),
        |client| {
            client.model(|m| {
                m.inventory.is_some() && m.snapshot.as_ref().is_some_and(|s| s.is_current())
            })
        },
    );
    assert_eq!(client.front(), None, "the connect dialog closed");
    evidence.step("connected", json!({ "seconds": seconds(took) }));
    fingerprint
}

/// Gate G2: Add Server with a reviewed certificate, the inventory in the
/// tree, the status bar, and the General tab following a change made from
/// outside this client while the selection stays.
#[test]
#[ignore]
fn lab_g2_add_server_and_follow_the_host() {
    let lab = Lab::load();
    lab_support::assert_fresh_state();
    let mut evidence = Evidence::new(&lab, "g2-add-server-and-follow");
    let mut client = Client::start(Platform::Rendered);
    client.snap(&mut evidence, "first-start");
    assert!(
        client.model(|m| m.workspace.profiles().is_empty()),
        "a fresh state directory has no saved connection"
    );

    let fingerprint = add_server(&mut client, &lab, &mut evidence);
    evidence.note("certificate_sha256", json!(fingerprint));

    // The tree shows the pool, both servers and the guests the host has.
    let outside = Outside::connect(&lab);
    let expected = {
        let snapshot = outside.connection.latest();
        let inventory = snapshot.inventory().expect("outside inventory");
        let mut names = vec![inventory.pool_name.clone()];
        names.extend(inventory.hosts.values().map(|host| host.name.clone()));
        names.extend(
            inventory
                .guests()
                .filter(|vm| !vm.is_template)
                .map(|vm| vm.name.clone()),
        );
        names
    };
    let took = client.wait(
        "the tree shows the pool, the servers and the guests",
        Duration::from_secs(60),
        |client| {
            let labels = client.tree_labels();
            expected.iter().all(|name| labels.contains(name))
        },
    );
    let labels = client.tree_labels();
    for name in &expected {
        assert!(
            client.tree_shows(name),
            "the tree has no visible row named {name}"
        );
    }
    let said = connection_status(&mut client);
    assert!(
        said.contains("Connected"),
        "the status bar says {said:?}, not connected"
    );
    client.snap(&mut evidence, "connected");
    evidence.step(
        "the tree and the status bar show the pool",
        json!({ "seconds": seconds(took), "rows": labels, "status": said }),
    );
    evidence.note("tree_rows", json!(labels));
    evidence.note("status_bar", json!(said));

    // Inventory follows the host: a description changed from outside shows
    // on the General tab of the selected guest, and the selection stays.
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    client.tab(TabId::General);
    assert!(client.says(&name));
    let previous = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .vms[&vm]
        .description
        .clone();
    let changed = format!("Changed from outside the client at {}", stamp());
    let describe = |from: &str, to: &str| {
        let patch = VmSettingsPatch::new(vec![VmSettingEdit::Description {
            previous: from.to_owned(),
            value: to.to_owned(),
        }])
        .expect("patch");
        let observed = outside
            .connection
            .latest()
            .observe_vm(vm)
            .expect("observed guest");
        outside
            .connection
            .submit(
                OperationId::new(),
                VmCommand::Configure {
                    vm: observed,
                    patch,
                },
            )
            .expect("the outside edit is accepted")
    };
    client.snap(&mut evidence, "general-before");
    let edit = describe(&previous, &changed);
    let took = client.wait(
        "the General tab shows the description set from outside",
        Duration::from_secs(60),
        |client| client.says(&changed),
    );
    assert_eq!(
        client.model(|m| m.scope().single().cloned()),
        Some(object.clone()),
        "the selection stayed on the guest"
    );
    assert_eq!(client.model(|m| m.active_tab()), TabId::General);
    client.snap(&mut evidence, "general-after-outside-change");
    evidence.step(
        "the General tab followed a change made from outside",
        json!({ "seconds": seconds(took), "guest": name, "description": changed }),
    );
    evidence.note("followed_in_seconds", json!(seconds(took)));
    drop(edit);

    // Put back what was changed.
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.vms[&vm].description == changed
        }),
        "the outside connection read its own change"
    );
    let restore = describe(&changed, &previous);
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.vms[&vm].description == previous
        }),
        "the description was put back"
    );
    drop(restore);
    client.wait(
        "the General tab shows the description put back",
        Duration::from_secs(60),
        |client| !client.says(&changed),
    );
    client.snap(&mut evidence, "general-restored");
    evidence.note("description_restored", json!(true));

    // Nothing of the password is in the state directory.
    let leaked: Vec<String> = std::fs::read_dir(lab_support::state_directory())
        .expect("state directory")
        .flatten()
        .filter(|entry| lab_support::holds_password(&lab, &entry.path()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(leaked.is_empty(), "the password is in {leaked:?}");
    client.close();
    evidence.finish();
}

/// A client on the fresh state directory of this run, connected to the
/// coordinator through Add Server.
fn connected(lab: &Lab, evidence: &mut Evidence, platform: Platform) -> Client {
    lab_support::assert_fresh_state();
    let mut client = Client::start(platform);
    add_server(&mut client, lab, evidence);
    client
}

/// Tools > Options: "Review routine VM power actions" on, so that a
/// routine power action asks for its confirmation.
fn review_power_actions(client: &mut Client, evidence: &mut Evidence) {
    client.menu(Menu::Tools, "tools.options");
    assert_eq!(client.front().as_deref(), Some("settings-dialog"));
    client.press("settings-review-power");
    client.press("settings-apply");
    if client.front().is_some() {
        client.press("settings-close");
    }
    assert_eq!(client.front(), None);
    assert!(client.model(|m| m.preferences.options.review_power));
    evidence.step("Options: routine power actions are reviewed", json!(true));
}

fn power_state(client: &mut Client, vm: VmId) -> PowerState {
    client.model(|m| {
        m.inventory.as_ref().expect("inventory").vms[&vm]
            .power_state
            .clone()
    })
}

/// Seconds since the epoch for a time as XAPI writes it, `YYYYMMDDTHH:MM:SSZ`.
fn host_time(text: &str) -> Option<i64> {
    let digits = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    if text.len() != 18 || !text.is_ascii() || &text[8..9] != "T" || !text.ends_with('Z') {
        return None;
    }
    let (year, month, day) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    let (hour, minute, second) = (digits(9..11)?, digits(12..14)?, digits(15..17)?);
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// A running guest is given until its host offers a clean shutdown and
/// until 75 seconds after its start, as the core's live harness does: a
/// request sent before the guest's kernel listens is lost and the host
/// holds its task for many minutes.
fn wait_guest_ready(client: &mut Client, vm: VmId, evidence: &mut Evidence) {
    if power_state(client, vm) != PowerState::Running {
        return;
    }
    let took = client.wait(
        "the host offers a clean shutdown of the guest",
        Duration::from_secs(180),
        |client| {
            client.model(|m| {
                m.inventory.as_ref().expect("inventory").vms[&vm]
                    .allowed_actions
                    .contains(&PowerAction::CleanShutdown)
            })
        },
    );
    let age = client.model(|m| {
        let inventory = m.inventory.as_ref()?;
        let metrics = inventory.details.vms.get(&vm)?.metrics?;
        let started = host_time(
            inventory
                .details
                .vm_metrics
                .get(&metrics)?
                .start_time
                .as_deref()?,
        )?;
        Some(stamp() as i64 - started)
    });
    if let Some(age) = age.filter(|age| (0..75).contains(age)) {
        client.pass(Duration::from_secs((75 - age) as u64));
    }
    evidence.step(
        "the guest is ready for a clean shutdown",
        json!({ "seconds": seconds(took), "seconds_since_start": age }),
    );
}

/// The newest operation record of a guest.
fn newest_operation(client: &mut Client, vm: VmId) -> Option<(OperationId, OperationState)> {
    client.model(|m| {
        m.operation_rows
            .iter()
            .filter(|record| record.vm == vm)
            .max_by_key(|record| record.created_at_ms)
            .map(|record| (record.id, record.state.clone()))
    })
}

/// The row of an operation in the list the Events tab shows: its key and
/// the texts of its cells.
fn history_row(client: &mut Client, operation: OperationId) -> Option<(String, Vec<String>)> {
    let nodes = client.nodes();
    let id = operation.to_string();
    let at = nodes.iter().position(|node| {
        node.role == "row" && node.key.as_deref().is_some_and(|key| key.ends_with(&id))
    })?;
    let cells = nodes[at + 1..]
        .iter()
        .take_while(|node| node.role == "cell")
        .filter_map(|node| node.text.clone())
        .collect();
    Some((nodes[at].key.clone()?, cells))
}

/// Submits the open power confirmation and follows the operation to its
/// end: the record, the power state the host reports and what the General
/// tab says of it. Returns the operation and how long it took.
fn confirm_and_follow(
    client: &mut Client,
    evidence: &mut Evidence,
    vm: VmId,
    name: &str,
    expected: PowerState,
) -> (OperationId, f64) {
    assert_eq!(client.front().as_deref(), Some("power-confirm"));
    client.snap(evidence, &format!("{name}-confirmation"));
    let before = newest_operation(client, vm).map(|(id, _)| id);
    client.press("power-confirm-submit");
    client.wait(
        "the operation is recorded",
        Duration::from_secs(30),
        |client| newest_operation(client, vm).is_some_and(|(id, _)| Some(id) != before),
    );
    let (operation, state) = newest_operation(client, vm).expect("the record");
    let operations = client
        .chrome(status::OPERATIONS_KEY)
        .and_then(|element| element.label().map(str::to_owned));
    client.snap(evidence, &format!("{name}-in-progress"));
    evidence.step(
        "the operation shows where it was started",
        json!({ "operation": operation.to_string(), "state": format!("{state:?}"), "status_bar": operations }),
    );
    let took = client.wait(
        "the operation finished and the host reports the power state",
        Duration::from_secs(360),
        |client| {
            power_state(client, vm) == expected
                && newest_operation(client, vm).is_some_and(|(_, state)| state.terminal())
        },
    );
    let (_, state) = newest_operation(client, vm).expect("the record");
    assert_eq!(state, OperationState::Succeeded);
    let shown = client
        .node(&format!("vm-operation-{operation}"))
        .expect("the General tab shows the operation");
    let said = format!("{:?} {:?}", shown.label, shown.text);
    assert!(said.contains("Succeeded"), "the General tab says {said}");
    client.snap(evidence, &format!("{name}-outcome"));
    evidence.step(
        "the outcome shows where it was started",
        json!({ "seconds": seconds(took), "general_tab": shown.label, "power_state": format!("{expected:?}") }),
    );
    (operation, seconds(took))
}

/// P3.1 and P3.2: a refused action says why, Shut Down from the VM menu
/// through its confirmation, the progress and the outcome on the General
/// tab and in the Events tab of the Notifications view, Start from the
/// toolbar.
#[test]
#[ignore]
fn lab_p3_power_and_events() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-1-power-and-events");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    review_power_actions(&mut client, &mut evidence);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    client.tab(TabId::General);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    wait_guest_ready(&mut client, vm, &mut evidence);

    // A refused action shows its reason: Start on a running guest.
    let reason = client
        .try_menu(Menu::Vm, "vm.start")
        .expect_err("Start is disabled for a running guest");
    assert!(!reason.is_empty(), "the disabled item gives a reason");
    assert!(
        client.try_toolbar("vm.start").is_err(),
        "the toolbar offers no Start for a running guest"
    );
    evidence.step("Start is refused for a running guest", json!(reason));
    evidence.note("start_refused_with", json!(reason));

    // Shut Down from the VM menu, through its confirmation.
    client.menu(Menu::Vm, "vm.shutdown");
    let (shutdown, took) = confirm_and_follow(
        &mut client,
        &mut evidence,
        vm,
        "shut-down",
        PowerState::Halted,
    );
    evidence.note("shut_down_seconds", json!(took));

    // The Events tab of the Notifications view has the record.
    client.nav_view(NavView::Notifications);
    client.tab(TabId::Events);
    client.press("activity-History");
    let row = history_row(&mut client, shutdown).expect("the Events tab has the shutdown");
    assert_eq!(row.1, ["Shut down", "Succeeded"]);
    // The row is drawn, and choosing it shows the outcome of the record.
    client.press(&row.0);
    let outcome = client.node("activity-outcome").expect("the outcome");
    client.snap(&mut evidence, "events-history");
    evidence.step(
        "the Events tab has the record and its outcome",
        json!({ "row": row.1, "outcome": outcome.label, "lines": outcome.text }),
    );
    evidence.note("events_row", json!(row.1));

    // Start again, from the toolbar.
    client.nav_view(NavView::Infrastructure);
    client.select(&object);
    client.tab(TabId::General);
    client
        .try_toolbar("vm.start")
        .expect("the toolbar offers Start for a halted guest");
    let (_, took) =
        confirm_and_follow(&mut client, &mut evidence, vm, "start", PowerState::Running);
    evidence.note("start_seconds", json!(took));
    evidence.note("guest", json!(name));
    evidence.note(
        "power_batch",
        json!("not run: see the report (two halted guests need the tools guest stopped)"),
    );
    client.close();
    evidence.finish();
}

#[allow(dead_code)]
const _: &str = TOOLS_GUEST;
