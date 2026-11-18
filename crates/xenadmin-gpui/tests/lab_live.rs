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
use xenadmin_gpui::charts;
use xenadmin_gpui::shell::status;
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
    evidence.note(
        "power_batch",
        json!("not run: see the report (two halted guests need the tools guest stopped)"),
    );
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

#[allow(dead_code)]
const _: &str = TOOLS_GUEST;
