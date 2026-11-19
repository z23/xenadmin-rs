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
//! cargo test -p xenadmin-desktop --features scenes --test lab_live -- \
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
    DiskPlacement, HostId, NetworkId, NicPlacement, OperationId, OperationState, PowerAction,
    PowerState, SnapshotId, SnapshotName, SrId, VmCommand, VmId, VmMigration, VmSettingEdit,
    VmSettingsPatch,
};
use xenadmin_desktop::charts;
use xenadmin_desktop::shell::status;
use xenadmin_ui::areas::console::{self, Seat};
use xenadmin_ui::areas::performance;
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

/// Inspects the certificate of a host of the manifest from the connect
/// form, compares what the review shows with what the core inspected, and
/// trusts it: the endpoint typed again and the confirmation checked.
/// Returns the fingerprint.
fn pin_certificate(
    client: &mut Client,
    lab: &Lab,
    evidence: &mut Evidence,
    index: usize,
) -> String {
    let fingerprint = lab_support::inspect(&lab.endpoint(index))
        .details
        .fingerprint
        .to_string();
    let host = lab.hosts[index].name.clone();
    evidence.step(
        "inspected a host's certificate through the core",
        json!({ "host": host, "sha256": fingerprint }),
    );
    client.type_into("inspect-endpoint", &lab.address(index));
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
    client.snap(evidence, &format!("certificate-review-{}", index + 1));
    evidence.step(
        "the certificate review shows the inspected fingerprint",
        json!({ "host": host, "seconds": seconds(took) }),
    );
    let refused = client
        .try_press("accept-certificate")
        .expect_err("the pin needs the endpoint and the confirmation");
    assert!(refused.contains("exact endpoint"), "{refused}");
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
    fingerprint
}

/// Add Server from the Home tab, to the coordinator: the form, the
/// certificate of each of the two servers inspected, reviewed and pinned
/// (consoles and migrations reach the member directly), and the
/// connection. Returns the coordinator's fingerprint as the review showed
/// it.
fn add_server(client: &mut Client, lab: &Lab, evidence: &mut Evidence) -> String {
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

    // The hosts' certificates are self-signed: each is inspected and
    // reviewed before anything is sent to a host.
    client.press("certificate-options");
    let fingerprint = pin_certificate(client, lab, evidence, 0);
    let member = pin_certificate(client, lab, evidence, 1);
    assert_ne!(fingerprint, member);
    client.snap(evidence, "certificates-pinned");

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
    evidence.note("power_batch", json!("its own scenario: lab_p3_power_batch"));
    client.close();
    evidence.finish();
}

/// The share of the pixels of a picture that are clearly red. The guest's
/// 4:3 picture is letterboxed in a wide canvas, so a screen that is all red
/// is about half of the canvas.
fn red_share(picture: &image::RgbaImage) -> f64 {
    let red = picture
        .pixels()
        .filter(|pixel| pixel[0] > 120 && pixel[1] < 60 && pixel[2] < 60)
        .count();
    red as f64 / (picture.width() * picture.height()).max(1) as f64
}

/// How many pixels differ between two pictures of one size.
fn differing(left: &image::RgbaImage, right: &image::RgbaImage) -> usize {
    if left.dimensions() != right.dimensions() {
        return usize::MAX;
    }
    left.pixels()
        .zip(right.pixels())
        .filter(|(a, b)| a != b)
        .count()
}

/// The picture the model holds of the tab's console: its generation and
/// its size.
fn console_frame(client: &mut Client) -> Option<(u64, u16, u16)> {
    client.model(|m| {
        console::canvas::frame(m, Seat::Tab)
            .map(|frame| (frame.generation, frame.width, frame.height))
    })
}

/// Waits until the Console tab draws a picture of its console.
fn wait_console_picture(client: &mut Client, what: &str) -> f64 {
    let window = client.window;
    let took = client.wait(what, Duration::from_secs(90), |client| {
        console_frame(client).is_some()
            && client
                .canvas(window, Seat::Tab)
                .is_some_and(|(uploaded, _, _)| uploaded > 0)
    });
    seconds(took)
}

/// P3.3, the rendered half: the Console tab of the plain guest opens its
/// console, a frame arrives and is drawn, keys typed through the canvas
/// reach the guest; the host text console of the second server opens.
#[test]
#[ignore]
fn lab_p3_console() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-3-console");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let window = client.window;
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    wait_guest_ready(&mut client, vm, &mut evidence);

    client.tab(TabId::Console);
    let took = wait_console_picture(&mut client, "the guest's console shows a picture");
    let (generation, width, height) = console_frame(&mut client).expect("the frame");
    client.snap(&mut evidence, "console-first-frame");
    evidence.step(
        "the console opened and a frame was drawn",
        json!({ "seconds": took, "guest": name, "width": width, "height": height }),
    );
    evidence.note("guest_frame", json!([width, height]));
    assert!(width >= 320 && height >= 200);
    let first = client
        .pixels_of(console::view::CANVAS)
        .expect("the canvas pixels");
    // The canvas shows the guest's picture, not an empty box: it has more
    // than one colour.
    let colours: std::collections::BTreeSet<[u8; 4]> =
        first.pixels().map(|pixel| pixel.0).take(200_000).collect();
    assert!(colours.len() > 1, "the canvas is one flat colour");

    // A press on the canvas gives it the keyboard; Return reaches the guest
    // and its screen changes.
    client.press(console::view::CANVAS);
    let (_, keyboard, sent) = client.canvas(window, Seat::Tab).expect("the canvas");
    assert!(keyboard, "the canvas has the keyboard after a press");
    client.key("enter");
    let took = client.wait(
        "the guest's screen changed after Return",
        Duration::from_secs(20),
        |client| {
            console_frame(client).is_some_and(|(now, _, _)| now != generation)
                && client
                    .pixels_of(console::view::CANVAS)
                    .is_some_and(|now| differing(&first, &now) > 0)
        },
    );
    let (_, _, forwarded) = client.canvas(window, Seat::Tab).expect("the canvas");
    assert!(forwarded > sent, "the canvas forwarded the key");
    client.snap(&mut evidence, "console-after-return");
    evidence.step(
        "Return changed the guest's screen",
        json!({ "seconds": seconds(took) }),
    );

    // The root shell of the lab guest: no password. A command that paints
    // the whole screen red shows that every key arrived, in order.
    // A run that failed may have left the console at a password prompt:
    // the Return above ends it, and the guest takes a few seconds to offer
    // the login again.
    client.pass(Duration::from_secs(5));
    client.key("enter");
    client.pass(Duration::from_secs(1));
    client.type_keys("root");
    client.key("enter");
    client.pass(Duration::from_secs(3));
    client.snap(&mut evidence, "console-logged-in");
    client.type_keys("printf '\\033[41m'; clear; echo typed through the canvas");
    client.key("enter");
    client.pass(Duration::from_secs(2));
    client.snap(&mut evidence, "console-command-typed");
    let took = client.wait(
        "the command painted the guest's screen red",
        Duration::from_secs(30),
        |client| {
            client
                .pixels_of(console::view::CANVAS)
                .is_some_and(|now| red_share(&now) > 0.3)
        },
    );
    let red = client
        .pixels_of(console::view::CANVAS)
        .map(|now| red_share(&now));
    client.snap(&mut evidence, "console-command-echo");
    evidence.step(
        "a command typed through the canvas ran in the guest",
        json!({ "seconds": seconds(took), "red_share_of_canvas": red }),
    );
    evidence.note("typed_command_red_share", json!(red));
    // Put the guest's console back: default colours, and logged out.
    client.type_keys("printf '\\033[0m'; clear; exit");
    client.key("enter");
    client.wait(
        "the guest's screen is back to its colours",
        Duration::from_secs(30),
        |client| {
            client
                .pixels_of(console::view::CANVAS)
                .is_some_and(|now| red_share(&now) < 0.02)
        },
    );
    client.pass(Duration::from_secs(2));
    client.snap(&mut evidence, "console-logged-out");

    // Send Ctrl+Alt+Del is offered and is NOT pressed.
    let reason = client.disabled_reason(None, "local:vm.send-ctrl-alt-del");
    evidence.step(
        "Send Ctrl+Alt+Del is on the row and was not pressed",
        json!({ "disabled": reason }),
    );
    // Release Input lets go of what is held in the guest. Whether the
    // canvas keeps the keyboard afterwards is recorded, not asserted: see
    // the report.
    client
        .try_local("console.release-input")
        .expect("Release Input");
    let (_, keyboard, _) = client.canvas(window, Seat::Tab).expect("the canvas");
    evidence.note("canvas_has_keyboard_after_release_input", json!(keyboard));

    // The host text console of the second server. Nobody logs in.
    let host = client.model(|m| {
        let inventory = m.inventory.as_ref().expect("inventory");
        let host = inventory
            .hosts
            .values()
            .find(|host| Some(host.id) != inventory.coordinator)
            .expect("a member");
        (
            ObjectRef::host(inventory.pool_id, host.id),
            host.name.clone(),
        )
    });
    client.select(&host.0);
    client.tab(TabId::Console);
    let outcome = client.wait_for(Duration::from_secs(60), &mut |client| {
        console_frame(client).is_some()
            && client
                .canvas(window, Seat::Tab)
                .is_some_and(|(uploaded, _, _)| uploaded > 0)
    });
    client.snap(&mut evidence, "host-console");
    let said: Vec<String> = [
        console::view::STATUS,
        console::view::ERROR,
        console::view::MESSAGE,
    ]
    .into_iter()
    .filter_map(|key| client.node(key).and_then(|node| node.text))
    .collect();
    evidence.step(
        "the host text console",
        json!({ "host": host.1, "picture_after_seconds": outcome.map(seconds), "says": said }),
    );
    assert!(
        outcome.is_some(),
        "the host text console showed no picture: {said:?}"
    );
    let (_, width, height) = console_frame(&mut client).expect("the frame");
    evidence.note("host_console_frame", json!([width, height]));
    client.try_local("console.close").expect("Close Console");
    client.close();
    evidence.finish();
}

/// P3.3, the half that needs a path prompt: Save Screenshot of the guest's
/// console writes a PNG of the guest's picture to the path the test
/// platform's prompt gives. No frame of the window is captured here: the
/// platform that answers prompts has no renderer.
#[test]
#[ignore]
fn lab_p3_console_save_screenshot() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-3-console-save-screenshot");
    let mut client = connected(&lab, &mut evidence, Platform::Prompts);
    let (object, _, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    client.tab(TabId::Console);
    let took = wait_console_picture(&mut client, "the guest's console shows a picture");
    let (_, width, height) = console_frame(&mut client).expect("the frame");
    client.snap(&mut evidence, "console");
    evidence.step(
        "the console opened and a frame was drawn",
        json!({ "seconds": took, "guest": name, "width": width, "height": height }),
    );

    // Cancelled: nothing is written and the command can be used again.
    client
        .try_local("console.save-screenshot")
        .expect("Save Screenshot");
    client.wait("the path prompt", Duration::from_secs(5), |client| {
        client.asked_for_new_path()
    });
    client.answer_new_path(None);
    let scratch = xenadmin_ui::testing::TempDir::new("gpui-lab-screenshot");
    let path = scratch.path().join("guest-console.png");
    client.wait(
        "Save Screenshot can be used again after a cancel",
        Duration::from_secs(10),
        |client| client.try_local("console.save-screenshot").is_ok(),
    );
    client.wait("the path prompt", Duration::from_secs(5), |client| {
        client.asked_for_new_path()
    });
    client.answer_new_path(Some(path.clone()));
    let took = client.wait("the PNG is written", Duration::from_secs(20), |_| {
        std::fs::read(&path).is_ok_and(|bytes| bytes.starts_with(b"\x89PNG"))
    });
    let bytes = std::fs::read(&path).expect("the screenshot");
    let picture = image::load_from_memory(&bytes).expect("a PNG that decodes");
    assert_eq!(
        (picture.width(), picture.height()),
        (u32::from(width), u32::from(height)),
        "the screenshot has the size of the guest's picture"
    );
    let colours: std::collections::BTreeSet<[u8; 4]> = picture
        .to_rgba8()
        .pixels()
        .map(|pixel| pixel.0)
        .take(400_000)
        .collect();
    assert!(colours.len() > 1, "the screenshot is one flat colour");
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    evidence.keep("guest-console-screenshot.png", &bytes);
    evidence.step(
        "Save Screenshot wrote a PNG through the path prompt",
        json!({ "seconds": seconds(took), "width": picture.width(), "height": picture.height(),
            "bytes": bytes.len(), "mode": format!("{mode:o}") }),
    );
    evidence.note("screenshot", json!([picture.width(), picture.height()]));
    evidence.note("screenshot_mode", json!(format!("{mode:o}")));
    client.snap(&mut evidence, "after-save");
    client.try_local("console.close").expect("Close Console");
    client.close();
    evidence.finish();
}

/// The key of the row of a table that has a cell with the text.
fn table_row(client: &mut Client, table: &str, text: &str) -> Option<String> {
    let nodes = client.nodes();
    let prefix = format!("{table}:");
    nodes.iter().enumerate().find_map(|(at, node)| {
        let key = node.key.as_deref().filter(|key| key.starts_with(&prefix))?;
        (node.role == "row"
            && nodes[at + 1..]
                .iter()
                .take_while(|cell| cell.role == "cell")
                .any(|cell| cell.text.as_deref().is_some_and(|cell| cell.contains(text))))
        .then(|| key.to_owned())
    })
}

/// Brings a checkbox of the surface in front to the state wanted.
fn set_checkbox(client: &mut Client, key: &str, wanted: bool) {
    let checked = client
        .node(key)
        .unwrap_or_else(|| panic!("no checkbox {key}"))
        .checked;
    if checked != Some(wanted) {
        client.press(key);
    }
    assert_eq!(client.node(key).and_then(|node| node.checked), Some(wanted));
}

/// The operations of a guest that are not finished.
fn unfinished(client: &mut Client, vm: VmId) -> usize {
    client.model(|m| {
        m.operation_rows
            .iter()
            .filter(|record| record.vm == vm && !record.state.terminal())
            .count()
    })
}

/// The states of the operations of a guest, oldest first.
fn operation_states(client: &mut Client, vm: VmId) -> Vec<String> {
    client.model(|m| {
        let mut records: Vec<_> = m
            .operation_rows
            .iter()
            .filter(|record| record.vm == vm)
            .collect();
        records.sort_by_key(|record| record.created_at_ms);
        records
            .iter()
            .map(|record| format!("{} {:?}", record.action.label(), record.state))
            .collect()
    })
}

/// Sets the description of a guest from outside the client and waits until
/// the outside connection reads it back.
fn describe_from_outside(outside: &Outside, vm: VmId, value: &str) {
    let snapshot = outside.connection.latest();
    let previous = snapshot.inventory().expect("inventory").vms[&vm]
        .description
        .clone();
    let patch = VmSettingsPatch::new(vec![VmSettingEdit::Description {
        previous,
        value: value.to_owned(),
    }])
    .expect("patch");
    let observed = snapshot.observe_vm(vm).expect("observed guest");
    let operation = outside
        .connection
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm: observed,
                patch,
            },
        )
        .expect("the outside edit is accepted");
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.vms[&vm].description == value
        }),
        "the outside connection read its own change"
    );
    drop(operation);
}

/// P3.4: Take a Snapshot, see it in the Snapshots tab, Revert to it through
/// its review (the guest ends with what the snapshot had, and running),
/// Delete it.
#[test]
#[ignore]
fn lab_p3_snapshots() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-4-snapshots");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, guest_name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    wait_guest_ready(&mut client, vm, &mut evidence);
    client.tab(TabId::Snapshots);
    let before = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .snapshots
        .for_vm(vm)
        .count();
    let original = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .vms[&vm]
        .description
        .clone();
    client.snap(&mut evidence, "snapshots-before");

    // Take a Snapshot, from the VM menu.
    let name = format!("gpui-live-{}", stamp());
    client.menu(Menu::Vm, "vm.snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-create"));
    client.type_into("snapshot-name", &name);
    set_checkbox(&mut client, "snapshot-memory", false);
    client.snap(&mut evidence, "snapshot-create");
    client.press("confirm-snapshot");
    let took = client.wait(
        "the snapshot is in the Snapshots tab",
        Duration::from_secs(180),
        |client| {
            client.front().is_none()
                && table_row(client, "snapshots-table", &name).is_some()
                && unfinished(client, vm) == 0
        },
    );
    let row = table_row(&mut client, "snapshots-table", &name).expect("the row");
    assert!(client.says(&name), "the snapshot's name is in the frame");
    client.snap(&mut evidence, "snapshot-taken");
    evidence.step(
        "the snapshot was taken and is listed",
        json!({ "seconds": seconds(took), "name": name, "operations": operation_states(&mut client, vm) }),
    );
    evidence.note("take_seconds", json!(seconds(took)));

    // Something the snapshot does not have: a description set afterwards.
    let later = format!("Set after the snapshot {name}");
    describe_from_outside(&outside, vm, &later);
    client.wait(
        "the client read the description set after the snapshot",
        Duration::from_secs(60),
        |client| {
            client.model(|m| m.inventory.as_ref().expect("inventory").vms[&vm].description == later)
        },
    );

    // Revert, through its review: shut down first, start afterwards.
    client.press(&row);
    client
        .try_local("snapshot.revert")
        .expect("Revert for the selected snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-restore"));
    assert!(client.element("snapshot-review").is_some());
    set_checkbox(&mut client, "restore-shutdown", true);
    set_checkbox(&mut client, "restore-start", true);
    client.snap(&mut evidence, "snapshot-revert-review");
    client.press("confirm-snapshot");
    client.wait("the revert started", Duration::from_secs(30), |client| {
        client.front().is_none() && unfinished(client, vm) > 0
    });
    client.snap(&mut evidence, "snapshot-revert-in-progress");
    let took = client.wait(
        "the revert finished and the guest runs again",
        Duration::from_secs(420),
        |client| unfinished(client, vm) == 0 && power_state(client, vm) == PowerState::Running,
    );
    let states = operation_states(&mut client, vm);
    let restored = client.model(|m| {
        m.inventory.as_ref().expect("inventory").vms[&vm]
            .description
            .clone()
    });
    client.snap(&mut evidence, "snapshot-reverted");
    evidence.step(
        "the revert finished",
        json!({ "seconds": seconds(took), "operations": states, "description": restored }),
    );
    assert!(
        states.iter().all(|state| state.ends_with("Succeeded")),
        "an operation did not succeed: {states:?}"
    );
    assert_eq!(
        restored, original,
        "the guest has the description the snapshot had, not the one set afterwards"
    );
    evidence.note("revert_seconds", json!(seconds(took)));
    evidence.note("reverted_to_snapshot_state", json!(true));

    // Delete the snapshot, with the disks only it owns.
    let row = table_row(&mut client, "snapshots-table", &name).expect("the row after the revert");
    client.press(&row);
    client
        .try_local("snapshot.delete")
        .expect("Delete for the selected snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-delete"));
    if client.node("snapshot-delete-disks").is_some() {
        set_checkbox(&mut client, "snapshot-delete-disks", true);
    }
    client.snap(&mut evidence, "snapshot-delete-review");
    client.press("confirm-snapshot");
    let took = client.wait(
        "the snapshot is gone from the Snapshots tab",
        Duration::from_secs(240),
        |client| {
            client.front().is_none()
                && table_row(client, "snapshots-table", &name).is_none()
                && unfinished(client, vm) == 0
        },
    );
    client.snap(&mut evidence, "snapshot-deleted");
    let states = operation_states(&mut client, vm);
    evidence.step(
        "the snapshot was deleted",
        json!({ "seconds": seconds(took), "operations": states }),
    );
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.snapshots.for_vm(vm).count() == before
                && !inventory
                    .snapshots
                    .iter()
                    .any(|snapshot| snapshot.name == name)
        }),
        "the host still has the snapshot"
    );
    assert!(
        states.iter().all(|state| state.ends_with("Succeeded")),
        "{states:?}"
    );
    evidence.note("delete_seconds", json!(seconds(took)));
    evidence.note("guest", json!(guest_name));
    evidence.note("snapshots_left_of_this_run", json!(0));
    client.close();
    evidence.finish();
}

/// The Performance tab of the selected object: waits for charts with real
/// samples, and checks what the chart element computed and drew of them.
fn performance_of(client: &mut Client, evidence: &mut Evidence, what: &str) -> serde_json::Value {
    client.tab(TabId::Performance);
    client.snap(evidence, &format!("performance-{what}-opened"));
    let took = client.wait(
        "the Performance tab has a chart with samples",
        Duration::from_secs(180),
        |client| {
            client.model(|m| {
                performance::charts(m).iter().any(|chart| {
                    chart
                        .plot
                        .as_ref()
                        .is_some_and(|plot| plot.segments.iter().map(Vec::len).sum::<usize>() >= 2)
                })
            })
        },
    );
    let charts = client.model(performance::charts);
    let with_samples: Vec<_> = charts.iter().filter(|chart| chart.plot.is_some()).collect();
    // The first chart with samples: what the element painted is the
    // geometry of the model's plot, at the size of the element in the frame.
    let chart = with_samples[0];
    let plot = chart.plot.as_ref().expect("a plot");
    let samples: usize = plot.segments.iter().map(Vec::len).sum();
    let key = format!("{}{}", chart.key, charts::PLOT);
    client
        .on(|window, cx| common::reach(window, cx, None, &key).map(|_| ()))
        .unwrap_or_else(|error| panic!("the plot cannot be brought into view: {error}"));
    let painted = client
        .painted_chart(&chart.key)
        .expect("the chart element painted its plot");
    let drawn: usize =
        painted.geometry.lines.iter().map(Vec::len).sum::<usize>() + painted.geometry.marks.len();
    assert_eq!(
        drawn, samples,
        "every sample of the plot is a point of the painted geometry"
    );
    assert_eq!(
        painted.geometry,
        charts::geometry(plot, painted.geometry.width, painted.geometry.height)
    );
    assert!(painted.geometry.width > 100. && painted.geometry.height == charts::PLOT_HEIGHT);
    let summary = client
        .chrome(&format!("{}{}", chart.key, charts::SUMMARY))
        .and_then(|element| element.label().map(str::to_owned))
        .expect("the summary line of the card");
    assert_eq!(summary, chart.summary);
    assert!(summary.starts_with("Latest:"), "{summary}");
    assert_ne!(plot.first, plot.last, "the samples span time");
    client.snap(evidence, &format!("performance-{what}"));
    let result = json!({
        "seconds_to_first_samples": seconds(took),
        "charts": charts.len(),
        "charts_with_samples": with_samples.len(),
        "first_chart": chart.title,
        "summary": summary,
        "samples": samples,
        "strokes": painted.geometry.lines.len(),
        "first_sample": plot.first,
        "last_sample": plot.last,
        "high": plot.high,
        "low": plot.low,
    });
    evidence.step(
        "the Performance tab shows charts with real samples",
        result.clone(),
    );
    result
}

/// P3.5: the Performance tab of the plain guest and of the coordinator
/// shows charts with real samples.
#[test]
#[ignore]
fn lab_p3_performance() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-5-performance");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (object, _, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    let result = performance_of(&mut client, &mut evidence, "guest");
    evidence.note("guest", json!({ "name": name, "performance": result }));

    let host = client.model(|m| {
        let inventory = m.inventory.as_ref().expect("inventory");
        let id = inventory.coordinator.expect("a coordinator");
        (
            ObjectRef::host(inventory.pool_id, id),
            inventory.hosts[&id].name.clone(),
        )
    });
    client.select(&host.0);
    let result = performance_of(&mut client, &mut evidence, "server");
    evidence.note("server", json!({ "name": host.1, "performance": result }));
    client.close();
    evidence.finish();
}

/// The rows of a table of what the window describes: the key of each and
/// the texts of its cells.
fn table_rows(client: &mut Client, table: &str) -> Vec<(String, Vec<String>)> {
    let nodes = client.nodes();
    let prefix = format!("{table}:");
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.role == "row"
                && node
                    .key
                    .as_deref()
                    .is_some_and(|key| key.starts_with(&prefix))
        })
        .map(|(at, node)| {
            (
                node.key.clone().unwrap_or_default(),
                nodes[at + 1..]
                    .iter()
                    .take_while(|cell| cell.role == "cell")
                    .map(|cell| cell.text.clone().unwrap_or_default())
                    .collect(),
            )
        })
        .collect()
}

/// Every file under a directory.
fn files_under(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// The Home tab: nothing selected, the list of saved connections.
fn show_home(client: &mut Client) {
    if client.model(|m| m.active_tab()) != TabId::Home || !client.model(|m| m.scope().is_empty()) {
        client
            .try_toolbar("nav.back")
            .expect("Back leads to the Home tab");
    }
    assert!(
        client.model(|m| m.scope().is_empty()),
        "nothing is selected"
    );
    if client.model(|m| m.active_tab()) != TabId::Home {
        client.tab(TabId::Home);
    }
}

/// Connect… for the one saved connection, from the Home tab: the form
/// opens on the saved settings and asks for the password, which is not
/// saved; without it Connect is refused. With it the connection comes up,
/// and no certificate is reviewed again: the pins were saved.
fn connect_saved(client: &mut Client, lab: &Lab, evidence: &mut Evidence, tag: &str) -> f64 {
    show_home(client);
    let rows = table_rows(client, "connections");
    assert_eq!(rows.len(), 1, "one saved connection: {rows:?}");
    client.press(&rows[0].0);
    client
        .try_local("server.connect")
        .expect("Connect… for the saved connection");
    assert_eq!(client.front().as_deref(), Some("connect-dialog"));
    let endpoint = client.node("host-endpoint").and_then(|node| node.value);
    let username = client.node("username").and_then(|node| node.value);
    assert!(
        endpoint
            .as_deref()
            .is_some_and(|value| value.contains(&lab.hosts[0].ip)),
        "the form has the saved endpoint: {endpoint:?}"
    );
    assert_eq!(username.as_deref(), Some("root"));
    assert!(
        client.model(|m| m.workspace.password.is_empty()),
        "the password is asked for: none was saved"
    );
    client.press("connect");
    let refused = client
        .node("connect-error")
        .and_then(|node| node.text)
        .expect("Connect without a password is refused with a reason");
    client.snap(evidence, &format!("{tag}-password-asked"));
    evidence.step("the saved connection asks for the password", json!(refused));
    lab_support::type_password(client, lab, "password");
    client.press("connect");
    let took = client.wait(
        "the saved connection is current again",
        Duration::from_secs(120),
        |client| {
            client.front().is_none()
                && client.model(|m| {
                    m.inventory.is_some() && m.snapshot.as_ref().is_some_and(|s| s.is_current())
                })
        },
    );
    client.wait(
        "the Home tab says connected",
        Duration::from_secs(30),
        |client| {
            show_home(client);
            table_rows(client, "connections")
                .first()
                .is_some_and(|row| row.1.iter().any(|cell| cell == "Connected"))
        },
    );
    let said = connection_status(client);
    assert!(said.contains("Connected"), "the status bar says {said:?}");
    client.snap(evidence, &format!("{tag}-connected"));
    evidence.step(
        "connected again with the saved connection",
        json!({ "seconds": seconds(took), "status": said }),
    );
    seconds(took)
}

/// P3.8, what the lab's rules allow: Disconnect and Connect again from the
/// Home tab with the saved connection, and a second start on the same state
/// directory, which restores the saved connection as not connected and
/// asks for the password.
#[test]
#[ignore]
fn lab_p3_connection_recovery() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-8-connection-recovery");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let pool = client.model(|m| m.inventory.as_ref().expect("inventory").pool_name.clone());

    // Disconnect, from the Home tab.
    show_home(&mut client);
    let rows = table_rows(&mut client, "connections");
    assert_eq!(rows.len(), 1, "one saved connection: {rows:?}");
    assert!(rows[0].1.iter().any(|cell| cell == "Connected"), "{rows:?}");
    client.snap(&mut evidence, "home-connected");
    client.press(&rows[0].0);
    client
        .try_local("server.disconnect")
        .expect("Disconnect for the connected row");
    let took = client.wait(
        "the connection is closed",
        Duration::from_secs(60),
        |client| {
            table_rows(client, "connections")
                .first()
                .is_some_and(|row| row.1.iter().any(|cell| cell == "Disconnected"))
        },
    );
    let said = connection_status(&mut client);
    let labels = client.tree_labels();
    client.snap(&mut evidence, "home-disconnected");
    evidence.step(
        "disconnected from the Home tab",
        json!({ "seconds": seconds(took), "status": said, "tree": labels }),
    );
    assert!(
        !said.contains("Connected"),
        "the status bar still says {said:?}"
    );
    // The tree keeps the pool as it was last seen, under the status word
    // of its connection.
    let word = client.model(|m| {
        xenadmin_ui::nav::rows(m)
            .into_iter()
            .find(|row| row.label == pool)
            .and_then(|row| row.status)
    });
    assert_eq!(word.as_deref(), Some("Disconnected"));

    // Connect again with the saved connection.
    let again = connect_saved(&mut client, &lab, &mut evidence, "reconnect");
    evidence.note("reconnect_seconds", json!(again));
    assert!(client.tree_labels().contains(&pool));

    // A second start on the same state directory.
    client.close();
    let leaked: Vec<String> = files_under(&lab_support::state_directory())
        .into_iter()
        .filter(|path| lab_support::holds_password(&lab, path))
        .map(|path| path.display().to_string())
        .collect();
    assert!(leaked.is_empty(), "the password is in {leaked:?}");
    let mut client = Client::start(Platform::Rendered);
    client.pass(Duration::from_secs(3));
    let profiles = client.model(|m| m.workspace.profiles().len());
    assert_eq!(profiles, 1, "the saved connection is restored");
    assert!(
        client.model(|m| m.workspace.active.is_empty() && m.workspace.loads.is_empty()),
        "nothing connects by itself: the connection is not automatic and has no saved password"
    );
    show_home(&mut client);
    let rows = table_rows(&mut client, "connections");
    assert!(
        rows[0].1.iter().any(|cell| cell == "Disconnected"),
        "{rows:?}"
    );
    let said = connection_status(&mut client);
    let labels = client.tree_labels();
    client.snap(&mut evidence, "second-start");
    evidence.step(
        "a second start restores the saved connection as not connected",
        json!({ "row": rows[0].1, "status": said, "tree": labels }),
    );
    assert!(!said.contains("Connected"), "{said:?}");
    let after = connect_saved(&mut client, &lab, &mut evidence, "second-start");
    evidence.note("second_start_connect_seconds", json!(after));
    assert!(client.tree_labels().contains(&pool));
    evidence.note("password_in_state_directory", json!(false));
    client.close();
    evidence.finish();
}

/// A client closed in the middle of an operation: a Shut Down of the plain
/// guest is started, the whole application is closed while the operation
/// is in flight, a new one is started on the same state directory and
/// connected, and the Events tab has the operation's record, reconciled
/// with the host. Then the guest is started again.
#[test]
#[ignore]
fn lab_client_closed_in_the_middle_of_an_operation() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "closed-in-the-middle-of-an-operation");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    client.tab(TabId::General);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    wait_guest_ready(&mut client, vm, &mut evidence);

    // Shut Down, submitted at once (routine actions are not reviewed by
    // default), and the application closed while the host works on it.
    let before = newest_operation(&mut client, vm).map(|(id, _)| id);
    client.menu(Menu::Vm, "vm.shutdown");
    assert_eq!(client.front(), None, "no confirmation by default");
    client.wait(
        "the operation is in flight",
        Duration::from_secs(30),
        |client| {
            newest_operation(client, vm)
                .is_some_and(|(id, state)| Some(id) != before && !state.terminal())
        },
    );
    let (operation, state) = newest_operation(&mut client, vm).expect("the record");
    assert!(
        !state.terminal(),
        "the operation ended before the client closed: {state:?}"
    );
    client.snap(&mut evidence, "shutdown-in-flight");
    let guest_state = power_state(&mut client, vm);
    client.close();
    evidence.step(
        "the application was closed while the operation was in flight",
        json!({ "operation": operation.to_string(), "state_at_close": format!("{state:?}"),
            "guest_at_close": format!("{guest_state:?}") }),
    );
    evidence.note("state_at_close", json!(format!("{state:?}")));

    // The host finishes what it was asked, with no client connected.
    assert!(
        outside.settle(Duration::from_secs(300), |inventory| {
            inventory.vms[&vm].power_state == PowerState::Halted
        }),
        "the guest did not halt"
    );

    // A new application on the same state directory.
    let mut client = Client::start(Platform::Rendered);
    connect_saved(&mut client, &lab, &mut evidence, "after-restart");
    let took = client.wait(
        "the record of the operation is reconciled",
        Duration::from_secs(180),
        |client| {
            client.model(|m| {
                m.operation_rows
                    .iter()
                    .any(|record| record.id == operation && record.state.terminal())
            })
        },
    );
    let records: Vec<(String, String, String)> = client.model(|m| {
        m.operation_rows
            .iter()
            .filter(|record| record.vm == vm)
            .map(|record| {
                (
                    record.id.to_string(),
                    record.action.label().to_owned(),
                    format!("{:?}", record.state),
                )
            })
            .collect()
    });
    client.nav_view(NavView::Notifications);
    client.tab(TabId::Events);
    client.press("activity-History");
    let row = history_row(&mut client, operation).expect("the Events tab has the operation");
    client.press(&row.0);
    let outcome = client.node("activity-outcome").expect("the outcome");
    let details = client.node("activity-details").and_then(|node| node.text);
    client.snap(&mut evidence, "events-after-restart");
    evidence.step(
        "the Events tab has the reconciled record",
        json!({ "seconds": seconds(took), "row": row.1, "outcome": outcome.label,
            "details": details, "records_of_the_guest": records }),
    );
    // One record of the shutdown, the one that was submitted before the
    // close, and it says what the host did. A second request would be a
    // second record, or a failure of this one: the guest was halted.
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(row.1, ["Shut down", "Succeeded"]);
    assert_eq!(power_state(&mut client, vm), PowerState::Halted);
    assert_eq!(
        client.model(|m| m.status().unknown_outcomes),
        0,
        "no outcome is left unknown"
    );
    evidence.note("reconciled_as", json!(row.1));
    evidence.note("records_of_the_guest_after_restart", json!(records.len()));

    // Start the guest again, from the toolbar.
    client.nav_view(NavView::Infrastructure);
    client.select(&object);
    client.tab(TabId::General);
    client
        .try_toolbar("vm.start")
        .expect("the toolbar offers Start for a halted guest");
    let took = client.wait("the guest runs again", Duration::from_secs(360), |client| {
        power_state(client, vm) == PowerState::Running && unfinished(client, vm) == 0
    });
    let states = operation_states(&mut client, vm);
    client.snap(&mut evidence, "started-again");
    evidence.step(
        "the guest was started again",
        json!({ "seconds": seconds(took), "operations": states }),
    );
    assert!(
        states.iter().all(|state| state.ends_with("Succeeded")),
        "{states:?}"
    );
    evidence.note("guest", json!(name));
    client.close();
    evidence.finish();
}

/// The servers of the pool: the coordinator and the member, with their
/// names.
fn servers(client: &mut Client) -> ((HostId, String), (HostId, String)) {
    client.model(|m| {
        let inventory = m.inventory.as_ref().expect("inventory");
        let coordinator = inventory.coordinator.expect("a coordinator");
        let member = inventory
            .hosts
            .keys()
            .copied()
            .find(|id| *id != coordinator)
            .expect("a member");
        (
            (coordinator, inventory.hosts[&coordinator].name.clone()),
            (member, inventory.hosts[&member].name.clone()),
        )
    })
}

fn resident_host(client: &mut Client, vm: VmId) -> Option<HostId> {
    client.model(|m| m.inventory.as_ref().expect("inventory").vms[&vm].host)
}

/// The resource operations of the session that are not finished.
fn unfinished_resources(client: &mut Client) -> usize {
    client.model(|m| {
        m.resource_rows
            .iter()
            .filter(|record| !record.state.terminal())
            .count()
    })
}

fn resource_states(client: &mut Client) -> Vec<String> {
    client.model(|m| {
        m.resource_rows
            .iter()
            .map(|record| format!("{:?}", record.state))
            .collect()
    })
}

/// The lines of a review of the surface in front, one per line: the review
/// is a group, and each line is a text described under it.
fn review_lines(client: &mut Client, key: &str) -> String {
    let nodes = client.nodes();
    let Some(group) = nodes
        .iter()
        .find(|node| node.key.as_deref() == Some(key))
        .map(|node| format!("{}/", node.path))
    else {
        return String::new();
    };
    nodes
        .iter()
        .filter(|node| node.path.starts_with(&group))
        .filter_map(|node| node.text.clone().or_else(|| node.label.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Migrate to Server from the VM menu: the wizard's three steps with the
/// destination chosen and the storage and networks as offered, its review,
/// and the move followed until the host reports the guest on the
/// destination. Returns the seconds the move took.
fn migrate(
    client: &mut Client,
    evidence: &mut Evidence,
    object: &ObjectRef,
    vm: VmId,
    destination: (HostId, &str),
    tag: &str,
) -> f64 {
    client.select(object);
    client.tab(TabId::General);
    assert_eq!(power_state(client, vm), PowerState::Running);
    assert_ne!(resident_host(client, vm), Some(destination.0));
    client.menu(Menu::Vm, "vm.migrate");
    assert_eq!(client.front().as_deref(), Some("migration-dialog"));
    client.press(&format!("migration-host-{}", destination.0));
    client.snap(evidence, &format!("{tag}-destination"));
    client.press("migration-next");
    client.snap(evidence, &format!("{tag}-storage-and-networks"));
    client.press("migration-next");
    assert!(
        client.element("migration-review").is_some(),
        "the third step is the review"
    );
    let review = review_lines(client, "migration-review");
    client.snap(evidence, &format!("{tag}-review"));
    let before = newest_operation(client, vm).map(|(id, _)| id);
    client.press("migration-apply");
    client.wait(
        "the migration is recorded",
        Duration::from_secs(30),
        |client| newest_operation(client, vm).is_some_and(|(id, _)| Some(id) != before),
    );
    client.snap(evidence, &format!("{tag}-in-progress"));
    let took = client.wait(
        "the guest runs on the destination",
        Duration::from_secs(420),
        |client| {
            unfinished(client, vm) == 0
                && resident_host(client, vm) == Some(destination.0)
                && power_state(client, vm) == PowerState::Running
        },
    );
    let (_, state) = newest_operation(client, vm).expect("the record");
    assert_eq!(state, OperationState::Succeeded);
    if client.front().is_some() {
        // The wizard stays open on its outcome: close it as a user does.
        client.snap(evidence, &format!("{tag}-outcome"));
        client.key("escape");
    }
    client.wait("the wizard closed", Duration::from_secs(10), |client| {
        client.front().is_none()
    });
    assert!(
        client.says(destination.1),
        "the General tab names the destination server"
    );
    client.snap(evidence, &format!("{tag}-done"));
    evidence.step(
        "migrated through the wizard",
        json!({ "to": destination.1, "seconds": seconds(took), "review": review }),
    );
    seconds(took)
}

/// P3.7: Migrate to Server for the tools guest through the wizard and its
/// review, to the other server and back.
#[test]
#[ignore]
fn lab_p3_migration() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-7-migration");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (object, vm, name) = guest(&mut client, TOOLS_GUEST);
    let (coordinator, member) = servers(&mut client);
    let home = resident_host(&mut client, vm).expect("the tools guest runs");
    let (away, back) = if home == member.0 {
        (coordinator.clone(), member.clone())
    } else {
        (member.clone(), coordinator.clone())
    };
    evidence.step(
        "the tools guest before",
        json!({ "guest": name, "on": back.1, "moves_to": away.1 }),
    );
    let there = migrate(
        &mut client,
        &mut evidence,
        &object,
        vm,
        (away.0, &away.1),
        "there",
    );
    evidence.note("first_move", json!({ "to": away.1, "seconds": there }));
    let home_again = migrate(
        &mut client,
        &mut evidence,
        &object,
        vm,
        (back.0, &back.1),
        "back",
    );
    evidence.note(
        "second_move",
        json!({ "to": back.1, "seconds": home_again }),
    );
    assert_eq!(resident_host(&mut client, vm), Some(home));
    evidence.note("guest", json!(name));
    evidence.note("ends_on", json!(back.1));
    client.close();
    evidence.finish();
}

/// P3.6: Enter Maintenance Mode on the member from the Server menu through
/// its evacuation review, see it in maintenance, Exit Maintenance Mode, see
/// it enabled; the guest the evacuation moved is migrated back.
///
/// Before: the Xen Orchestra appliance is on the coordinator (the core
/// scenario `lab_move_xo_appliance`); the lab's rules do not let a member
/// in maintenance host it.
#[test]
#[ignore]
fn lab_p3_host_maintenance() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-6-host-maintenance");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (coordinator, member) = servers(&mut client);
    let (tools, tools_vm, tools_name) = guest(&mut client, TOOLS_GUEST);
    let tools_home = resident_host(&mut client, tools_vm);
    let residents: Vec<String> = client.model(|m| {
        let inventory = m.inventory.as_ref().expect("inventory");
        inventory
            .guests()
            .filter(|vm| vm.host == Some(member.0) && !vm.is_template)
            .map(|vm| vm.name.clone())
            .collect()
    });
    assert!(
        !residents.iter().any(|name| name == "XOA"),
        "XOA is on the member: move it to the coordinator first (core scenario lab_move_xo_appliance)"
    );
    evidence.step(
        "the member before",
        json!({ "server": member.1, "guests": residents }),
    );
    let host = client
        .model(|m| ObjectRef::host(m.inventory.as_ref().expect("inventory").pool_id, member.0));
    let enabled = |client: &mut Client| {
        client.model(|m| m.inventory.as_ref().expect("inventory").hosts[&member.0].enabled)
    };
    client.select(&host);
    client.tab(TabId::General);
    assert!(enabled(&mut client));
    assert_eq!(
        client
            .menu_label(Menu::Server, "server.maintenance")
            .as_deref(),
        Some("Enter Maintenance Mode")
    );

    // Enter, through the evacuation review.
    client.menu(Menu::Server, "server.maintenance");
    assert_eq!(client.front().as_deref(), Some("host-enter-maintenance"));
    for name in &residents {
        assert!(
            client.describes(&format!("Move guest: {name}")),
            "the review does not name the guest {name}"
        );
    }
    client.snap(&mut evidence, "enter-maintenance-review");
    client.press("host-confirm");
    client.wait(
        "the operation is recorded",
        Duration::from_secs(30),
        |client| client.front().is_none() && !resource_states(client).is_empty(),
    );
    client.snap(&mut evidence, "enter-maintenance-in-progress");
    let took = client.wait(
        "the member is in maintenance and its guests moved",
        Duration::from_secs(480),
        |client| {
            unfinished_resources(client) == 0
                && !enabled(client)
                && client.model(|m| {
                    !m.inventory
                        .as_ref()
                        .expect("inventory")
                        .guests()
                        .any(|vm| vm.host == Some(member.0) && !vm.is_template)
                })
        },
    );
    let states = resource_states(&mut client);
    let row = client.tree_row(&host).expect("the member's row");
    let header = client
        .chrome(xenadmin_desktop::scene_view::HEADER_KEY)
        .and_then(|element| element.label().map(str::to_owned));
    client.snap(&mut evidence, "in-maintenance");
    evidence.step(
        "the member is in maintenance",
        json!({ "seconds": seconds(took), "operations": states, "tree_row": row.status, "header": header,
            "tools_guest_on": if resident_host(&mut client, tools_vm) == Some(coordinator.0) { &coordinator.1 } else { &member.1 } }),
    );
    assert!(
        states.iter().all(|state| state == "Succeeded"),
        "{states:?}"
    );
    assert!(
        row.status
            .as_deref()
            .is_some_and(|status| !status.is_empty()),
        "the tree does not mark the member"
    );
    assert_eq!(
        client
            .menu_label(Menu::Server, "server.maintenance")
            .as_deref(),
        Some("Exit Maintenance Mode")
    );
    evidence.note("enter_seconds", json!(seconds(took)));
    evidence.note("tree_row_in_maintenance", json!(row.status));

    // Exit.
    client.menu(Menu::Server, "server.maintenance");
    assert_eq!(client.front().as_deref(), Some("host-exit-maintenance"));
    client.snap(&mut evidence, "exit-maintenance-review");
    client.press("host-confirm");
    let took = client.wait(
        "the member is enabled again",
        Duration::from_secs(240),
        |client| client.front().is_none() && unfinished_resources(client) == 0 && enabled(client),
    );
    let states = resource_states(&mut client);
    let row = client.tree_row(&host).expect("the member's row");
    client.snap(&mut evidence, "enabled-again");
    evidence.step(
        "the member is enabled again",
        json!({ "seconds": seconds(took), "operations": states, "tree_row": row.status }),
    );
    assert!(
        states.iter().all(|state| state == "Succeeded"),
        "{states:?}"
    );
    evidence.note("exit_seconds", json!(seconds(took)));

    // No evacuated guest is moved back automatically: this run moves back
    // what it moved.
    if residents.contains(&tools_name) && resident_host(&mut client, tools_vm) != Some(member.0) {
        let took = migrate(
            &mut client,
            &mut evidence,
            &tools,
            tools_vm,
            (member.0, &member.1),
            "tools-back",
        );
        evidence.note("tools_guest_moved_back_seconds", json!(took));
    }
    assert_eq!(
        resident_host(&mut client, tools_vm),
        tools_home,
        "the tools guest ends where it was"
    );
    client.close();
    evidence.finish();
}

/// The baseline of the lab has the tools guest on the coordinator. When it
/// is found on the member (an earlier run left it there), this moves it
/// with the application's own Migrate to Server wizard; otherwise it only
/// records where the guest is.
#[test]
#[ignore]
fn lab_baseline_tools_guest_on_the_coordinator() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "baseline-tools-guest-on-the-coordinator");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (object, vm, name) = guest(&mut client, TOOLS_GUEST);
    let (coordinator, _) = servers(&mut client);
    if resident_host(&mut client, vm) == Some(coordinator.0) {
        evidence.note("moved", json!(false));
    } else {
        let took = migrate(
            &mut client,
            &mut evidence,
            &object,
            vm,
            (coordinator.0, &coordinator.1),
            "to-the-coordinator",
        );
        evidence.note("moved", json!(true));
        evidence.note("seconds", json!(took));
    }
    assert_eq!(resident_host(&mut client, vm), Some(coordinator.0));
    evidence.note("guest", json!(name));
    evidence.note("ends_on", json!(coordinator.1));
    client.close();
    evidence.finish();
}

/// The maintenance scenario shows an evacuation only when a guest runs on
/// the member. When the tools guest is found on the coordinator, this moves
/// it to the member with the application's own Migrate to Server wizard;
/// otherwise it only records where the guest is. The baseline test above
/// moves it back after the maintenance scenario.
#[test]
#[ignore]
fn lab_place_tools_guest_on_the_member() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "place-tools-guest-on-the-member");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (object, vm, name) = guest(&mut client, TOOLS_GUEST);
    let (_, member) = servers(&mut client);
    if resident_host(&mut client, vm) == Some(member.0) {
        evidence.note("moved", json!(false));
    } else {
        let took = migrate(
            &mut client,
            &mut evidence,
            &object,
            vm,
            (member.0, &member.1),
            "to-the-member",
        );
        evidence.note("moved", json!(true));
        evidence.note("seconds", json!(took));
    }
    assert_eq!(resident_host(&mut client, vm), Some(member.0));
    evidence.note("guest", json!(name));
    evidence.note("ends_on", json!(member.1));
    client.close();
    evidence.finish();
}

/// Selects the two guests in the Search tab of their pool, with the grid
/// narrowed to VMs by its Filters so that their rows are drawn: a press
/// on the row of the first, a press with the secondary modifier on the row
/// of the second.
fn select_two(
    client: &mut Client,
    evidence: &mut Evidence,
    pool: &ObjectRef,
    first: &str,
    second: &str,
    tag: &str,
) {
    client.nav_view(NavView::Infrastructure);
    client.select(pool);
    client.tab(TabId::Search);
    if client.node("search-kind-VM").is_none() {
        client.press("search-filters");
    }
    set_checkbox(client, "search-kind-VM", true);
    client.press("search-apply-filters");
    client.wait(
        "the grid lists both guests",
        Duration::from_secs(30),
        |client| {
            table_row(client, "search-results", first).is_some()
                && table_row(client, "search-results", second).is_some()
        },
    );
    client.snap(evidence, &format!("{tag}-grid"));
    let row = table_row(client, "search-results", first).expect("the first row");
    client.press(&row);
    let row = table_row(client, "search-results", second).expect("the second row");
    client.press_adding(&row);
    client.snap(evidence, tag);
    let selected = client.model(|m| m.target().objects.len());
    assert_eq!(selected, 2, "two guests are selected");
}

/// A power command on the two selected guests: the batch opens on its
/// review with the action chosen and both guests listed, Run batch, and the
/// batch followed until the host reports the power state for both. Returns
/// the record of the batch and the seconds it took.
fn run_batch(
    client: &mut Client,
    evidence: &mut Evidence,
    command: &str,
    tag: &str,
    guests: [(VmId, &str); 2],
    expected: PowerState,
) -> (OperationId, f64) {
    client.menu(Menu::Vm, command);
    assert_eq!(client.front().as_deref(), Some("power-batch"));
    assert!(
        client.element("batch-review").is_some(),
        "the batch opens on its review"
    );
    for (_, name) in guests {
        assert!(
            table_row(client, "batch-vms", name).is_some(),
            "the review lists {name}"
        );
    }
    client.snap(evidence, &format!("{tag}-batch-review"));
    let before: Vec<OperationId> =
        client.model(|m| m.resource_rows.iter().map(|record| record.id).collect());
    client.press("batch-power-submit");
    client.wait("the batch is recorded", Duration::from_secs(30), |client| {
        client.front().is_none()
            && client.model(|m| {
                m.resource_rows
                    .iter()
                    .any(|record| !before.contains(&record.id))
            })
    });
    let batch = client.model(|m| {
        m.resource_rows
            .iter()
            .filter(|record| !before.contains(&record.id) && record.batch_parent.is_none())
            .min_by_key(|record| record.created_at_ms)
            .map(|record| record.id)
            .expect("the record of the batch")
    });
    client.snap(evidence, &format!("{tag}-batch-in-progress"));
    let took = client.wait(
        "the batch finished and the host reports the power state of both guests",
        Duration::from_secs(480),
        |client| {
            unfinished_resources(client) == 0
                && guests
                    .iter()
                    .all(|(vm, _)| power_state(client, *vm) == expected)
        },
    );
    let records: Vec<String> = client.model(|m| {
        m.resource_rows
            .iter()
            .filter(|record| !before.contains(&record.id))
            .map(|record| format!("{:?} {:?}: {}", record.action, record.state, record.detail))
            .collect()
    });
    let state = client.model(|m| {
        m.resource_rows
            .iter()
            .find(|record| record.id == batch)
            .map(|record| record.state.clone())
            .expect("the record")
    });
    client.snap(evidence, &format!("{tag}-batch-done"));
    evidence.step(
        "the power batch finished",
        json!({ "command": command, "seconds": seconds(took), "records": records }),
    );
    assert_eq!(state, OperationState::Succeeded, "{records:?}");
    (batch, seconds(took))
}

/// P3.1, the batch: Shut Down for the plain guest and the tools guest
/// selected together, then Start for the two halted guests, each through
/// the review of the batch; the Events tab has the batches. The tools guest
/// is stopped for the time of the two batches only, and is moved back when
/// the host starts it on the other server.
#[test]
#[ignore]
fn lab_p3_power_batch() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-1-power-batch");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (_, plain_vm, plain_name) = guest(&mut client, PLAIN_GUEST);
    let (tools, tools_vm, tools_name) = guest(&mut client, TOOLS_GUEST);
    let (coordinator, member) = servers(&mut client);
    let home = resident_host(&mut client, tools_vm).expect("the tools guest runs");
    let pool = client.model(|m| ObjectRef::pool(m.inventory.as_ref().expect("inventory").pool_id));
    for vm in [plain_vm, tools_vm] {
        assert_eq!(power_state(&mut client, vm), PowerState::Running);
        wait_guest_ready(&mut client, vm, &mut evidence);
    }
    let guests = [
        (plain_vm, plain_name.as_str()),
        (tools_vm, tools_name.as_str()),
    ];

    // Shut Down for both.
    select_two(
        &mut client,
        &mut evidence,
        &pool,
        &plain_name,
        &tools_name,
        "two-running-guests-selected",
    );
    let (shut_down, took) = run_batch(
        &mut client,
        &mut evidence,
        "vm.shutdown",
        "shut-down",
        guests,
        PowerState::Halted,
    );
    evidence.note("shut_down_batch_seconds", json!(took));

    // Start for the two halted guests.
    select_two(
        &mut client,
        &mut evidence,
        &pool,
        &plain_name,
        &tools_name,
        "two-halted-guests-selected",
    );
    let (start, took) = run_batch(
        &mut client,
        &mut evidence,
        "vm.start",
        "start",
        guests,
        PowerState::Running,
    );
    evidence.note("start_batch_seconds", json!(took));

    // The Events tab of the Notifications view has both batches.
    client.nav_view(NavView::Notifications);
    client.tab(TabId::Events);
    client.press("activity-History");
    let rows = table_rows(&mut client, "activity-table");
    let mut found = Vec::new();
    for batch in [shut_down, start] {
        let row = history_row(&mut client, batch).expect("the Events tab has the batch");
        assert!(row.1.iter().any(|cell| cell == "Succeeded"), "{row:?}");
        found.push(row.1);
    }
    client.snap(&mut evidence, "events-history");
    evidence.step(
        "the Events tab has the batches",
        json!({ "batches": found, "rows": rows.into_iter().map(|row| row.1).collect::<Vec<_>>() }),
    );
    evidence.note("events_rows", json!(found));

    // The host chooses where a started guest runs: the tools guest is moved
    // back when it is not where it was.
    client.nav_view(NavView::Infrastructure);
    if resident_host(&mut client, tools_vm) != Some(home) {
        wait_guest_ready(&mut client, tools_vm, &mut evidence);
        let back = if home == coordinator.0 {
            &coordinator
        } else {
            &member
        };
        let took = migrate(
            &mut client,
            &mut evidence,
            &tools,
            tools_vm,
            (back.0, &back.1),
            "tools-back",
        );
        evidence.note("tools_guest_moved_back_seconds", json!(took));
    }
    assert_eq!(resident_host(&mut client, tools_vm), Some(home));
    evidence.note("guests", json!([plain_name, tools_name]));
    client.close();
    evidence.finish();
}

/// Read-only: what the lab holds now, through a core connection of the
/// test's own (no window). Run before and after the scenarios to see the
/// baseline: both servers enabled, the guests running and where, and no
/// snapshot of a scenario left.
#[test]
#[ignore]
fn lab_state() {
    let lab = Lab::load();
    let outside = Outside::connect(&lab);
    let snapshot = outside.connection.latest();
    let inventory = snapshot.inventory().expect("inventory");
    println!("pool {}", inventory.pool_name);
    for host in inventory.hosts.values() {
        println!(
            "server {} enabled={} coordinator={}",
            host.name,
            host.enabled,
            inventory.coordinator == Some(host.id)
        );
    }
    for vm in inventory.guests() {
        let on = vm
            .host
            .and_then(|id| inventory.hosts.get(&id))
            .map_or("-", |host| host.name.as_str());
        println!(
            "guest {} {:?} on {on} ({:?})",
            vm.name, vm.power_state, vm.description
        );
    }
    for snapshot in inventory.snapshots.iter() {
        let of = snapshot
            .vm
            .and_then(|id| inventory.vms.get(&id))
            .map_or("-", |vm| vm.name.as_str());
        println!("snapshot {:?} of {of}", snapshot.name);
    }
    println!("snapshots {}", inventory.snapshots.len());
}

// ---------------------------------------------------------------------------
// The second run: the paths the first run left out
// (docs/reports/2026-11-19-gpui-live-paths.md)
// ---------------------------------------------------------------------------

/// When the host says the guest was started, as the host writes it.
fn started_at(client: &mut Client, vm: VmId) -> Option<String> {
    client.model(|m| {
        let inventory = m.inventory.as_ref()?;
        let metrics = inventory.details.vms.get(&vm)?.metrics?;
        inventory
            .details
            .vm_metrics
            .get(&metrics)?
            .start_time
            .clone()
    })
}

/// The record of an operation of a guest: its state and what it says.
fn operation_record(client: &mut Client, operation: OperationId) -> (OperationState, String) {
    client.model(|m| {
        m.operation_rows
            .iter()
            .find(|record| record.id == operation)
            .map(|record| (record.state.clone(), record.detail.clone()))
            .expect("the record")
    })
}

/// Waits for a new operation record of the guest (one that is not `before`)
/// and then for its end. The operation and the seconds from the press to
/// its end.
fn follow_new_operation(
    client: &mut Client,
    vm: VmId,
    before: Option<OperationId>,
    limit: Duration,
) -> (OperationId, f64) {
    let start = std::time::Instant::now();
    client.wait(
        "the operation is recorded",
        Duration::from_secs(30),
        |client| newest_operation(client, vm).is_some_and(|(id, _)| Some(id) != before),
    );
    let (operation, _) = newest_operation(client, vm).expect("the record");
    client.wait("the operation ended", limit, |client| {
        operation_record(client, operation).0.terminal()
    });
    (operation, seconds(start.elapsed()))
}

/// What the Events tab of the Notifications view shows of an operation:
/// the cells of its row and, with the row chosen, the outcome. Leaves the
/// Notifications view shown.
fn events_outcome(
    client: &mut Client,
    evidence: &mut Evidence,
    operation: OperationId,
    tag: &str,
) -> (Vec<String>, String) {
    client.nav_view(NavView::Notifications);
    client.tab(TabId::Events);
    client.press("activity-History");
    let row = history_row(client, operation).expect("the Events tab has the operation");
    client.press(&row.0);
    let outcome = client.node("activity-outcome").expect("the outcome");
    let details = client.node("activity-details");
    let said = format!(
        "{} {} {} {}",
        outcome.label.clone().unwrap_or_default(),
        outcome.value.clone().unwrap_or_default(),
        outcome.text.clone().unwrap_or_default(),
        details.and_then(|node| node.text).unwrap_or_default()
    );
    client.snap(evidence, tag);
    (row.1, said)
}

/// A press on a command of the VM menu, the confirmation, and the guest
/// followed to the power state. `restarted` asks that the host's start time
/// of the guest changed (a reboot is a new start).
fn power_through_confirmation(
    client: &mut Client,
    evidence: &mut Evidence,
    vm: VmId,
    command: &str,
    tag: &str,
    expected: PowerState,
    restarted: bool,
) -> f64 {
    let before = started_at(client, vm);
    client.menu(Menu::Vm, command);
    let title = client.model(|m| {
        xenadmin_ui::areas::power::view::surfaces(m)
            .first()
            .map(|surface| surface.title.clone())
    });
    let (_, took) = confirm_and_follow(client, evidence, vm, tag, expected);
    if restarted {
        client.wait(
            "the host reports a new start of the guest",
            Duration::from_secs(120),
            |client| started_at(client, vm) != before,
        );
    } else {
        assert_eq!(
            started_at(client, vm),
            before,
            "the guest was not restarted"
        );
    }
    evidence.step(
        "power action through its confirmation",
        json!({ "command": command, "confirmation": title, "seconds": took,
            "start_time_before": before, "start_time_after": started_at(client, vm) }),
    );
    evidence.note(&format!("{tag}_seconds"), json!(took));
    took
}

/// P3.1, what the first run left out for one guest: Suspend and Resume,
/// Reboot, Force Reboot and Force Shut Down through their confirmations
/// (the force actions are confirmed whatever the options say), and Start
/// on Server: refused by the host for the server that cannot reach the
/// guest's disks, with the host's reason shown, and accepted for the
/// server that can.
#[test]
#[ignore]
fn lab_p3_power_paths() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-1-power-paths");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    review_power_actions(&mut client, &mut evidence);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (coordinator, member) = servers(&mut client);
    client.select(&object);
    client.tab(TabId::General);
    if power_state(&mut client, vm) == PowerState::Halted {
        // An earlier run that failed left the guest halted.
        client
            .try_toolbar("vm.start")
            .expect("the toolbar offers Start for a halted guest");
        confirm_and_follow(
            &mut client,
            &mut evidence,
            vm,
            "start-first",
            PowerState::Running,
        );
    }
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    assert_eq!(resident_host(&mut client, vm), Some(coordinator.0));
    wait_guest_ready(&mut client, vm, &mut evidence);

    // Suspend and Resume.
    power_through_confirmation(
        &mut client,
        &mut evidence,
        vm,
        "vm.suspend",
        "suspend",
        PowerState::Suspended,
        false,
    );
    assert!(
        client.try_menu(Menu::Vm, "vm.shutdown").is_err(),
        "Shut Down is not offered for a suspended guest"
    );
    client
        .try_toolbar("vm.resume")
        .expect("the toolbar offers Resume for a suspended guest");
    let (_, took) = confirm_and_follow(
        &mut client,
        &mut evidence,
        vm,
        "resume",
        PowerState::Running,
    );
    evidence.note("resume_seconds", json!(took));
    assert_eq!(resident_host(&mut client, vm), Some(coordinator.0));

    // Reboot: the guest is asked, and starts anew.
    client.wait(
        "the host offers a clean reboot of the resumed guest",
        Duration::from_secs(120),
        |client| {
            client.model(|m| {
                m.inventory.as_ref().expect("inventory").vms[&vm]
                    .allowed_actions
                    .contains(&PowerAction::Reboot)
            })
        },
    );
    power_through_confirmation(
        &mut client,
        &mut evidence,
        vm,
        "vm.reboot",
        "reboot",
        PowerState::Running,
        true,
    );

    // Force Reboot and Force Shut Down: each through its confirmation,
    // which names the consequence. Review of routine actions is turned off
    // first, to show that the force actions are confirmed regardless.
    client.menu(Menu::Tools, "tools.options");
    client.press("settings-review-power");
    client.press("settings-apply");
    if client.front().is_some() {
        client.press("settings-close");
    }
    assert!(client.model(|m| !m.preferences.options.review_power));
    evidence.step(
        "Options: routine power actions are not reviewed",
        json!(true),
    );
    power_through_confirmation(
        &mut client,
        &mut evidence,
        vm,
        "vm.force-reboot",
        "force-reboot",
        PowerState::Running,
        true,
    );
    power_through_confirmation(
        &mut client,
        &mut evidence,
        vm,
        "vm.force-shutdown",
        "force-shut-down",
        PowerState::Halted,
        false,
    );
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.vms[&vm].power_state == PowerState::Halted
        }),
        "the host reports the guest halted to another client"
    );

    // Start on Server, the member: its review, and the host's refusal (the
    // guest's disks are on the coordinator's local repository).
    let place =
        |client: &mut Client, evidence: &mut Evidence, host: &(HostId, String), tag: &str| {
            let before = newest_operation(client, vm).map(|(id, _)| id);
            client.menu(Menu::Vm, "vm.start-on-server");
            assert_eq!(client.front().as_deref(), Some("vm-placement"));
            client.snap(evidence, &format!("{tag}-servers"));
            client.press(&format!("placement-host-{}", host.0));
            assert!(
                client.describes(&format!("Destination: {}", host.1)),
                "the review names the server"
            );
            client.snap(evidence, &format!("{tag}-review"));
            client.press("placement-confirm");
            let (operation, took) =
                follow_new_operation(client, vm, before, Duration::from_secs(360));
            assert_eq!(client.front(), None, "the dialog closed");
            (operation, took)
        };
    let (refused, took) = place(&mut client, &mut evidence, &member, "start-on-member");
    let (state, detail) = operation_record(&mut client, refused);
    assert_eq!(state, OperationState::Rejected, "{detail}");
    assert_eq!(power_state(&mut client, vm), PowerState::Halted);
    let shown = client
        .node(&format!("vm-operation-{refused}"))
        .expect("the General tab shows the operation");
    client.snap(&mut evidence, "start-on-member-refused");
    let (row, outcome) = events_outcome(
        &mut client,
        &mut evidence,
        refused,
        "start-on-member-refused-events",
    );
    evidence.step(
        "Start on Server was refused by the host",
        json!({ "server": member.1, "seconds": took, "record": detail,
            "general_tab": [shown.label, shown.text], "events_row": row, "events_outcome": outcome }),
    );
    assert!(row.iter().any(|cell| cell == "Rejected"), "{row:?}");
    assert!(
        !detail.is_empty() && outcome.contains(&detail),
        "the Events tab shows the host's reason: {outcome:?} does not hold {detail:?}"
    );
    evidence.note("start_on_member_refused_with", json!(detail));
    assert!(
        outside.settle(Duration::from_secs(10), |inventory| {
            inventory.vms[&vm].power_state == PowerState::Halted
        }),
        "the guest stays halted after the refusal"
    );

    // Start on Server, the coordinator: accepted.
    client.nav_view(NavView::Infrastructure);
    client.select(&object);
    client.tab(TabId::General);
    let (accepted, took) = place(
        &mut client,
        &mut evidence,
        &coordinator,
        "start-on-coordinator",
    );
    let (state, detail) = operation_record(&mut client, accepted);
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    client.wait(
        "the guest runs on the chosen server",
        Duration::from_secs(120),
        |client| {
            power_state(client, vm) == PowerState::Running
                && resident_host(client, vm) == Some(coordinator.0)
        },
    );
    client.snap(&mut evidence, "start-on-coordinator-done");
    evidence.step(
        "Start on Server was accepted",
        json!({ "server": coordinator.1, "seconds": took, "record": detail }),
    );
    evidence.note("start_on_coordinator_seconds", json!(took));
    evidence.note("operations", json!(operation_states(&mut client, vm)));
    evidence.note("guest", json!(name));
    client.close();
    evidence.finish();
}

// -- Changes made from outside, as another client makes them ---------------

/// The end of an operation of the outside connection.
fn outside_outcome(
    operation: &xenadmin_core::Operation,
    limit: Duration,
) -> (OperationState, String) {
    let start = std::time::Instant::now();
    loop {
        let row = operation.latest();
        if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
            return (row.state.clone(), row.detail.clone());
        }
        assert!(
            start.elapsed() < limit,
            "the outside operation did not end: {:?} {}",
            row.state,
            row.detail
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A disk snapshot of a guest, taken by the outside connection.
fn snapshot_from_outside(outside: &Outside, vm: VmId, name: &str) -> SnapshotId {
    let operation = outside
        .connection
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: outside.connection.latest().observe_vm(vm).expect("guest"),
                name: SnapshotName::new(name).expect("name"),
            },
        )
        .expect("the outside snapshot is accepted");
    let (state, detail) = outside_outcome(&operation, Duration::from_secs(300));
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.snapshots.for_vm(vm).any(|s| s.name == name)
        }),
        "the outside connection lists its snapshot"
    );
    outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .snapshots
        .for_vm(vm)
        .find(|s| s.name == name)
        .map(|s| s.id)
        .expect("the snapshot")
}

/// Deletes a snapshot with its disks through the outside connection.
fn delete_snapshot_from_outside(outside: &Outside, vm: VmId, snapshot: SnapshotId) {
    let latest = outside.connection.latest();
    let disks = latest
        .inventory()
        .expect("inventory")
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.snapshot == Some(snapshot) && vbd.kind == "Disk")
        .filter_map(|vbd| vbd.vdi)
        .collect::<Vec<_>>();
    let operation = outside
        .connection
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: latest.observe_snapshot(vm, snapshot).expect("snapshot"),
                disks,
            },
        )
        .expect("the outside deletion is accepted");
    let (state, detail) = outside_outcome(&operation, Duration::from_secs(300));
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.snapshots.for_vm(vm).all(|s| s.id != snapshot)
        }),
        "the snapshot is gone"
    );
}

/// A live move of a running guest to another server, its disks and
/// networks where they are, by the outside connection.
fn migrate_from_outside(outside: &Outside, vm: VmId, destination: HostId) {
    let latest = outside.connection.latest();
    let inventory = latest.inventory().expect("inventory");
    let disks = inventory
        .details
        .vm_disks
        .get(&vm)
        .into_iter()
        .flatten()
        .filter_map(|vbd| inventory.details.vbds.get(vbd))
        .filter(|vbd| vbd.kind == "Disk")
        .filter_map(|vbd| {
            let vdi = vbd.vdi?;
            let sr = inventory.details.vdis.get(&vdi)?.sr?;
            Some(DiskPlacement {
                vdi,
                expected_sr: sr,
                destination_sr: sr,
            })
        })
        .collect();
    let networks = inventory
        .details
        .vifs
        .values()
        .filter(|vif| vif.vm == Some(vm))
        .filter_map(|vif| {
            let network = vif.network?;
            Some(NicPlacement {
                vif: vif.id,
                expected_network: network,
                destination_network: network,
            })
        })
        .collect();
    let operation = VmMigration {
        destination,
        expected_host: inventory.vms[&vm].host,
        expected_affinity: inventory.details.vms.get(&vm).and_then(|v| v.affinity),
        live: true,
        disks,
        networks,
        transfer_network: None,
    };
    let handle = outside
        .connection
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: latest.observe_vm(vm).expect("guest"),
                operation,
            },
        )
        .expect("the outside migration is accepted");
    let (state, detail) = outside_outcome(&handle, Duration::from_secs(420));
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.vms[&vm].host == Some(destination)
        }),
        "the guest runs on the destination"
    );
}

// -- What the wizard needs of the inventory ---------------------------------

/// The local repository of a server: the one of kind `ext` that is not
/// shared and is plugged there.
fn local_repository(client: &mut Client, host: HostId) -> (SrId, String) {
    client.model(|m| {
        let inventory = m.inventory.as_ref().expect("inventory");
        inventory
            .details
            .pbds
            .values()
            .find_map(|pbd| {
                let sr = pbd.sr?;
                if pbd.host != Some(host)
                    || inventory.details.storage.get(&sr).and_then(|s| s.shared) != Some(false)
                {
                    return None;
                }
                inventory
                    .storage
                    .iter()
                    .find(|s| SrId::from(s.uuid) == sr && s.kind == "ext")
                    .map(|s| (sr, s.name.clone()))
            })
            .expect("a local repository")
    })
}

/// The network of the management interface of a server.
fn management_network(client: &mut Client, host: HostId) -> NetworkId {
    client.model(|m| {
        m.inventory
            .as_ref()
            .expect("inventory")
            .details
            .pifs
            .values()
            .find(|pif| pif.host == Some(host) && pif.management == Some(true))
            .and_then(|pif| pif.network)
            .expect("the management network")
    })
}

/// The repositories the disks of a guest are on, one per disk.
fn disk_repositories(client: &mut Client, vm: VmId) -> Vec<SrId> {
    client.model(|m| {
        let inventory = m.inventory.as_ref().expect("inventory");
        let mut disks: Vec<_> = inventory
            .details
            .vbds
            .values()
            .filter(|vbd| vbd.vm == Some(vm) && vbd.kind == "Disk")
            .filter_map(|vbd| vbd.vdi)
            .collect();
        disks.sort();
        disks.dedup();
        disks
            .into_iter()
            .filter_map(|vdi| inventory.details.vdis.get(&vdi).and_then(|d| d.sr))
            .collect()
    })
}

/// Picks an option of a drop-down of the surface in front: the list is
/// opened with the pointer and the option is reached with the Down key, as
/// many times as it takes until `chosen` holds (the drop-down shows the
/// name of its option, and two repositories can have one name).
#[track_caller]
fn pick(client: &mut Client, key: &str, chosen: impl Fn(&mut Client) -> bool) {
    // Down moves on from the option the list has now: one step a time
    // comes by every option. Should the list start from its top instead,
    // the second round reaches further each time.
    for downs in std::iter::repeat_n(1, 40).chain(2..=20) {
        if chosen(client) {
            return;
        }
        client.pick_with_keys(key, downs);
        client.pass(Duration::from_millis(200));
    }
    let shows = client.node(key).and_then(|node| node.value);
    panic!("the drop-down {key} never held the option wanted: it shows {shows:?}");
}

/// Migrate to Server from the VM menu, brought to its review: the
/// destination, and with a repository every disk placed on it and the
/// transfer over the destination's management network. `Err` with what the
/// wizard says when it does not reach the review. The lines of the review.
fn migration_to_review(
    client: &mut Client,
    evidence: &mut Evidence,
    object: &ObjectRef,
    vm: VmId,
    destination: HostId,
    repository: Option<SrId>,
    tag: &str,
) -> Result<String, String> {
    client.select(object);
    client.tab(TabId::General);
    client.menu(Menu::Vm, "vm.migrate");
    assert_eq!(client.front().as_deref(), Some("migration-dialog"));
    client.press(&format!("migration-host-{destination}"));
    client.press("migration-next");
    if let Some(repository) = repository {
        for index in 0..disk_repositories(client, vm).len() {
            pick(client, &format!("migration-disk-{index}"), |client| {
                client.model(|m| {
                    m.areas.migration.wizard.as_ref().is_some_and(|wizard| {
                        wizard.disks.get(index).map(|disk| disk.destination_sr) == Some(repository)
                    })
                })
            });
        }
        let transfer = management_network(client, destination);
        pick(client, "migration-transfer", |client| {
            client.model(|m| {
                m.areas
                    .migration
                    .wizard
                    .as_ref()
                    .is_some_and(|wizard| wizard.transfer == Some(transfer))
            })
        });
    }
    client.snap(evidence, &format!("{tag}-storage-and-networks"));
    client.press("migration-next");
    if client.element("migration-review").is_none() {
        let said = client
            .node("migration-error")
            .and_then(|node| node.text.or(node.label))
            .unwrap_or_default();
        client.snap(evidence, &format!("{tag}-not-reviewed"));
        return Err(said);
    }
    let review = review_lines(client, "migration-review");
    client.snap(evidence, &format!("{tag}-review"));
    Ok(review)
}

/// Each refused review that needs no migration to be running: a move of a
/// guest's disks while a snapshot exists; a revert whose review is not
/// complete, and one whose snapshot was deleted by another client while the
/// review was open; the maintenance review after the set of the server's
/// guests changed; Reboot and Shut Down of a server that is enabled.
/// Nothing is confirmed: every step ends with the refusal, and the lab is
/// left as it was found.
#[test]
#[ignore]
fn lab_p3_refused_reviews() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-refused-reviews");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (_, tools_vm, tools_name) = guest(&mut client, TOOLS_GUEST);
    let (coordinator, member) = servers(&mut client);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    assert_eq!(resident_host(&mut client, vm), Some(coordinator.0));
    assert_eq!(resident_host(&mut client, tools_vm), Some(coordinator.0));
    let repositories_before = disk_repositories(&mut client, vm);
    let (member_repository, member_repository_name) = local_repository(&mut client, member.0);

    // What an earlier run that failed left behind goes first.
    let leftovers: Vec<SnapshotId> = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .snapshots
        .for_vm(vm)
        .filter(|s| s.name.starts_with("gpui-refusal-"))
        .map(|s| s.id)
        .collect();
    for leftover in leftovers {
        delete_snapshot_from_outside(&outside, vm, leftover);
    }

    // 1. A move of the guest's disks while a snapshot exists.
    let snapshot_name = format!("gpui-refusal-{}", stamp());
    let snapshot = snapshot_from_outside(&outside, vm, &snapshot_name);
    client.wait(
        "the client lists the snapshot taken outside",
        Duration::from_secs(60),
        |client| {
            client.model(|m| {
                m.inventory
                    .as_ref()
                    .expect("inventory")
                    .snapshots
                    .for_vm(vm)
                    .any(|s| s.id == snapshot)
            })
        },
    );
    let reviewed = migration_to_review(
        &mut client,
        &mut evidence,
        &object,
        vm,
        member.0,
        Some(member_repository),
        "storage-move-with-a-snapshot",
    );
    let refusal = match reviewed {
        Err(said) => json!({ "where": "the wizard's second step", "reason": said }),
        Ok(review) => {
            match client.disabled_reason(Some("migration-dialog"), "migration-apply") {
                Some(reason) => {
                    json!({ "where": "the review: Apply migration is disabled", "reason": reason, "review": review })
                }
                None => {
                    // The review lets it through: the core or the host
                    // refuses it.
                    let before = newest_operation(&mut client, vm).map(|(id, _)| id);
                    client.press("migration-apply");
                    let (operation, took) =
                        follow_new_operation(&mut client, vm, before, Duration::from_secs(300));
                    let (state, detail) = operation_record(&mut client, operation);
                    assert_ne!(state, OperationState::Succeeded, "{detail}");
                    client.snap(&mut evidence, "storage-move-with-a-snapshot-refused");
                    json!({ "where": "after Apply", "state": format!("{state:?}"), "reason": detail, "seconds": took, "review": review })
                }
            }
        }
    };
    assert!(
        refusal["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "the refusal gives a reason: {refusal}"
    );
    if client.front().is_some() {
        client.press("migration-cancel");
    }
    assert_eq!(client.front(), None);
    assert_eq!(resident_host(&mut client, vm), Some(coordinator.0));
    assert_eq!(disk_repositories(&mut client, vm), repositories_before);
    evidence.step(
        "a move of the disks is refused while a snapshot exists",
        refusal.clone(),
    );
    evidence.note("storage_move_with_a_snapshot", refusal);
    evidence.note("storage_move_destination", json!(member_repository_name));

    // 2. A revert that is refused: first by its own review, then because
    // another client deleted the snapshot while the review was open.
    client.select(&object);
    client.tab(TabId::Snapshots);
    let row = table_row(&mut client, "snapshots-table", &snapshot_name).expect("the row");
    client.press(&row);
    client
        .try_local("snapshot.revert")
        .expect("Revert for the selected snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-restore"));
    set_checkbox(&mut client, "restore-shutdown", false);
    let incomplete = client
        .try_press("confirm-snapshot")
        .expect_err("a running guest is not reverted without its shutdown chosen");
    client.snap(&mut evidence, "revert-refused-without-shutdown");
    set_checkbox(&mut client, "restore-shutdown", true);
    assert_eq!(
        client.disabled_reason(Some("snapshot-restore"), "confirm-snapshot"),
        None,
        "with the shutdown chosen the review can be confirmed"
    );
    delete_snapshot_from_outside(&outside, vm, snapshot);
    client.wait(
        "the client reads that the snapshot is gone",
        Duration::from_secs(60),
        |client| {
            client.model(|m| {
                m.inventory
                    .as_ref()
                    .expect("inventory")
                    .snapshots
                    .for_vm(vm)
                    .all(|s| s.id != snapshot)
            })
        },
    );
    let stale = client
        .try_press("confirm-snapshot")
        .expect_err("a revert to a snapshot that is gone is refused");
    // The review itself says that what it shows is no longer current.
    let notice = "The connection is not ready or the selected object is no longer current";
    assert!(
        client.describes(notice) && client.says(notice),
        "the review says that the snapshot is no longer current"
    );
    client.snap(&mut evidence, "revert-refused-snapshot-gone");
    client.press("cancel-snapshot");
    assert_eq!(client.front(), None);
    assert_eq!(unfinished(&mut client, vm), 0);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    evidence.step(
        "a revert is refused",
        json!({ "without_shutdown_chosen": incomplete, "snapshot_deleted_outside": stale, "notice": notice }),
    );
    evidence.note("revert_refused_without_shutdown", json!(incomplete));
    evidence.note("revert_refused_snapshot_gone", json!(stale));

    // 3. Reboot and Shut Down of a server that is enabled.
    let host = client
        .model(|m| ObjectRef::host(m.inventory.as_ref().expect("inventory").pool_id, member.0));
    client.select(&host);
    client.tab(TabId::General);
    let reboot = client
        .try_menu(Menu::Server, "server.reboot")
        .expect_err("Reboot is refused for an enabled server");
    let shut_down = client
        .try_menu(Menu::Server, "server.shutdown")
        .expect_err("Shut Down is refused for an enabled server");
    assert_eq!(client.front(), None);
    evidence.step(
        "Reboot and Shut Down are refused for an enabled server",
        json!({ "server": member.1, "reboot": reboot, "shut_down": shut_down }),
    );
    evidence.note("reboot_refused_for_an_enabled_server", json!(reboot));
    evidence.note("shut_down_refused_for_an_enabled_server", json!(shut_down));

    // 4. The maintenance review after the server's guests changed: the
    // review is opened, another client moves a guest onto the server, and
    // the confirmation is refused.
    client.menu(Menu::Server, "server.maintenance");
    assert_eq!(client.front().as_deref(), Some("host-enter-maintenance"));
    assert!(
        !client.describes(&format!("Move guest: {tools_name}")),
        "the tools guest is not on the member when the review opens"
    );
    assert_eq!(
        client.disabled_reason(Some("host-enter-maintenance"), "host-confirm"),
        None,
        "the review can be confirmed as it opens"
    );
    client.snap(&mut evidence, "maintenance-review");
    migrate_from_outside(&outside, tools_vm, member.0);
    client.wait(
        "the client reads that the tools guest is on the member",
        Duration::from_secs(60),
        |client| resident_host(client, tools_vm) == Some(member.0),
    );
    let stale = client
        .try_press("host-confirm")
        .expect_err("the review of another set of guests is refused");
    client.snap(&mut evidence, "maintenance-review-stale");
    client.press("host-close");
    assert_eq!(client.front(), None);
    assert!(
        client.model(|m| m.inventory.as_ref().expect("inventory").hosts[&member.0].enabled),
        "the member stays enabled"
    );
    assert!(
        resource_states(&mut client).is_empty(),
        "nothing was submitted"
    );
    evidence.step(
        "the maintenance review is refused after the server's guests changed",
        json!({ "server": member.1, "moved_onto_it": tools_name, "reason": stale }),
    );
    evidence.note("maintenance_review_refused_when_stale", json!(stale));
    migrate_from_outside(&outside, tools_vm, coordinator.0);
    client.wait(
        "the tools guest is back on the coordinator",
        Duration::from_secs(60),
        |client| resident_host(client, tools_vm) == Some(coordinator.0),
    );
    evidence.note("guest", json!(name));
    evidence.note(
        "operations_of_this_client",
        json!(operation_states(&mut client, vm)),
    );
    client.close();
    evidence.finish();
}

/// The record of a resource operation of the session.
fn resource_record(
    client: &mut Client,
    operation: OperationId,
) -> (OperationState, String, Vec<String>) {
    client.model(|m| {
        m.resource_rows
            .iter()
            .find(|record| record.id == operation)
            .map(|record| {
                (
                    record.state.clone(),
                    record.detail.clone(),
                    record
                        .steps
                        .iter()
                        .map(|step| format!("{}: {:?} {}", step.label, step.state, step.detail))
                        .collect(),
                )
            })
            .expect("the record")
    })
}

/// P3.1: a batch that stops on a failure. Shut Down for the two guests as
/// a batch; while the host works on the first, another administrator
/// forces that guest off, which ends the batch's task for it without
/// success. The batch stops, the second guest is not asked to shut down,
/// and the window says so; the unknown outcome is acknowledged through the
/// Events tab and the first guest is started again.
#[test]
#[ignore]
fn lab_p3_power_batch_stops_on_a_failure() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-1-power-batch-stops-on-a-failure");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (plain, plain_vm, plain_name) = guest(&mut client, PLAIN_GUEST);
    let (tools, tools_vm, tools_name) = guest(&mut client, TOOLS_GUEST);
    let pool = client.model(|m| ObjectRef::pool(m.inventory.as_ref().expect("inventory").pool_id));
    for vm in [plain_vm, tools_vm] {
        assert_eq!(power_state(&mut client, vm), PowerState::Running);
        wait_guest_ready(&mut client, vm, &mut evidence);
    }
    let homes = [
        resident_host(&mut client, plain_vm),
        resident_host(&mut client, tools_vm),
    ];
    select_two(
        &mut client,
        &mut evidence,
        &pool,
        &plain_name,
        &tools_name,
        "two-running-guests-selected",
    );
    client.menu(Menu::Vm, "vm.shutdown");
    assert_eq!(client.front().as_deref(), Some("power-batch"));
    assert!(client.element("batch-review").is_some());
    let order: Vec<String> = table_rows(&mut client, "batch-vms")
        .into_iter()
        .map(|row| row.1.first().cloned().unwrap_or_default())
        .collect();
    assert_eq!(order.len(), 2, "{order:?}");
    let ((first, first_vm, first_name), (second, second_vm, second_name)) =
        if order[0] == plain_name {
            (
                (&plain, plain_vm, &plain_name),
                (&tools, tools_vm, &tools_name),
            )
        } else {
            (
                (&tools, tools_vm, &tools_name),
                (&plain, plain_vm, &plain_name),
            )
        };
    client.snap(&mut evidence, "batch-review");
    let before: Vec<OperationId> =
        client.model(|m| m.resource_rows.iter().map(|record| record.id).collect());
    client.press("batch-power-submit");
    client.wait("the batch is recorded", Duration::from_secs(30), |client| {
        client.front().is_none()
            && client.model(|m| {
                m.resource_rows
                    .iter()
                    .any(|record| !before.contains(&record.id))
            })
    });
    let batch = client.model(|m| {
        m.resource_rows
            .iter()
            .filter(|record| !before.contains(&record.id) && record.batch_parent.is_none())
            .min_by_key(|record| record.created_at_ms)
            .map(|record| record.id)
            .expect("the record of the batch")
    });

    // Another administrator forces the first guest off.
    let forced = outside
        .connection
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: outside
                    .connection
                    .latest()
                    .observe_vm(first_vm)
                    .expect("guest"),
                action: PowerAction::HardShutdown,
            },
        )
        .expect("the outside forced shutdown is accepted");
    let forced = outside_outcome(&forced, Duration::from_secs(120));
    evidence.step(
        "another administrator forced the first guest off",
        json!({ "guest": first_name, "state": format!("{:?}", forced.0), "detail": forced.1 }),
    );
    client.snap(&mut evidence, "batch-in-progress");
    let took = client.wait("the batch stopped", Duration::from_secs(480), |client| {
        let (state, _, _) = resource_record(client, batch);
        state.terminal() || state == OperationState::OutcomeUnknown
    });
    let (state, detail, steps) = resource_record(&mut client, batch);
    client.snap(&mut evidence, "batch-stopped");
    evidence.step(
        "the batch stopped",
        json!({ "seconds": seconds(took), "state": format!("{state:?}"), "detail": detail, "steps": steps,
            "status_bar": client.chrome(status::OPERATIONS_KEY).and_then(|e| e.label().map(str::to_owned)) }),
    );
    assert_ne!(state, OperationState::Succeeded, "{detail} {steps:?}");
    assert_eq!(steps.len(), 2, "{steps:?}");
    // The second guest was not asked: it runs, and no task of it was made.
    assert!(
        outside.settle(Duration::from_secs(5), |inventory| {
            inventory.vms[&second_vm].power_state == PowerState::Running
        }),
        "the second guest still runs"
    );
    assert_eq!(power_state(&mut client, second_vm), PowerState::Running);
    assert_eq!(power_state(&mut client, first_vm), PowerState::Halted);

    // What the Events tab says of it.
    client.nav_view(NavView::Notifications);
    client.tab(TabId::Events);
    client.press("activity-History");
    let row = history_row(&mut client, batch).expect("the Events tab has the batch");
    client.press(&row.0);
    let outcome = client.node("activity-outcome").expect("the outcome");
    let details = client
        .node("activity-details")
        .and_then(|node| node.text)
        .unwrap_or_default();
    let unresolved = client
        .node("activity-unresolved")
        .and_then(|node| node.text.or(node.label));
    client.snap(&mut evidence, "events-batch-stopped");
    evidence.step(
        "the Events tab shows the stopped batch",
        json!({ "row": row.1, "outcome": [outcome.label, outcome.value, outcome.text], "details": details, "unresolved": unresolved }),
    );
    let said = format!("{detail}\n{details}\n{}", steps.join("\n"));
    assert!(
        client.says("not submitted")
            || details.contains("not submitted")
            || client.describes("not submitted"),
        "the window says that the remaining guest was not submitted: {said}"
    );
    evidence.note("batch_state", json!(format!("{state:?}")));
    evidence.note("batch_detail", json!(detail));
    evidence.note("batch_steps", json!(steps));
    evidence.note("events_row", json!(row.1));

    // An unknown outcome keeps the pool reserved until it is acknowledged.
    if state == OperationState::OutcomeUnknown {
        client
            .try_local("events.acknowledge")
            .expect("Acknowledge for the selected unknown outcome");
        assert_eq!(client.front().as_deref(), Some("acknowledge-unknown"));
        client.snap(&mut evidence, "acknowledge-review");
        client.press("acknowledge-confirm");
        client.wait(
            "the batch is acknowledged",
            Duration::from_secs(60),
            |client| client.front().is_none() && resource_record(client, batch).0.terminal(),
        );
        let (state, detail, _) = resource_record(&mut client, batch);
        client.snap(&mut evidence, "acknowledged");
        evidence.step(
            "the unknown outcome was acknowledged",
            json!({ "state": format!("{state:?}"), "detail": detail }),
        );
        evidence.note("acknowledged_state", json!(format!("{state:?}")));
    }

    // The first guest is started again.
    client.nav_view(NavView::Infrastructure);
    client.select(first);
    client.tab(TabId::General);
    let before = newest_operation(&mut client, first_vm).map(|(id, _)| id);
    client
        .try_toolbar("vm.start")
        .expect("the toolbar offers Start for the halted guest");
    let (start, took) =
        follow_new_operation(&mut client, first_vm, before, Duration::from_secs(360));
    let (state, detail) = operation_record(&mut client, start);
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    client.wait("the guest runs", Duration::from_secs(120), |client| {
        power_state(client, first_vm) == PowerState::Running
    });
    evidence.step(
        "the first guest was started again",
        json!({ "seconds": took }),
    );
    // The host chooses where a started guest runs: back to where it was.
    for ((object, vm, home), name) in [(&plain, plain_vm, homes[0]), (&tools, tools_vm, homes[1])]
        .into_iter()
        .zip([&plain_name, &tools_name])
    {
        if resident_host(&mut client, vm) != home {
            wait_guest_ready(&mut client, vm, &mut evidence);
            let (coordinator, member) = servers(&mut client);
            let back = if home == Some(coordinator.0) {
                coordinator
            } else {
                member
            };
            let took = migrate(
                &mut client,
                &mut evidence,
                object,
                vm,
                (back.0, &back.1),
                "back",
            );
            evidence.note("moved_back", json!({ "guest": name, "seconds": took }));
        }
    }
    let _ = (second, second_name);
    evidence.note("guests", json!([first_name, second_name]));
    client.close();
    evidence.finish();
}

/// Presses Apply on the migration review and waits for the record of the
/// move. The operation.
fn apply_migration(
    client: &mut Client,
    evidence: &mut Evidence,
    vm: VmId,
    tag: &str,
) -> OperationId {
    let before = newest_operation(client, vm).map(|(id, _)| id);
    client.press("migration-apply");
    client.wait(
        "the migration is recorded",
        Duration::from_secs(60),
        |client| newest_operation(client, vm).is_some_and(|(id, _)| Some(id) != before),
    );
    assert_eq!(client.front(), None, "the wizard closed on Apply");
    client.snap(evidence, &format!("{tag}-in-progress"));
    newest_operation(client, vm).expect("the record").0
}

/// Whether the guest and every disk of it are where they are expected.
fn placed(client: &mut Client, vm: VmId, host: Option<HostId>, repository: SrId) -> bool {
    resident_host(client, vm) == host
        && disk_repositories(client, vm)
            .iter()
            .all(|sr| *sr == repository)
}

/// The same as another client reads it.
fn placed_outside(outside: &Outside, vm: VmId, host: Option<HostId>, repository: SrId) -> bool {
    outside.settle(Duration::from_secs(60), |inventory| {
        let disks: Vec<_> = inventory
            .details
            .vbds
            .values()
            .filter(|vbd| vbd.vm == Some(vm) && vbd.kind == "Disk")
            .filter_map(|vbd| vbd.vdi)
            .filter_map(|vdi| inventory.details.vdis.get(&vdi).and_then(|d| d.sr))
            .collect();
        inventory.vms[&vm].host == host
            && !disks.is_empty()
            && disks.iter().all(|sr| *sr == repository)
    })
}

/// How many disks a repository holds, as another client reads it.
fn disks_on(outside: &Outside, repository: SrId) -> usize {
    outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .details
        .vdis
        .values()
        .filter(|vdi| vdi.sr == Some(repository))
        .count()
}

/// While a migration of the guest runs: Force Shut Down from the VM menu.
/// The review that opens warns of the migration and its confirmation is
/// refused until the consequence is accepted. ONLY the refusal is taken:
/// the acceptance is never ticked and nothing is confirmed (the accepted
/// path wedged this lab twice). What the window showed, or `None` when the
/// migration ended before the review could be opened.
fn forced_shutdown_refused_during_migration(
    client: &mut Client,
    evidence: &mut Evidence,
    vm: VmId,
) -> Option<serde_json::Value> {
    let start = std::time::Instant::now();
    let mut seen = None;
    while start.elapsed() < Duration::from_secs(40) && unfinished(client, vm) > 0 {
        match client.try_menu(Menu::Vm, "vm.force-shutdown") {
            Err(reason) => seen = Some(json!({ "menu_item_disabled": reason })),
            Ok(()) => match client.front().as_deref() {
                Some("vm-recovery") => {
                    let caution = client
                        .node("recovery-migration-caution")
                        .and_then(|node| node.text.or(node.label));
                    let accepted = client
                        .node("recovery-accept-migration")
                        .and_then(|node| node.checked);
                    let refused = client.disabled_reason(Some("vm-recovery"), "recovery-confirm");
                    client.snap(evidence, "forced-shutdown-during-migration-review");
                    client.press("recovery-close");
                    assert_eq!(client.front(), None);
                    if caution.is_some() {
                        return Some(json!({
                            "caution": caution, "acceptance_ticked": accepted,
                            "confirm_refused_with": refused,
                            "seconds_into_the_migration": seconds(start.elapsed()),
                        }));
                    }
                    seen = Some(json!({ "review_without_a_migration_caution": refused }));
                }
                Some("power-confirm") => {
                    // The inventory does not show the migration's task yet.
                    client.press("power-confirm-cancel");
                    assert_eq!(client.front(), None);
                }
                other => panic!("Force Shut Down opened {other:?}"),
            },
        }
        client.pass(Duration::from_millis(500));
    }
    seen
}

/// P3.7: a live migration of the plain guest with its disks to the other
/// server's local repository, and back. While the first move runs, the
/// forced shutdown of the guest is asked for and its refusal recorded; the
/// refusal is not overridden.
#[test]
#[ignore]
fn lab_p3_migration_with_disks() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-7-migration-with-disks");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (coordinator, member) = servers(&mut client);
    let home = local_repository(&mut client, coordinator.0);
    let away = local_repository(&mut client, member.0);
    client.select(&object);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    assert!(placed(&mut client, vm, Some(coordinator.0), home.0));
    assert_eq!(
        client.model(|m| m.inventory.as_ref().expect("inventory").snapshots.len()),
        0,
        "a snapshot exists: a move of disks would be refused"
    );
    wait_guest_ready(&mut client, vm, &mut evidence);
    let mut refusal = None;
    for (tag, host, repository) in [("there", &member, &away), ("back", &coordinator, &home)] {
        let review = migration_to_review(
            &mut client,
            &mut evidence,
            &object,
            vm,
            host.0,
            Some(repository.0),
            tag,
        )
        .expect("the wizard reaches its review");
        assert!(
            review.contains(&format!("({})", repository.0)) && review.contains(&host.1),
            "the review names the server and the repository: {review}"
        );
        assert_eq!(
            client.disabled_reason(Some("migration-dialog"), "migration-apply"),
            None
        );
        let started = std::time::Instant::now();
        let operation = apply_migration(&mut client, &mut evidence, vm, tag);
        if tag == "there" {
            refusal = forced_shutdown_refused_during_migration(&mut client, &mut evidence, vm);
            evidence.step(
                "Force Shut Down during the migration",
                json!(refusal.clone()),
            );
        }
        client.wait("the migration ended", Duration::from_secs(1500), |client| {
            operation_record(client, operation).0.terminal()
        });
        let (state, detail) = operation_record(&mut client, operation);
        assert_eq!(state, OperationState::Succeeded, "{detail}");
        client.wait(
            "the guest and its disks are on the destination",
            Duration::from_secs(120),
            |client| {
                placed(client, vm, Some(host.0), repository.0)
                    && power_state(client, vm) == PowerState::Running
            },
        );
        let took = seconds(started.elapsed());
        assert!(
            placed_outside(&outside, vm, Some(host.0), repository.0),
            "another client reads the guest and its disks on the destination"
        );
        assert!(
            client.says(&host.1),
            "the General tab names the destination server"
        );
        client.snap(&mut evidence, &format!("{tag}-done"));
        evidence.step(
            "migrated with its disks",
            json!({ "to": host.1, "repository": repository.1, "seconds": took, "review": review, "record": detail }),
        );
        evidence.note(&format!("{tag}_seconds"), json!(took));
    }
    evidence.note("guest", json!(name));
    evidence.note("forced_shutdown_during_migration", json!(refusal.clone()));
    let refusal = refusal.expect("the forced shutdown review was seen during the migration");
    assert!(
        refusal["caution"]
            .as_str()
            .is_some_and(|text| text.contains("migration")),
        "{refusal}"
    );
    assert_eq!(refusal["acceptance_ticked"], json!(false), "{refusal}");
    assert!(
        refusal["confirm_refused_with"]
            .as_str()
            .is_some_and(|text| text.contains("accept")),
        "{refusal}"
    );
    client.close();
    evidence.finish();
}

/// P3.7: the same migration with disks, cancelled from the client while it
/// runs, through the review of the cancellation on the General tab. The
/// guest and its disks are verified on the source afterwards, by this
/// client and by another.
#[test]
#[ignore]
fn lab_p3_migration_cancelled() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-7-migration-cancelled");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (coordinator, member) = servers(&mut client);
    let home = local_repository(&mut client, coordinator.0);
    let away = local_repository(&mut client, member.0);
    client.select(&object);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    assert!(placed(&mut client, vm, Some(coordinator.0), home.0));
    wait_guest_ready(&mut client, vm, &mut evidence);
    let away_before = disks_on(&outside, away.0);
    let away_disks: Vec<String> = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .details
        .vdis
        .values()
        .filter(|vdi| vdi.sr == Some(away.0))
        .map(|vdi| vdi.name.clone())
        .collect();
    evidence.step(
        "the destination repository before",
        json!({ "repository": away.1, "disks": away_disks }),
    );
    migration_to_review(
        &mut client,
        &mut evidence,
        &object,
        vm,
        member.0,
        Some(away.0),
        "to-cancel",
    )
    .expect("the wizard reaches its review");
    let started = std::time::Instant::now();
    let operation = apply_migration(&mut client, &mut evidence, vm, "to-cancel");
    // Let the host begin the copy. How often the record of the migration
    // changes while it runs is sampled: the review of a cancellation is
    // bound to the record as it was when the review opened.
    let revision = |client: &mut Client| {
        client.model(|m| {
            m.operation_rows
                .iter()
                .find(|record| record.id == operation)
                .map(|record| record.revision())
        })
    };
    let mut changes = Vec::new();
    let mut last = revision(&mut client);
    let sampling = std::time::Instant::now();
    while sampling.elapsed() < Duration::from_secs(4) {
        client.pass(Duration::from_millis(60));
        let now = revision(&mut client);
        if now != last {
            changes.push((seconds(sampling.elapsed()) * 100.).round() / 100.);
            last = now;
        }
    }
    evidence.step(
        "the record of the running migration changed at these seconds",
        json!(changes),
    );
    evidence.note("record_changes_in_four_seconds", json!(changes.len()));
    let review = format!("recovery-cancel-{operation}");
    let direct = format!("operation-cancel-{operation}");
    // A review that the record's progress overtook refuses its
    // confirmation; it is closed and opened again, as its text says.
    let mut overtaken = Vec::new();
    let how = loop {
        if operation_record(&mut client, operation).0.terminal() {
            break "the migration ended before a cancellation could be confirmed";
        }
        if client.element(&review).is_some() {
            client.press(&review);
            assert_eq!(client.front().as_deref(), Some("vm-recovery"));
            match client.try_press("recovery-confirm") {
                Ok(()) => {
                    if client.front().is_some() {
                        client.press("recovery-close");
                    }
                    break "Review cancellation request… on the General tab, confirmed";
                }
                Err(reason) => {
                    if overtaken.is_empty() {
                        client.snap(&mut evidence, "cancellation-review-overtaken");
                    }
                    overtaken.push(reason);
                    client.press("recovery-close");
                    assert!(
                        overtaken.len() < 40,
                        "the review of the cancellation was refused 40 times: {:?}",
                        overtaken.last()
                    );
                }
            }
        } else if client.element(&direct).is_some() {
            client.press(&direct);
            break "Request cancellation on the General tab";
        } else {
            client.pass(Duration::from_millis(300));
        }
    };
    evidence.step(
        "reviews of the cancellation that the record's progress overtook",
        json!({ "count": overtaken.len(), "reason": overtaken.first() }),
    );
    evidence.note("cancellation_reviews_overtaken", json!(overtaken.len()));
    let asked = seconds(started.elapsed());
    client.snap(&mut evidence, "cancellation-requested");
    client.wait("the migration ended", Duration::from_secs(900), |client| {
        operation_record(client, operation).0.terminal()
    });
    let (state, detail) = operation_record(&mut client, operation);
    let took = seconds(started.elapsed());
    client.nav_view(NavView::Infrastructure);
    client.select(&object);
    client.tab(TabId::General);
    let shown = client
        .node(&format!("vm-operation-{operation}"))
        .map(|node| [node.label, node.text]);
    client.snap(&mut evidence, "cancelled");
    evidence.step(
        "the migration was cancelled from the client",
        json!({ "how": how, "asked_after_seconds": asked, "ended_after_seconds": took,
            "state": format!("{state:?}"), "record": detail, "general_tab": shown }),
    );
    if state == OperationState::Succeeded {
        // The request came too late: the guest moved. Bring it back before
        // failing.
        let review = migration_to_review(
            &mut client,
            &mut evidence,
            &object,
            vm,
            coordinator.0,
            Some(home.0),
            "back",
        );
        if review.is_ok() {
            let back = apply_migration(&mut client, &mut evidence, vm, "back");
            client.wait("the guest is back", Duration::from_secs(1500), |client| {
                operation_record(client, back).0.terminal()
            });
        }
        panic!("the migration finished before it was cancelled: {detail}");
    }
    assert_eq!(state, OperationState::Cancelled, "{detail}");
    client.wait(
        "the guest runs on the source with its disks there",
        Duration::from_secs(120),
        |client| {
            placed(client, vm, Some(coordinator.0), home.0)
                && power_state(client, vm) == PowerState::Running
        },
    );
    assert!(
        placed_outside(&outside, vm, Some(coordinator.0), home.0),
        "another client reads the guest and its disks on the source"
    );
    assert!(outside.settle(Duration::from_secs(10), |inventory| {
        inventory.vms[&vm].power_state == PowerState::Running
    }));
    // What the cancelled copy left on the destination repository.
    let cleared = outside.settle(Duration::from_secs(180), |inventory| {
        inventory
            .details
            .vdis
            .values()
            .filter(|vdi| vdi.sr == Some(away.0))
            .count()
            <= away_before
    });
    let away_after = disks_on(&outside, away.0);
    evidence.step(
        "the destination repository after the cancellation",
        json!({ "repository": away.1, "disks_before": away_before, "disks_after": away_after, "cleared": cleared }),
    );
    evidence.note("state", json!(format!("{state:?}")));
    evidence.note("record", json!(detail));
    evidence.note("cancelled_how", json!(how));
    evidence.note("destination_disks", json!([away_before, away_after]));
    evidence.note("guest", json!(name));
    assert_eq!(
        operation_states(&mut client, vm).len(),
        1,
        "one record, nothing sent twice"
    );
    assert!(cleared, "the cancelled copy left disks on {}", away.1);
    client.close();
    evidence.finish();
}

/// P3.7: the disks of the halted plain guest moved to the other server's
/// local repository and back, through the same wizard; the guest stays
/// halted through both moves and is started again at the end.
#[test]
#[ignore]
fn lab_p3_halted_disk_move() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-7-halted-disk-move");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (coordinator, member) = servers(&mut client);
    let home = local_repository(&mut client, coordinator.0);
    let away = local_repository(&mut client, member.0);
    let affinity = |client: &mut Client| {
        client.model(|m| {
            m.inventory
                .as_ref()
                .expect("inventory")
                .details
                .vms
                .get(&vm)
                .and_then(|details| details.affinity)
                .map(|host| host.to_string())
        })
    };
    client.select(&object);
    client.tab(TabId::General);
    assert_eq!(disk_repositories(&mut client, vm), vec![home.0; 2]);
    assert_eq!(
        client.model(|m| m.inventory.as_ref().expect("inventory").snapshots.len()),
        0,
        "a snapshot exists: a move of disks would be refused"
    );
    let home_before = affinity(&mut client);
    if power_state(&mut client, vm) == PowerState::Running {
        wait_guest_ready(&mut client, vm, &mut evidence);
        let before = newest_operation(&mut client, vm).map(|(id, _)| id);
        client
            .try_toolbar("vm.shutdown")
            .expect("the toolbar offers Shut Down");
        let (shutdown, took) =
            follow_new_operation(&mut client, vm, before, Duration::from_secs(360));
        assert_eq!(
            operation_record(&mut client, shutdown).0,
            OperationState::Succeeded
        );
        client.wait("the guest is halted", Duration::from_secs(120), |client| {
            power_state(client, vm) == PowerState::Halted
        });
        evidence.step("the guest was shut down", json!({ "seconds": took }));
    }
    for (tag, host, repository) in [("there", &member, &away), ("back", &coordinator, &home)] {
        let review = migration_to_review(
            &mut client,
            &mut evidence,
            &object,
            vm,
            host.0,
            Some(repository.0),
            tag,
        )
        .expect("the wizard reaches its review");
        assert!(
            review.contains("Remain halted") && review.contains(&format!("({})", repository.0)),
            "the review says the guest stays halted and names the repository: {review}"
        );
        let started = std::time::Instant::now();
        let operation = apply_migration(&mut client, &mut evidence, vm, tag);
        client.wait("the move ended", Duration::from_secs(1500), |client| {
            operation_record(client, operation).0.terminal()
        });
        let (state, detail) = operation_record(&mut client, operation);
        assert_eq!(state, OperationState::Succeeded, "{detail}");
        client.wait(
            "the disks are on the destination repository",
            Duration::from_secs(120),
            |client| {
                disk_repositories(client, vm) == vec![repository.0; 2]
                    && power_state(client, vm) == PowerState::Halted
            },
        );
        let took = seconds(started.elapsed());
        assert!(
            placed_outside(&outside, vm, None, repository.0),
            "another client reads the halted guest's disks on the destination"
        );
        client.snap(&mut evidence, &format!("{tag}-done"));
        evidence.step(
            "the halted guest's disks were moved",
            json!({ "to": repository.1, "home_server": affinity(&mut client), "seconds": took, "review": review, "record": detail }),
        );
        evidence.note(&format!("{tag}_seconds"), json!(took));
    }
    // Start it where it belongs.
    client.select(&object);
    client.tab(TabId::General);
    let before = newest_operation(&mut client, vm).map(|(id, _)| id);
    client
        .try_toolbar("vm.start")
        .expect("the toolbar offers Start for the halted guest");
    let (start, took) = follow_new_operation(&mut client, vm, before, Duration::from_secs(360));
    let (state, detail) = operation_record(&mut client, start);
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    client.wait(
        "the guest runs on the coordinator",
        Duration::from_secs(120),
        |client| {
            power_state(client, vm) == PowerState::Running
                && resident_host(client, vm) == Some(coordinator.0)
        },
    );
    evidence.step("the guest was started again", json!({ "seconds": took }));
    evidence.note("home_server_before", json!(home_before));
    evidence.note("home_server_after", json!(affinity(&mut client)));
    evidence.note("guest", json!(name));
    client.close();
    evidence.finish();
}

/// The share of the pixels of a picture that are clearly of one of the
/// console's colours: magenta (`\033[45m`) or green (`\033[42m`).
fn colour_share(picture: &image::RgbaImage, colour: &str) -> f64 {
    let matches = |pixel: &image::Rgba<u8>| match colour {
        "magenta" => pixel[0] > 120 && pixel[1] < 60 && pixel[2] > 120,
        "green" => pixel[0] < 60 && pixel[1] > 120 && pixel[2] < 60,
        _ => pixel[0] > 120 && pixel[1] < 60 && pixel[2] < 60,
    };
    picture.pixels().filter(|pixel| matches(pixel)).count() as f64
        / (picture.width() * picture.height()).max(1) as f64
}

/// The Console tab of the selected guest with a picture and the keyboard
/// on the canvas, and a root shell: the lab's plain guest lets root in
/// without a password.
fn console_root_shell(client: &mut Client, evidence: &mut Evidence, tag: &str) {
    client.tab(TabId::Console);
    if console_frame(client).is_none() && client.element(console::view::CANVAS).is_none() {
        let _ = client.try_local("vm.console");
    }
    wait_console_picture(client, "the guest's console shows a picture");
    client.press(console::view::CANVAS);
    client.key("enter");
    client.pass(Duration::from_secs(5));
    client.key("enter");
    client.pass(Duration::from_secs(1));
    client.type_keys("root");
    client.key("enter");
    client.pass(Duration::from_secs(3));
    client.snap(evidence, &format!("{tag}-logged-in"));
}

/// Types a command at the console of the tab and waits until the canvas
/// shows the colour it paints.
fn console_command_paints(
    client: &mut Client,
    command: &str,
    colour: &str,
    limit: Duration,
) -> Option<f64> {
    client.press(console::view::CANVAS);
    client.type_keys(command);
    client.key("enter");
    client.wait_for(limit, &mut |client| {
        client
            .pixels_of(console::view::CANVAS)
            .is_some_and(|now| colour_share(&now, colour) > 0.3)
    })?;
    client
        .pixels_of(console::view::CANVAS)
        .map(|now| colour_share(&now, colour))
}

/// P3.4: a snapshot with memory. A file is made in the guest's memory
/// (`/dev/shm`) from a root shell at the console; the snapshot is taken
/// with "Include guest memory"; the file is removed; Revert through its
/// review, with the shutdown and "Resume restored memory" chosen. The guest
/// resumes: the shell at the console is the one that was logged in, and
/// the file is there again. The snapshot is deleted with its disks.
#[test]
#[ignore]
fn lab_p3_snapshot_with_memory() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-4-snapshot-with-memory");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (object, vm, guest_name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    wait_guest_ready(&mut client, vm, &mut evidence);
    let before = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .snapshots
        .for_vm(vm)
        .count();
    let started = started_at(&mut client, vm);

    // A root shell at the console, and a file that exists in memory only.
    console_root_shell(&mut client, &mut evidence, "before");
    client.type_keys("rm -f /dev/shm/gm; cp /etc/hostname /dev/shm/gm");
    client.key("enter");
    client.pass(Duration::from_secs(2));
    let probe = "if [ -e /dev/shm/gm ]; then printf '\\033[45m'; clear; fi";
    let reset = "printf '\\033[0m'; clear";
    let share = console_command_paints(&mut client, probe, "magenta", Duration::from_secs(30))
        .expect("the file is there before the snapshot: the screen turns magenta");
    client.snap(&mut evidence, "marker-present");
    evidence.step(
        "a file was made in the guest's memory",
        json!({ "magenta_share": share }),
    );
    client.type_keys(reset);
    client.key("enter");
    client.pass(Duration::from_secs(2));

    // Take a Snapshot with the guest's memory.
    let name = format!("gpui-memory-{}", stamp());
    client.menu(Menu::Vm, "vm.snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-create"));
    client.type_into("snapshot-name", &name);
    set_checkbox(&mut client, "snapshot-memory", true);
    client.snap(&mut evidence, "snapshot-create-with-memory");
    client.press("confirm-snapshot");
    client.tab(TabId::Snapshots);
    let took = client.wait(
        "the snapshot with memory is in the Snapshots tab",
        Duration::from_secs(900),
        |client| {
            client.front().is_none()
                && table_row(client, "snapshots-table", &name).is_some()
                && unfinished(client, vm) == 0
        },
    );
    let checkpoint = client.model(|m| {
        m.inventory
            .as_ref()
            .expect("inventory")
            .snapshots
            .for_vm(vm)
            .find(|s| s.name == name)
            .map(|s| format!("{:?}", s.power_state))
    });
    assert_eq!(
        checkpoint.as_deref(),
        Some("Suspended"),
        "the snapshot holds memory"
    );
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    let row = table_rows(&mut client, "snapshots-table")
        .into_iter()
        .find(|row| row.1.iter().any(|cell| cell.contains(&name)));
    client.snap(&mut evidence, "snapshot-with-memory-taken");
    evidence.step(
        "the snapshot with memory was taken",
        json!({ "seconds": seconds(took), "name": name, "row": row.map(|row| row.1), "operations": operation_states(&mut client, vm) }),
    );
    evidence.note("take_seconds", json!(seconds(took)));

    // After the snapshot the file is removed: the guest now differs from it.
    client.tab(TabId::Console);
    wait_console_picture(&mut client, "the console after the snapshot");
    client.press(console::view::CANVAS);
    client.type_keys("rm /dev/shm/gm");
    client.key("enter");
    client.pass(Duration::from_secs(2));
    client.type_keys(probe);
    client.key("enter");
    client.pass(Duration::from_secs(4));
    let share = client
        .pixels_of(console::view::CANVAS)
        .map(|now| colour_share(&now, "magenta"))
        .unwrap_or(1.);
    assert!(
        share < 0.05,
        "the file is gone and the probe paints nothing: {share}"
    );
    client.snap(&mut evidence, "marker-removed");

    // Revert, through its review: shut down first, resume the memory.
    client.tab(TabId::Snapshots);
    let row = table_row(&mut client, "snapshots-table", &name).expect("the row");
    client.press(&row);
    client
        .try_local("snapshot.revert")
        .expect("Revert for the selected snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-restore"));
    assert!(
        client.describes("Resume restored memory after restore"),
        "the review offers to resume the memory"
    );
    set_checkbox(&mut client, "restore-shutdown", true);
    set_checkbox(&mut client, "restore-start", true);
    client.snap(&mut evidence, "revert-review");
    client.press("confirm-snapshot");
    client.wait("the revert started", Duration::from_secs(30), |client| {
        client.front().is_none() && unfinished(client, vm) > 0
    });
    let took = client.wait(
        "the revert finished and the guest runs again",
        Duration::from_secs(900),
        |client| unfinished(client, vm) == 0 && power_state(client, vm) == PowerState::Running,
    );
    let states = operation_states(&mut client, vm);
    client.snap(&mut evidence, "reverted");
    evidence.step(
        "the revert finished",
        json!({ "seconds": seconds(took), "operations": states,
            "start_time_before": started, "start_time_after": started_at(&mut client, vm) }),
    );
    assert!(
        states.iter().all(|state| state.ends_with("Succeeded")),
        "an operation did not succeed: {states:?}"
    );
    evidence.note("revert_seconds", json!(seconds(took)));

    // The guest resumed: no login is asked, and the file is in memory
    // again. Typed straight away, without a login: at a login prompt the
    // probe would paint nothing.
    client.tab(TabId::Console);
    if client
        .wait_for(Duration::from_secs(60), &mut |client| {
            console_frame(client).is_some() && client.element(console::view::CANVAS).is_some()
        })
        .is_none()
    {
        let reconnect = client
            .try_local("console.reconnect")
            .or_else(|_| client.try_local("vm.console"));
        evidence.step(
            "the console was opened again",
            json!(format!("{reconnect:?}")),
        );
    }
    wait_console_picture(&mut client, "the console after the revert");
    client.pass(Duration::from_secs(3));
    let share = console_command_paints(&mut client, probe, "magenta", Duration::from_secs(60));
    client.snap(&mut evidence, "marker-back-after-resume");
    evidence.step(
        "the resumed guest has the memory of the snapshot",
        json!({ "magenta_share": share }),
    );
    evidence.note("resumed_with_memory_state", json!(share.is_some()));
    if share.is_some() {
        client.type_keys("printf '\\033[0m'; clear; rm /dev/shm/gm; exit");
        client.key("enter");
        client.pass(Duration::from_secs(3));
    }
    client.snap(&mut evidence, "console-left");

    // Delete the snapshot, with its disks.
    client.tab(TabId::Snapshots);
    let row = table_row(&mut client, "snapshots-table", &name).expect("the row after the revert");
    client.press(&row);
    client
        .try_local("snapshot.delete")
        .expect("Delete for the selected snapshot");
    assert_eq!(client.front().as_deref(), Some("snapshot-delete"));
    if client.node("snapshot-delete-disks").is_some() {
        set_checkbox(&mut client, "snapshot-delete-disks", true);
    }
    client.snap(&mut evidence, "delete-review");
    client.press("confirm-snapshot");
    let took = client.wait(
        "the snapshot is gone from the Snapshots tab",
        Duration::from_secs(600),
        |client| {
            client.front().is_none()
                && table_row(client, "snapshots-table", &name).is_none()
                && unfinished(client, vm) == 0
        },
    );
    client.snap(&mut evidence, "deleted");
    evidence.step(
        "the snapshot was deleted",
        json!({ "seconds": seconds(took), "operations": operation_states(&mut client, vm) }),
    );
    assert!(
        outside.settle(Duration::from_secs(60), |inventory| {
            inventory.snapshots.for_vm(vm).count() == before
                && !inventory
                    .snapshots
                    .iter()
                    .any(|snapshot| snapshot.name == name)
        }),
        "the host still has the snapshot"
    );
    evidence.note("delete_seconds", json!(seconds(took)));
    evidence.note("guest", json!(guest_name));
    assert!(
        share.is_some(),
        "after the revert the probe did not paint: the guest did not resume with the snapshot's memory"
    );
    client.close();
    evidence.finish();
}

/// A text as the octal escapes of `printf`: typed with keys that need no
/// Shift.
fn octal(text: &str) -> String {
    text.bytes().map(|byte| format!("\\{byte:03o}")).collect()
}

/// The share of a colour in what a console window draws on its canvas.
fn window_colour(
    client: &mut Client,
    handle: xenadmin_desktop::kit::AnyWindowHandle,
    colour: &str,
) -> f64 {
    client
        .pixels_in(handle, console::view::CANVAS)
        .map(|now| colour_share(&now, colour))
        .unwrap_or(0.)
}

/// P3.3, what the first console scenario left out. Type Clipboard as Keys
/// with a text of capitals and punctuation, checked by a comparison the
/// guest's shell makes; Open in Separate Window (frames in the second
/// window, keys typed there reach the guest), Fullscreen of that window
/// and of the main one (the headless platform's window state only), the
/// window closed and the console opened in the tab again; Send
/// Ctrl+Alt+Del, after which the guest reboots in place.
#[test]
#[ignore]
fn lab_p3_console_window_and_typing() {
    const WANTED: &str = "Hi, World! (A+B)*C? <D> {E} ~F #G @H %I ^J &K _L |M :N";
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-3-console-window-and-typing");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let main = client.window;
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    client.select(&object);
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    wait_guest_ready(&mut client, vm, &mut evidence);
    console_root_shell(&mut client, &mut evidence, "tab");

    // The text the comparison expects is given to the shell by keys that
    // need no Shift (octal escapes), through the canvas: a path the first
    // console scenario showed.
    client.type_keys(&format!("x=`printf '{}'`", octal(WANTED)));
    client.key("enter");
    client.pass(Duration::from_secs(2));

    // Type Clipboard as Keys: the clipboard holds a command with capitals
    // and shifted punctuation and a line break. The screen turns green
    // only if the shell read exactly the text.
    let typed = format!("[ \"$x\" = \"{WANTED}\" ] && printf '\\033[42m' && clear\n");
    client.copy_to_clipboard(&typed);
    client
        .try_local("console.type-clipboard")
        .expect("Type Clipboard as Keys on the Console tab's row");
    client.wait(
        "the review of the typing is shown",
        Duration::from_secs(15),
        |client| client.node(console::view::TYPING_PROMPT).is_some(),
    );
    let prompt = client
        .node(console::view::TYPING_PROMPT)
        .and_then(|node| node.text.or(node.label))
        .unwrap_or_default();
    client.snap(&mut evidence, "typing-review");
    assert!(
        !prompt.contains("World") && !client.says("World"),
        "the review shows the text: {prompt}"
    );
    assert!(
        prompt.contains("1 line break sent as Enter") && prompt.contains("US keyboard"),
        "{prompt}"
    );
    client.press(console::view::CONFIRM_TYPING);
    let took = client.wait_for(Duration::from_secs(120), &mut |client| {
        client
            .pixels_of(console::view::CANVAS)
            .is_some_and(|now| colour_share(&now, "green") > 0.3)
    });
    let green = client
        .pixels_of(console::view::CANVAS)
        .map(|now| colour_share(&now, "green"));
    client.snap(&mut evidence, "typed-clipboard-effect");
    evidence.step(
        "Type Clipboard as Keys",
        json!({ "characters": typed.chars().count(), "prompt": prompt,
            "screen_green_after_seconds": took.map(seconds), "green_share": green }),
    );
    evidence.note(
        "typed_clipboard_matched_in_the_guest",
        json!(took.is_some()),
    );
    assert!(
        took.is_some(),
        "the shell did not read the typed text as written: the screen did not turn green"
    );
    client.press(console::view::CANVAS);
    client.type_keys("printf '\\033[0m'; clear");
    client.key("enter");
    client.pass(Duration::from_secs(2));

    // Open in Separate Window.
    assert!(client.other_windows().is_empty());
    client
        .try_local("console.undock")
        .expect("Open in Separate Window on the Console tab's row");
    client.wait(
        "a second window opened",
        Duration::from_secs(15),
        |client| client.other_windows().len() == 1,
    );
    let handle = client.other_windows()[0];
    let entry = client.model(|m| {
        console::windows::open_windows(m)
            .first()
            .map(|entry| (entry.id, entry.target.window_title()))
    });
    let (id, title) = entry.expect("the model has the console in a window");
    let seat = Seat::Window(id);
    let took = client.wait(
        "the second window draws the guest's picture",
        Duration::from_secs(60),
        |client| {
            client.model(|m| console::canvas::frame(m, seat).is_some())
                && client
                    .canvas(handle, seat)
                    .is_some_and(|(uploaded, _, _)| uploaded > 0)
        },
    );
    let tab_says = client
        .node(console::view::CLOSED)
        .and_then(|node| node.text.or(node.label));
    client.snap_window(&mut evidence, handle, "window-first-frame");
    client.snap(&mut evidence, "tab-while-in-a-window");
    evidence.step(
        "the console moved to a window of its own",
        json!({ "title": title, "picture_after_seconds": seconds(took), "tab_says": tab_says,
            "tab_has_a_console": client.model(|m| console::canvas::frame(m, Seat::Tab).is_some()) }),
    );
    assert!(title.contains(&name), "{title}");

    // Keys typed in the second window reach the guest.
    client
        .try_press_in(handle, console::view::CANVAS)
        .expect("the canvas of the window");
    let (uploaded_before, keyboard, sent) = client.canvas(handle, seat).expect("the canvas");
    assert!(
        keyboard,
        "the window's canvas has the keyboard after a press"
    );
    client.type_keys_in(handle, "printf '\\033[45m'; clear");
    client.key_in(handle, "enter");
    let took = client.wait_for(Duration::from_secs(40), &mut |client| {
        window_colour(client, handle, "magenta") > 0.3
    });
    let (uploaded, _, forwarded) = client.canvas(handle, seat).expect("the canvas");
    client.snap_window(&mut evidence, handle, "window-command-typed");
    evidence.step(
        "a command typed in the second window ran in the guest",
        json!({ "seconds": took.map(seconds), "magenta_share": window_colour(&mut client, handle, "magenta"),
            "keys_forwarded": forwarded - sent, "pictures_uploaded": uploaded - uploaded_before }),
    );
    assert!(took.is_some(), "the window's screen did not turn magenta");
    assert!(forwarded > sent && uploaded > uploaded_before);

    // Fullscreen of the console window, and back.
    let before = client.is_fullscreen(handle);
    client
        .try_press_in(handle, "console-fullscreen")
        .expect("Fullscreen in the console window");
    client.pass(Duration::from_millis(500));
    let during = client.is_fullscreen(handle);
    client.snap_window(&mut evidence, handle, "window-fullscreen");
    client
        .try_press_in(handle, "console-fullscreen")
        .expect("Fullscreen in the console window, again");
    client.pass(Duration::from_millis(500));
    let after = client.is_fullscreen(handle);
    evidence.step(
        "Fullscreen in the console window",
        json!({ "before": before, "after_the_first_press": during, "after_the_second": after }),
    );
    evidence.note("window_fullscreen", json!([before, during, after]));

    // The guest's colours back, typed in the window; then the window is
    // closed with its Close console button.
    client
        .try_press_in(handle, console::view::CANVAS)
        .expect("the canvas of the window");
    client.type_keys_in(handle, "printf '\\033[0m'; clear");
    client.key_in(handle, "enter");
    client.pass(Duration::from_secs(2));
    client
        .try_press_in(handle, "close-console")
        .expect("Close console in the window");
    client.wait(
        "the second window closed",
        Duration::from_secs(15),
        |client| {
            client.other_windows().is_empty()
                && client.model(|m| console::windows::open_windows(m).is_empty())
        },
    );
    let tab_says = client
        .node(console::view::CLOSED)
        .and_then(|node| node.text.or(node.label));
    client.snap(&mut evidence, "tab-after-the-window-closed");

    // The console is the tab's to open again.
    let reopened = client.try_local("vm.console");
    let took = wait_console_picture(&mut client, "the tab shows the console again");
    client.press(console::view::CANVAS);
    client.type_keys("printf '\\033[42m'; clear");
    client.key("enter");
    let painted = client.wait_for(Duration::from_secs(40), &mut |client| {
        client
            .pixels_of(console::view::CANVAS)
            .is_some_and(|now| colour_share(&now, "green") > 0.3)
    });
    client.snap(&mut evidence, "tab-console-again");
    evidence.step(
        "the window closed and the tab opened the console again",
        json!({ "tab_said": tab_says, "open_console": format!("{reopened:?}"),
            "picture_after_seconds": took, "typed_in_the_tab_after_seconds": painted.map(seconds) }),
    );
    assert!(
        painted.is_some(),
        "keys typed in the tab did not reach the guest after the window closed"
    );
    client.type_keys("printf '\\033[0m'; clear");
    client.key("enter");
    client.pass(Duration::from_secs(2));

    // Fullscreen from the tab's row is the main window's.
    let before = client.is_fullscreen(main);
    client
        .try_local("console.fullscreen")
        .expect("Fullscreen on the Console tab's row");
    client.pass(Duration::from_millis(500));
    let during = client.is_fullscreen(main);
    client.snap(&mut evidence, "main-window-fullscreen");
    client
        .try_local("console.fullscreen")
        .expect("Fullscreen on the Console tab's row, again");
    client.pass(Duration::from_millis(500));
    let after = client.is_fullscreen(main);
    evidence.step(
        "Fullscreen from the Console tab",
        json!({ "before": before, "after_the_first_press": during, "after_the_second": after }),
    );
    evidence.note("main_window_fullscreen", json!([before, during, after]));

    // Send Ctrl+Alt+Del: the guest reboots in place.
    let start_before = started_at(&mut client, vm);
    let records_before = operation_states(&mut client, vm).len();
    client
        .try_local("vm.send-ctrl-alt-del")
        .expect("Send Ctrl+Alt+Del on the Console tab's row");
    let rebooted = client.wait_for(Duration::from_secs(300), &mut |client| {
        let now = started_at(client, vm);
        now.is_some() && now != start_before
    });
    client.snap(&mut evidence, "after-ctrl-alt-del");
    let said: Vec<String> = [
        console::view::STATUS,
        console::view::RECOVERY_STATUS,
        console::view::ERROR,
        console::view::MESSAGE,
    ]
    .into_iter()
    .filter_map(|key| client.node(key).and_then(|node| node.text))
    .collect();
    evidence.step(
        "Send Ctrl+Alt+Del",
        json!({ "start_time_before": start_before, "start_time_after": started_at(&mut client, vm),
            "restarted_after_seconds": rebooted.map(seconds), "console_says": said,
            "power_records_added": operation_states(&mut client, vm).len() - records_before }),
    );
    assert!(
        rebooted.is_some(),
        "the guest's start time did not change after Ctrl+Alt+Del"
    );
    assert_eq!(power_state(&mut client, vm), PowerState::Running);
    assert_eq!(
        resident_host(&mut client, vm),
        client.model(|m| m.inventory.as_ref().expect("inventory").coordinator),
        "the guest rebooted in place"
    );
    // The console after the reboot: whether the tab shows the new boot
    // without being asked is recorded.
    let picture = client.wait_for(Duration::from_secs(120), &mut |client| {
        let generation = console_frame(client).map(|frame| frame.0);
        client.pass(Duration::from_secs(2));
        console_frame(client).map(|frame| frame.0) != generation && generation.is_some()
    });
    client.snap(&mut evidence, "console-after-the-reboot");
    evidence.note(
        "console_follows_the_reboot_after_seconds",
        json!(picture.map(seconds)),
    );
    wait_guest_ready(&mut client, vm, &mut evidence);
    evidence.note("guest", json!(name));
    evidence.note(
        "rebooted_in_place_after_seconds",
        json!(rebooted.map(seconds)),
    );
    client.close();
    evidence.finish();
}

/// The baseline of the lab has the plain guest running on the coordinator
/// with its disks on the coordinator's local repository. When a run that
/// failed left it on the member, this moves it back with the application's
/// own wizard (a live migration with its disks, or a move of the halted
/// guest's disks and a start); otherwise it only records where it is.
#[test]
#[ignore]
fn lab_baseline_plain_guest_on_the_coordinator() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "baseline-plain-guest-on-the-coordinator");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (coordinator, _) = servers(&mut client);
    let home = local_repository(&mut client, coordinator.0);
    client.select(&object);
    client.tab(TabId::General);
    let running = power_state(&mut client, vm) == PowerState::Running;
    let at_home = disk_repositories(&mut client, vm)
        .iter()
        .all(|sr| *sr == home.0)
        && (!running || resident_host(&mut client, vm) == Some(coordinator.0));
    evidence.note("moved", json!(!at_home));
    if !at_home {
        if running {
            wait_guest_ready(&mut client, vm, &mut evidence);
        }
        let review = migration_to_review(
            &mut client,
            &mut evidence,
            &object,
            vm,
            coordinator.0,
            Some(home.0),
            "home",
        )
        .expect("the wizard reaches its review");
        let operation = apply_migration(&mut client, &mut evidence, vm, "home");
        client.wait("the move ended", Duration::from_secs(1500), |client| {
            operation_record(client, operation).0.terminal()
        });
        let (state, detail) = operation_record(&mut client, operation);
        assert_eq!(state, OperationState::Succeeded, "{detail}");
        evidence.step("moved home", json!({ "review": review, "record": detail }));
    }
    if power_state(&mut client, vm) == PowerState::Halted {
        let before = newest_operation(&mut client, vm).map(|(id, _)| id);
        client
            .try_toolbar("vm.start")
            .expect("the toolbar offers Start for the halted guest");
        let (start, _) = follow_new_operation(&mut client, vm, before, Duration::from_secs(360));
        let (state, detail) = operation_record(&mut client, start);
        assert_eq!(state, OperationState::Succeeded, "{detail}");
        evidence.note("started", json!(true));
    }
    client.wait(
        "the guest runs on the coordinator with its disks there",
        Duration::from_secs(180),
        |client| {
            placed(client, vm, Some(coordinator.0), home.0)
                && power_state(client, vm) == PowerState::Running
        },
    );
    evidence.note("guest", json!(name));
    client.close();
    evidence.finish();
}

/// What the client holds of a server and what its window shows of it: the
/// state word of its row of the tree, the status bar, and of the inventory
/// whether it is enabled, marked in maintenance and live, and when it and
/// its toolstack started.
fn host_shown(client: &mut Client, object: &ObjectRef, host: HostId) -> serde_json::Value {
    let row = client.tree_row(object).map(|row| row.status);
    let held = client.model(|m| {
        let inventory = m.inventory.as_ref()?;
        let details = inventory.details.hosts.get(&host)?;
        let live = details
            .metrics
            .and_then(|id| inventory.details.host_metrics.get(&id))
            .and_then(|metrics| metrics.live);
        Some(json!({
            "enabled": inventory.hosts.get(&host).map(|host| host.enabled),
            "maintenance": details.maintenance_mode,
            "live": live,
            "boot_time": details.boot_time,
            "toolstack_start": details.agent_start_time,
        }))
    });
    json!({ "tree_row": row, "status_bar": connection_status(client), "held": held })
}

/// Waits until `done` holds, recording every change of what the window
/// shows of the server on the way. The seconds it took, `None` when the
/// limit passed.
fn watch_host(
    client: &mut Client,
    evidence: &mut Evidence,
    what: &str,
    object: &ObjectRef,
    host: HostId,
    limit: Duration,
    mut done: impl FnMut(&mut Client) -> bool,
) -> Option<f64> {
    let start = std::time::Instant::now();
    let mut last = serde_json::Value::Null;
    let mut changes = Vec::new();
    let took = client.wait_for(limit, &mut |client| {
        let shown = host_shown(client, object, host);
        if shown != last {
            if changes.len() < 80 {
                changes.push(
                    json!({ "at_seconds": seconds(start.elapsed()), "shown": shown.clone() }),
                );
            }
            last = shown;
        }
        done(client)
    });
    evidence.step(
        what,
        json!({ "seconds": took.map(seconds), "changes": changes }),
    );
    took.map(seconds)
}

fn host_detail<R>(
    client: &mut Client,
    host: HostId,
    read: impl FnOnce(&xenadmin_core::HostDetails, Option<bool>) -> R,
) -> Option<R> {
    client.model(|m| {
        let inventory = m.inventory.as_ref()?;
        let details = inventory.details.hosts.get(&host)?;
        let live = details
            .metrics
            .and_then(|id| inventory.details.host_metrics.get(&id))
            .and_then(|metrics| metrics.live);
        Some(read(details, live))
    })
}

fn host_enabled(client: &mut Client, host: HostId) -> bool {
    client.model(|m| {
        m.inventory
            .as_ref()
            .and_then(|inventory| inventory.hosts.get(&host))
            .is_some_and(|host| host.enabled)
    })
}

fn resource_ids(client: &mut Client) -> Vec<OperationId> {
    client.model(|m| m.resource_rows.iter().map(|record| record.id).collect())
}

/// A reviewed change of the selected server from the Server menu: the
/// review, its confirmation, and its record followed until it ends. The
/// record's state, what it says and its steps, the review's lines and the
/// seconds it took.
#[allow(clippy::too_many_arguments)]
fn host_change(
    client: &mut Client,
    evidence: &mut Evidence,
    object: &ObjectRef,
    host: HostId,
    command: &str,
    dialog: &str,
    tag: &str,
    limit: Duration,
) -> (OperationState, String, Vec<String>, String) {
    let before = resource_ids(client);
    client.menu(Menu::Server, command);
    assert_eq!(client.front().as_deref(), Some(dialog));
    let review = review_lines(client, "host-review");
    client.snap(evidence, &format!("{tag}-review"));
    client.press("host-confirm");
    client.wait(
        "the operation is recorded",
        Duration::from_secs(60),
        |client| {
            client.front().is_none() && resource_ids(client).iter().any(|id| !before.contains(id))
        },
    );
    let operation = resource_ids(client)
        .into_iter()
        .find(|id| !before.contains(id))
        .expect("the record");
    client.snap(evidence, &format!("{tag}-in-progress"));
    let took = watch_host(
        client,
        evidence,
        &format!("{tag}: the record until it ended"),
        object,
        host,
        limit,
        |client| resource_record(client, operation).0.terminal(),
    );
    let (state, detail, steps) = resource_record(client, operation);
    client.snap(evidence, &format!("{tag}-ended"));
    evidence.step(
        tag,
        json!({ "command": command, "review": review, "state": format!("{state:?}"),
            "record": detail, "steps": steps, "seconds": took }),
    );
    (state, detail, steps, review)
}

/// The guests of a server, by name.
fn residents(client: &mut Client, host: HostId) -> Vec<String> {
    client.model(|m| {
        m.inventory
            .as_ref()
            .expect("inventory")
            .guests()
            .filter(|vm| vm.host == Some(host) && !vm.is_template)
            .map(|vm| vm.name.clone())
            .collect()
    })
}

/// The member of the lab as a server of the window, after the checks every
/// host scenario makes first: it is not the coordinator, it is the second
/// host of the manifest, it is enabled and no guest runs on it.
fn empty_member(client: &mut Client, lab: &Lab) -> (ObjectRef, HostId, String) {
    let (coordinator, member) = servers(client);
    assert_ne!(coordinator.0, member.0);
    assert_eq!(
        member.1, lab.hosts[1].name,
        "the member is the second host of the manifest"
    );
    assert_ne!(member.1, lab.hosts[0].name);
    let guests = residents(client, member.0);
    assert!(
        guests.is_empty(),
        "guests run on the member ({guests:?}): move them to the coordinator first (core scenario lab_move_xo_appliance, lab_baseline_tools_guest_on_the_coordinator)"
    );
    let object = client
        .model(|m| ObjectRef::host(m.inventory.as_ref().expect("inventory").pool_id, member.0));
    (object, member.0, member.1)
}

/// P3.6: Restart Toolstack on the member from the Server menu. First with
/// the member's text console open in the Console tab: the host counts an
/// open console as a pending task, and the restart is refused with that
/// reason. Then with the console closed: the review, its confirmation, the
/// record until it succeeds, and the member enabled again with a newer
/// toolstack start.
#[test]
#[ignore]
fn lab_p3_member_toolstack_restart() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-6-member-toolstack-restart");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let window = client.window;
    let (host, member, member_name) = empty_member(&mut client, &lab);
    client.select(&host);
    client.tab(TabId::General);
    assert!(host_enabled(&mut client, member));
    let start_before = host_detail(&mut client, member, |details, _| {
        details.agent_start_time.clone()
    })
    .flatten();

    // With a console open.
    client.tab(TabId::Console);
    let picture = client.wait_for(Duration::from_secs(90), &mut |client| {
        console_frame(client).is_some()
            && client
                .canvas(window, Seat::Tab)
                .is_some_and(|(uploaded, _, _)| uploaded > 0)
    });
    assert!(picture.is_some(), "the member's text console did not open");
    client.snap(&mut evidence, "console-open");
    // The host's list of tasks reaches the client a moment later.
    client.pass(Duration::from_secs(5));
    let refused = match client.try_menu(Menu::Server, "server.restart-toolstack") {
        Err(reason) => json!({ "how": "the menu item is disabled", "reason": reason }),
        Ok(()) => {
            assert_eq!(client.front().as_deref(), Some("host-restart-toolstack"));
            client.snap(&mut evidence, "restart-with-a-console-open");
            let error = client
                .node("host-error")
                .and_then(|node| node.text.or(node.label));
            match client.try_press("host-confirm") {
                Err(reason) => {
                    client.press("host-close");
                    json!({ "how": "the review cannot be confirmed", "reason": reason, "says": error })
                }
                Ok(()) => {
                    let before = resource_ids(&mut client);
                    client.wait("the request ended", Duration::from_secs(300), |client| {
                        client
                            .model(|m| m.resource_rows.iter().all(|record| record.state.terminal()))
                    });
                    let said = client.model(|m| m.error.clone());
                    let record = before
                        .last()
                        .map(|id| resource_record(&mut client, *id))
                        .map(|(state, detail, _)| json!([format!("{state:?}"), detail]));
                    if client.front().is_some() {
                        client.snap(&mut evidence, "restart-refused-by-the-host");
                        client.press("host-close");
                    }
                    json!({ "how": "the request was made", "says": said.or(error), "record": record })
                }
            }
        }
    };
    assert_eq!(client.front(), None);
    evidence.step(
        "Restart Toolstack with a console open",
        json!(refused.clone()),
    );
    evidence.note("refused_with_a_console_open", refused.clone());
    let unchanged = host_detail(&mut client, member, |details, _| {
        details.agent_start_time.clone()
    })
    .flatten();
    assert_eq!(
        unchanged, start_before,
        "the toolstack was not restarted while the console was open: {refused}"
    );
    let reason = refused["reason"]
        .as_str()
        .or(refused["says"].as_str())
        .unwrap_or_default()
        .to_owned();
    assert!(
        reason.contains("console") || reason.contains("pending"),
        "the refusal names the pending task: {refused}"
    );

    // The console closed: the restart goes through.
    client
        .try_local("console.close")
        .expect("Close Console on the Console tab's row");
    client.tab(TabId::General);
    let offered = client.wait_for(Duration::from_secs(120), &mut |client| {
        client.pass(Duration::from_secs(2));
        match client.try_menu(Menu::Server, "server.restart-toolstack") {
            Ok(()) => {
                client.press("host-close");
                true
            }
            Err(_) => false,
        }
    });
    evidence.step(
        "Restart Toolstack is offered after the console closed",
        json!({ "seconds": offered.map(seconds) }),
    );
    assert!(
        offered.is_some(),
        "Restart Toolstack stayed refused after the console closed"
    );
    let (state, detail, steps, review) = host_change(
        &mut client,
        &mut evidence,
        &host,
        member,
        "server.restart-toolstack",
        "host-restart-toolstack",
        "restart-toolstack",
        Duration::from_secs(1200),
    );
    assert_eq!(state, OperationState::Succeeded, "{detail} {steps:?}");
    assert!(review.contains("Restart XAPI"), "{review}");
    let back = watch_host(
        &mut client,
        &mut evidence,
        "the member after the restart, until it is enabled",
        &host,
        member,
        Duration::from_secs(900),
        |client| host_enabled(client, member),
    );
    assert!(back.is_some(), "the member did not come back enabled");
    let start_after = host_detail(&mut client, member, |details, _| {
        details.agent_start_time.clone()
    })
    .flatten();
    assert_ne!(start_after, start_before, "the toolstack has a newer start");
    assert!(
        outside.settle(Duration::from_secs(120), |inventory| {
            inventory.hosts[&member].enabled
        }),
        "another client reads the member enabled"
    );
    client.snap(&mut evidence, "after-the-restart");
    evidence.note("server", json!(member_name));
    evidence.note("toolstack_start", json!([start_before, start_after]));
    evidence.note("record", json!(detail));
    client.close();
    evidence.finish();
}

/// Enter Maintenance Mode for the selected, empty member.
fn enter_maintenance(
    client: &mut Client,
    evidence: &mut Evidence,
    host: &ObjectRef,
    member: HostId,
) {
    assert_eq!(
        client
            .menu_label(Menu::Server, "server.maintenance")
            .as_deref(),
        Some("Enter Maintenance Mode")
    );
    let (state, detail, steps, _) = host_change(
        client,
        evidence,
        host,
        member,
        "server.maintenance",
        "host-enter-maintenance",
        "enter-maintenance",
        Duration::from_secs(600),
    );
    assert_eq!(state, OperationState::Succeeded, "{detail} {steps:?}");
    client.wait(
        "the member is in maintenance",
        Duration::from_secs(120),
        |client| !host_enabled(client, member),
    );
}

/// Exit Maintenance Mode for the selected member, once the menu offers it.
fn exit_maintenance(
    client: &mut Client,
    evidence: &mut Evidence,
    outside: &Outside,
    host: &ObjectRef,
    member: HostId,
) {
    let offered = client.wait_for(Duration::from_secs(600), &mut |client| {
        client
            .menu_label(Menu::Server, "server.maintenance")
            .as_deref()
            == Some("Exit Maintenance Mode")
            && host_detail(client, member, |_, live| live).flatten() == Some(true)
    });
    evidence.step(
        "Exit Maintenance Mode is offered",
        json!({ "seconds": offered.map(seconds), "shown": host_shown(client, host, member) }),
    );
    // The host answers again a little before it accepts changes.
    let mut last = None;
    for attempt in 0..6 {
        let (state, detail, steps, _) = host_change(
            client,
            evidence,
            host,
            member,
            "server.maintenance",
            "host-exit-maintenance",
            &format!("exit-maintenance-{attempt}"),
            Duration::from_secs(600),
        );
        if state == OperationState::Succeeded {
            last = None;
            break;
        }
        last = Some(format!("{state:?}: {detail} {steps:?}"));
        client.pass(Duration::from_secs(30));
        if client
            .menu_label(Menu::Server, "server.maintenance")
            .as_deref()
            != Some("Exit Maintenance Mode")
        {
            break;
        }
    }
    let enabled = client.wait_for(Duration::from_secs(600), &mut |client| {
        host_enabled(client, member)
    });
    assert!(
        enabled.is_some(),
        "the member is not enabled after Exit Maintenance Mode: {last:?}"
    );
    assert!(
        outside.settle(Duration::from_secs(120), |inventory| {
            inventory.hosts[&member].enabled
        }),
        "another client reads the member enabled"
    );
    evidence.note("exit_maintenance_refusals", json!(last));
}

/// P3.6: Reboot of the member through maintenance. Enter Maintenance Mode,
/// Reboot from the Server menu through its review, the record followed
/// until the server is back with a newer boot time (three to six minutes
/// on this nested lab), Exit Maintenance Mode.
#[test]
#[ignore]
fn lab_p3_member_reboot_through_maintenance() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-6-member-reboot-through-maintenance");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (host, member, member_name) = empty_member(&mut client, &lab);
    client.select(&host);
    client.tab(TabId::General);
    assert!(host_enabled(&mut client, member));
    enter_maintenance(&mut client, &mut evidence, &host, member);
    let boot_before =
        host_detail(&mut client, member, |details, _| details.boot_time.clone()).flatten();
    let started = std::time::Instant::now();
    let (state, detail, steps, review) = host_change(
        &mut client,
        &mut evidence,
        &host,
        member,
        "server.reboot",
        "host-reboot",
        "reboot",
        Duration::from_secs(2400),
    );
    let back = watch_host(
        &mut client,
        &mut evidence,
        "the member after the reboot, until it is live with a newer boot time",
        &host,
        member,
        Duration::from_secs(1800),
        |client| {
            host_detail(client, member, |details, live| {
                live == Some(true)
                    && details.boot_time.is_some()
                    && details.boot_time != boot_before
            }) == Some(true)
        },
    );
    let boot_after =
        host_detail(&mut client, member, |details, _| details.boot_time.clone()).flatten();
    client.snap(&mut evidence, "back-after-the-reboot");
    evidence.note("server", json!(member_name));
    evidence.note("reboot_state", json!(format!("{state:?}")));
    evidence.note("reboot_record", json!(detail));
    evidence.note("boot_time", json!([boot_before, boot_after]));
    evidence.note("reboot_seconds", json!(seconds(started.elapsed())));
    // The server is brought out of maintenance whatever the record said.
    exit_maintenance(&mut client, &mut evidence, &outside, &host, member);
    client.snap(&mut evidence, "enabled-again");
    assert_eq!(state, OperationState::Succeeded, "{detail} {steps:?}");
    assert!(back.is_some(), "the member did not come back");
    assert_ne!(boot_after, boot_before, "the member has a newer boot time");
    assert!(review.contains("disabled and empty"), "{review}");
    assert_eq!(
        resource_states(&mut client)
            .iter()
            .filter(|state| *state != "Succeeded")
            .count(),
        0,
        "{:?}",
        resource_states(&mut client)
    );
    client.close();
    evidence.finish();
}

/// What libvirt says of a domain of the lab.
fn domain_state(domain: &str) -> String {
    let output = std::process::Command::new("virsh")
        .args(["-c", "qemu:///system", "domstate", domain])
        .output()
        .expect("virsh");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Starts a domain of the lab that is shut off: the power button of the
/// member's machine.
fn start_domain(domain: &str) {
    let output = std::process::Command::new("virsh")
        .args(["-c", "qemu:///system", "start", domain])
        .output()
        .expect("virsh");
    assert!(
        output.status.success(),
        "virsh start: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// P3.6: Shut Down of the member through maintenance. Enter Maintenance
/// Mode, Shut Down from the Server menu through its review, the record
/// until it ends and the machine is off; then the machine is started as an
/// operator presses its power button (libvirt), the window is watched
/// until the server is live again, and Exit Maintenance Mode.
#[test]
#[ignore]
fn lab_p3_member_shutdown_through_maintenance() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-6-member-shutdown-through-maintenance");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (host, member, member_name) = empty_member(&mut client, &lab);
    let domain = lab.hosts[1].name.clone();
    assert_eq!(domain_state(&domain), "running");
    client.select(&host);
    client.tab(TabId::General);
    assert!(host_enabled(&mut client, member));
    enter_maintenance(&mut client, &mut evidence, &host, member);
    let boot_before =
        host_detail(&mut client, member, |details, _| details.boot_time.clone()).flatten();
    let (state, detail, steps, _) = host_change(
        &mut client,
        &mut evidence,
        &host,
        member,
        "server.shutdown",
        "host-shutdown",
        "shut-down",
        Duration::from_secs(1500),
    );
    let off = watch_host(
        &mut client,
        &mut evidence,
        "the member until its machine is off and the client reads it not live",
        &host,
        member,
        Duration::from_secs(900),
        |client| {
            domain_state(&domain) == "shut off"
                && host_detail(client, member, |_, live| live).flatten() == Some(false)
        },
    );
    let shown_off = host_shown(&mut client, &host, member);
    client.snap(&mut evidence, "member-off");
    let machine = domain_state(&domain);
    evidence.step(
        "the member is shut down",
        json!({ "state": format!("{state:?}"), "record": detail, "machine": machine, "shown": shown_off }),
    );
    // Whatever was seen, the machine is started again if it is off.
    if machine == "shut off" {
        start_domain(&domain);
    }
    let back = watch_host(
        &mut client,
        &mut evidence,
        "the member after its machine was started, until it is live with a newer boot time",
        &host,
        member,
        Duration::from_secs(2400),
        |client| {
            host_detail(client, member, |details, live| {
                live == Some(true)
                    && details.boot_time.is_some()
                    && details.boot_time != boot_before
            }) == Some(true)
        },
    );
    client.snap(&mut evidence, "member-back");
    evidence.note("server", json!(member_name));
    evidence.note("shutdown_state", json!(format!("{state:?}")));
    evidence.note("shutdown_record", json!(detail));
    evidence.note("off_after_seconds", json!(off));
    evidence.note("back_after_seconds", json!(back));
    exit_maintenance(&mut client, &mut evidence, &outside, &host, member);
    client.snap(&mut evidence, "enabled-again");
    assert_eq!(state, OperationState::Succeeded, "{detail} {steps:?}");
    assert!(off.is_some(), "the member was not read as off");
    assert!(back.is_some(), "the member did not come back");
    client.close();
    evidence.finish();
}

/// What another administrator of the lab does: `lab/other_admin.py`, pinned
/// to the coordinator's certificate. The script reads the password from
/// the file the environment names; no argument carries it. What it printed
/// and whether it ended well.
fn other_admin(lab: &Lab, arguments: &[&str]) -> (bool, String) {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lab/other_admin.py");
    let fingerprint = lab_support::inspect(&lab.endpoint(0))
        .details
        .fingerprint
        .to_string();
    let output = std::process::Command::new("python3")
        .arg(script)
        .arg(&lab.hosts[0].ip)
        .arg(&fingerprint)
        .args(arguments)
        .output()
        .expect("python3");
    let mut said = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        said.push_str(String::from_utf8_lossy(&output.stderr).trim());
    }
    (output.status.success(), said)
}

/// What the window shows of the connection and of the selected guest.
fn connection_shown(client: &mut Client, pool: &ObjectRef, guest: &ObjectRef) -> serde_json::Value {
    let (current, state, error) = client.model(|m| {
        (
            m.snapshot.as_ref().map(|s| s.is_current()),
            m.snapshot.as_ref().map(|s| format!("{:?}", s.state())),
            m.snapshot
                .as_ref()
                .and_then(|s| s.error().map(str::to_owned)),
        )
    });
    let shut_down = client.chrome("vm.shutdown").map(|element| {
        (element.disabled() == Some(true))
            .then(|| element.description().unwrap_or_default().to_owned())
    });
    json!({
        "status_bar": connection_status(client),
        "current": current,
        "state": state,
        "error": error,
        "pool_row": client.tree_row(pool).map(|row| row.status),
        "guest_row": client.tree_row(guest).map(|row| row.status),
        "tab": client.model(|m| format!("{:?}", m.active_tab())),
        "guest_selected": client.model(|m| m.scope().single().cloned()).as_ref() == Some(guest),
        "has_inventory": client.model(|m| m.inventory.is_some()),
        "shut_down_refused_with": shut_down,
        "error_line": client.model(|m| m.error.clone()),
    })
}

/// P3.8: the coordinator's toolstack restarted by another administrator
/// while the window is connected, with a guest selected and its General
/// tab open. What the status bar, the tree and the tab show while the host
/// is away is recorded; the connection is current again without anything
/// being asked of the user; the selection and the tab are kept; no record
/// of an operation appears and the guest is not restarted. Afterwards a
/// change made by another client is followed again.
///
/// The scenario ends with a wait: nothing is to start on this lab within
/// two minutes of a toolstack restart of the coordinator.
#[test]
#[ignore]
fn lab_p3_recovery_coordinator_toolstack_restart() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-8-coordinator-toolstack-restart");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let (object, vm, name) = guest(&mut client, PLAIN_GUEST);
    let (coordinator, _) = servers(&mut client);
    assert_eq!(coordinator.1, lab.hosts[0].name);
    let pool = client.model(|m| ObjectRef::pool(m.inventory.as_ref().expect("inventory").pool_id));
    client.select(&object);
    client.tab(TabId::General);
    assert!(client.says(&name));
    let start_before = started_at(&mut client, vm);
    let records_before = (
        client.model(|m| m.operation_rows.len()),
        client.model(|m| m.resource_rows.len()),
    );
    let before = connection_shown(&mut client, &pool, &object);
    client.snap(&mut evidence, "before");
    evidence.step("before the restart", before.clone());
    assert_eq!(before["current"], json!(true));

    let (accepted, said) = other_admin(&lab, &["restart-toolstack", &coordinator.0.to_string()]);
    evidence.step(
        "another administrator restarted the coordinator's toolstack",
        json!({ "accepted": accepted, "said": said }),
    );
    let asked = std::time::Instant::now();
    let mut last = before;
    let mut changes = Vec::new();
    let mut away_at = None;
    let mut snapped = false;
    let returned = client.wait_for(Duration::from_secs(900), &mut |client| {
        let shown = connection_shown(client, &pool, &object);
        if shown != last {
            if changes.len() < 60 {
                changes.push(
                    json!({ "at_seconds": seconds(asked.elapsed()), "shown": shown.clone() }),
                );
            }
            last = shown.clone();
        }
        if shown["current"] != json!(true) && away_at.is_none() {
            away_at = Some(seconds(asked.elapsed()));
        }
        if away_at.is_some() && !snapped && asked.elapsed().as_secs_f64() > away_at.unwrap() + 1.5 {
            snapped = true;
            return false;
        }
        away_at.is_some()
            && shown["current"] == json!(true)
            && shown["has_inventory"] == json!(true)
    });
    evidence.step(
        "what the window showed from the restart until the connection was current again",
        json!({ "away_after_seconds": away_at, "current_again_after_seconds": returned.map(|_| seconds(asked.elapsed())), "changes": changes }),
    );
    client.snap(&mut evidence, "current-again");
    let after = connection_shown(&mut client, &pool, &object);
    evidence.step("after the return", after.clone());
    evidence.note("away_after_seconds", json!(away_at));
    evidence.note(
        "current_again_after_seconds",
        json!(returned.map(|_| seconds(asked.elapsed()))),
    );
    evidence.note("shown_while_away", json!(changes));
    // The lab's rule, kept whatever follows.
    let rule = |client: &mut Client| {
        let left = 130_f64 - asked.elapsed().as_secs_f64();
        if left > 0. {
            client.pass(Duration::from_secs_f64(left));
        }
    };
    if !accepted || away_at.is_none() || returned.is_none() {
        rule(&mut client);
    }
    assert!(accepted, "the restart was not accepted: {said}");
    assert!(away_at.is_some(), "the window never showed the host away");
    assert!(
        returned.is_some(),
        "the connection did not become current again"
    );
    assert_eq!(
        after["guest_selected"],
        json!(true),
        "the selection is kept"
    );
    assert_eq!(after["tab"], json!("General"), "the tab is kept");
    assert!(client.says(&name), "the General tab shows the guest");
    assert_eq!(
        (
            client.model(|m| m.operation_rows.len()),
            client.model(|m| m.resource_rows.len()),
        ),
        records_before,
        "no operation was recorded: nothing was sent"
    );
    assert_eq!(
        started_at(&mut client, vm),
        start_before,
        "the guest was not restarted"
    );
    assert_eq!(power_state(&mut client, vm), PowerState::Running);

    // The connection follows the host again: a change made by another
    // client appears in the tab, and is put back.
    let outside = Outside::connect(&lab);
    let previous = outside
        .connection
        .latest()
        .inventory()
        .expect("inventory")
        .vms[&vm]
        .description
        .clone();
    let changed = format!("Changed after the toolstack restart at {}", stamp());
    describe_from_outside(&outside, vm, &changed);
    let followed = client.wait_for(Duration::from_secs(90), &mut |client| client.says(&changed));
    client.snap(&mut evidence, "follows-the-host-again");
    describe_from_outside(&outside, vm, &previous);
    let restored = client.wait_for(Duration::from_secs(90), &mut |client| {
        !client.says(&changed)
    });
    evidence.step(
        "a change from outside after the return",
        json!({ "followed_after_seconds": followed.map(seconds), "put_back": restored.is_some() }),
    );
    evidence.note("guest", json!(name));
    rule(&mut client);
    assert!(
        followed.is_some(),
        "the General tab did not follow a change made after the return"
    );
    client.close();
    evidence.finish();
}

/// Stops a domain of the lab at once, as a power cut does. Only the member
/// may be given.
fn cut_power(lab: &Lab, domain: &str) {
    assert_eq!(domain, lab.hosts[1].name, "only the member may lose power");
    assert_ne!(domain, lab.hosts[0].name);
    let output = std::process::Command::new("virsh")
        .args(["-c", "qemu:///system", "destroy", domain])
        .output()
        .expect("virsh");
    assert!(
        output.status.success(),
        "virsh destroy: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// P3.8: the member lost and returned. With no guest on it, the member's
/// machine loses power (libvirt); the tree, the status bar and the
/// member's General tab are watched until the client reads it as not live
/// (the pool takes up to ten minutes to declare a silent member dead);
/// the machine is started; the window is watched until the member is live
/// and enabled again. The connection to the pool stays current throughout
/// and the selection is kept.
#[test]
#[ignore]
fn lab_p3_recovery_member_lost_and_returned() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-8-member-lost-and-returned");
    let mut client = connected(&lab, &mut evidence, Platform::Rendered);
    let outside = Outside::connect(&lab);
    let (host, member, member_name) = empty_member(&mut client, &lab);
    let domain = lab.hosts[1].name.clone();
    assert_eq!(domain_state(&domain), "running");
    client.select(&host);
    client.tab(TabId::General);
    assert!(host_enabled(&mut client, member));
    let boot_before =
        host_detail(&mut client, member, |details, _| details.boot_time.clone()).flatten();
    let before = host_shown(&mut client, &host, member);
    client.snap(&mut evidence, "before");
    evidence.step("before the power cut", before);

    cut_power(&lab, &domain);
    evidence.step(
        "the member's machine lost power",
        json!(domain_state(&domain)),
    );
    let lost = watch_host(
        &mut client,
        &mut evidence,
        "the member after the power cut, until the client reads it as not live",
        &host,
        member,
        Duration::from_secs(900),
        |client| host_detail(client, member, |_, live| live).flatten() == Some(false),
    );
    let shown_lost = host_shown(&mut client, &host, member);
    let general: Vec<String> = client
        .nodes()
        .into_iter()
        .filter_map(|node| node.text.or(node.value))
        .filter(|text| {
            let text = text.to_lowercase();
            text.contains("live") || text.contains("unreachable") || text.contains("disabled")
        })
        .take(8)
        .collect();
    client.snap(&mut evidence, "member-lost");
    evidence.step(
        "the member is lost",
        json!({ "seconds": lost, "shown": shown_lost, "general_tab_says": general,
            "pool_connection_current": client.model(|m| m.snapshot.as_ref().map(|s| s.is_current())) }),
    );
    evidence.note("read_as_lost_after_seconds", json!(lost));
    evidence.note("shown_when_lost", shown_lost);

    // The machine is started again whatever was seen.
    start_domain(&domain);
    let back = watch_host(
        &mut client,
        &mut evidence,
        "the member after its machine was started, until it is live and enabled",
        &host,
        member,
        Duration::from_secs(2400),
        |client| {
            host_enabled(client, member)
                && host_detail(client, member, |details, live| {
                    live == Some(true)
                        && details.boot_time.is_some()
                        && details.boot_time != boot_before
                }) == Some(true)
        },
    );
    let shown_back = host_shown(&mut client, &host, member);
    client.snap(&mut evidence, "member-back");
    evidence.step(
        "the member is back",
        json!({ "seconds": back, "shown": shown_back }),
    );
    evidence.note("back_after_seconds", json!(back));
    evidence.note("server", json!(member_name));
    if back.is_none() && !host_enabled(&mut client, member) {
        // A member that came back disabled is enabled through the menu.
        let label = client.menu_label(Menu::Server, "server.maintenance");
        evidence.step("the member came back disabled", json!({ "menu": label }));
        if label.as_deref() == Some("Exit Maintenance Mode") {
            exit_maintenance(&mut client, &mut evidence, &outside, &host, member);
        }
    }
    assert!(
        back.is_some(),
        "the member did not come back live and enabled"
    );
    assert!(
        outside.settle(Duration::from_secs(120), |inventory| {
            inventory.hosts[&member].enabled
        }),
        "another client reads the member enabled"
    );
    assert_eq!(
        client.model(|m| m.scope().single().cloned()).as_ref(),
        Some(&host),
        "the selection is kept"
    );
    assert_eq!(
        client.model(|m| m.snapshot.as_ref().map(|s| s.is_current())),
        Some(true)
    );
    assert!(
        lost.is_some(),
        "the client never read the member as not live"
    );
    client.close();
    evidence.finish();
}

/// P3.8: a certificate that is not the pinned one. Add Server is filled in
/// with a pin that is not the coordinator's (the member's certificate
/// fingerprint: a real one, of another host) and with a password that is
/// NOT the lab's. Connect: the attempt ends with the certificate's refusal,
/// not with a refused login, so nothing was authenticated before the
/// certificate was checked. "Review this endpoint's certificate…" then
/// gives the review of the changed certificate: the saved pin and the
/// presented fingerprint side by side, the replacement refused until the
/// exact endpoint is typed and the verification confirmed. With the pin
/// replaced and the lab's password typed, the connection comes up.
#[test]
#[ignore]
fn lab_p3_recovery_changed_certificate() {
    let lab = Lab::load();
    let mut evidence = Evidence::new(&lab, "p3-8-changed-certificate");
    lab_support::assert_fresh_state();
    let mut client = Client::start(Platform::Rendered);
    let presented = lab_support::inspect(&lab.endpoint(0))
        .details
        .fingerprint
        .to_string();
    let wrong = lab_support::inspect(&lab.endpoint(1))
        .details
        .fingerprint
        .to_string();
    assert_ne!(presented, wrong);
    client
        .try_local("server.add")
        .expect("Add Server is on the Home tab");
    assert_eq!(client.front().as_deref(), Some("connect-dialog"));
    client.type_into("host-endpoint", &lab.address(0));
    client.type_into("username", "root");
    client.type_into("password", "not-the-password-of-the-lab");
    client.press("certificate-options");
    client.type_into("certificate-fingerprint", &wrong);
    client.snap(&mut evidence, "form-with-another-pin");
    client.press("connect");
    let ended = client.wait_for(Duration::from_secs(90), &mut |client| {
        client.model(|m| {
            m.error.is_some()
                || m.snapshot
                    .as_ref()
                    .is_some_and(|s| s.error().is_some() || s.is_current())
        }) || client.node("connect-error").is_some()
    });
    client.pass(Duration::from_secs(2));
    client.snap(&mut evidence, "refused");
    let said: Vec<String> = client
        .nodes()
        .into_iter()
        .filter(|node| matches!(node.role, "alert" | "status") || node.tone.is_some())
        .filter_map(|node| node.text.or(node.label))
        .collect();
    let state = client.model(|m| {
        json!({
            "error_line": m.error,
            "connection_state": m.snapshot.as_ref().map(|s| format!("{:?}", s.state())),
            "connection_error": m.snapshot.as_ref().and_then(|s| s.error().map(str::to_owned)),
            "current": m.snapshot.as_ref().map(|s| s.is_current()),
            "has_inventory": m.inventory.is_some(),
        })
    });
    evidence.step(
        "Connect with a pin that is not the host's and a password that is not the lab's",
        json!({ "ended_after_seconds": ended.map(seconds), "front": client.front(), "says": said,
            "model": state, "status_bar": connection_status(&mut client) }),
    );
    panic!("exploration: read the evidence and write the rest");
}
