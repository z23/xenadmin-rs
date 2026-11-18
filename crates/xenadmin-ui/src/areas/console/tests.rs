//! The tests of the console area that need a model.
//!
//! The pure Rune tests moved with their code: `recovery.rs` (six),
//! `canvas.rs` (four), `export.rs` (one). Here are the Rune tests that
//! built a `rune::Runtime`, as harness tests asserting the same thing, each
//! under its Rune name, and one test for each hazard the map lists.
//!
//! No test has a stream: a core `Console` handle exists only on a
//! connection. So what is tested is everything up to the handle: state,
//! eligibility, the descriptions, the requests to the front end. What a
//! connected console sends is tested where it is computed without one
//! (`canvas.rs`).
//!
//! Retired, with the reason:
//!
//! - `console_windows.rs`
//!   `a_console_window_is_drawn_at_the_interface_size_and_follows_changes_to_it`:
//!   layout at an interface scale, and options handed from window to
//!   window. Every window reads the one model's options now, so there is
//!   nothing to hand over; the layout half is on the GPUI list.
//! - The pixel halves of
//!   `the_console_canvas_is_laid_out_in_fit_and_actual_size_modes` and of
//!   `independent_console_windows_keep_targets_frames_and_input_state_separate`
//!   (rectangles, two real windows): on the GPUI list.

use std::sync::Arc;

use xenadmin_core::{ConsoleFrame, ConsoleSnapshot, ConsoleState, PoolId};

use super::canvas::{self, CanvasInput, PointerButton};
use super::commands::{self, Control};
use super::export::tests::Scratch;
use super::scenes::{self, guests, sample, set_snapshot};
use super::session::Session;
use super::view::{self, typing_prompt};
use super::windows::{self, WindowPayload};
use super::{Seat, session, session_mut};
use crate::surface::{Node, duplicate_keys, flatten_blocks};
use crate::{Harness, prelude::*};

fn tab(h: &Harness) -> &Session {
    session(&h.model, Seat::Tab).expect("the tab shows a console")
}

fn tab_mut(h: &mut Harness) -> &mut Session {
    session_mut(&mut h.model, Seat::Tab).expect("the tab shows a console")
}

fn window_nodes(h: &Harness, id: ConsoleWindowId) -> Vec<Node> {
    flatten_blocks("window", &view::window(&h.model, id).blocks)
}

fn keyed<'a>(nodes: &'a [Node], key: &str) -> Option<&'a Node> {
    nodes.iter().find(|node| node.key.as_deref() == Some(key))
}

fn local(h: &Harness, id: CommandId) -> Option<crate::command::MenuItem> {
    h.local_row().into_iter().find(|item| item.id == id)
}

fn pixel(rgba: [u8; 4], state: ConsoleState) -> ConsoleSnapshot {
    ConsoleSnapshot {
        state,
        frame: Some(Arc::new(ConsoleFrame {
            width: 1,
            height: 1,
            rgba: rgba.to_vec().into(),
        })),
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys: false,
        typing: 0,
    }
}

/// Opens a synthetic console for each of the first `count` guests and
/// detaches it, the picture of each tinted by its index.
fn detach_guests(h: &mut Harness, count: usize) -> Vec<ConsoleWindowId> {
    let targets: Vec<_> = guests(&h.model).into_iter().take(count).collect();
    assert_eq!(targets.len(), count);
    targets
        .into_iter()
        .enumerate()
        .filter_map(|(index, target)| {
            super::open_synthetic(&mut h.model, target);
            set_snapshot(&mut h.model, Seat::Tab, sample(40 + index as u8 * 20));
            windows::detach(&mut h.model)
        })
        .collect()
}

fn open_requests(h: &mut Harness) -> Vec<WindowPayload> {
    h.take_intents()
        .into_iter()
        .filter_map(|intent| match intent {
            Intent::ConsoleWindow(ConsoleWindowRequest::Open { payload, .. }) => payload
                .and_then(|payload| payload.downcast::<WindowPayload>().ok())
                .map(|payload| (*payload).clone()),
            _ => None,
        })
        .collect()
}

fn press(h: &mut Harness, seat: Seat, input: &mut CanvasInput, button: PointerButton, down: bool) {
    canvas::pointer_button(&mut h.model, seat, input, button, down, 5.0, 5.0);
}

// ---------------------------------------------------------------------------
// console_view.rs
// ---------------------------------------------------------------------------

/// Rune: `console_view.rs`.
#[test]
fn console_defaults_apply_only_when_opening_a_new_target() {
    let mut m = Model::fixture();
    m.preferences.options.console_actual_size = true;
    m.preferences.options.console_reconnect = false;
    scenes::recovery_scene(&mut m);
    let mut h = Harness::with(m);
    assert!(tab(&h).actual_size);
    assert!(!tab(&h).recovery.enabled);
    h.model.preferences.options.console_actual_size = false;
    h.model.preferences.options.console_reconnect = true;
    assert!(tab(&h).actual_size);
    assert!(!tab(&h).recovery.enabled);
    scenes::recovery_scene(&mut h.model);
    assert!(!tab(&h).actual_size);
    assert!(tab(&h).recovery.enabled);
}

/// Rune: `console_view.rs`.
#[test]
fn clipboard_text_is_typed_only_after_a_review_that_never_shows_it() {
    let mut h = Harness::scene("console-frame");
    assert!(
        h.find(view::TYPING_REVIEW).is_none(),
        "nothing is pending at first"
    );

    // Text a US keyboard cannot type is refused with the reason, and nothing is pending.
    tab_mut(&mut h).arm_typing("caf\u{e9} au lait");
    assert!(tab(&h).typed.is_none());
    assert!(
        tab(&h)
            .message
            .as_deref()
            .is_some_and(|m| m.contains("US keyboard")),
        "{:?}",
        tab(&h).message
    );
    assert!(h.find(view::MESSAGE).is_some(), "the refusal is on screen");

    // Typable text waits for confirmation; the review gives the amount and not the text.
    tab_mut(&mut h).arm_typing("Secret-Passw0rd!\r\nsecond line\n");
    let pending = tab(&h).typed.as_ref().expect("the text is pending");
    let prompt = typing_prompt(pending.review, "Lab guest");
    assert!(
        prompt.starts_with(
            "Type 29 characters, with 2 line breaks sent as Enter, into Lab guest as key presses?"
        ),
        "{prompt}"
    );
    assert!(!prompt.contains("Secret") && !prompt.contains("second"));
    assert_eq!(
        typing_prompt(xenadmin_core::TypedText::review("x").unwrap(), "a host"),
        typing_prompt(
            xenadmin_core::TypedText {
                characters: 1,
                line_breaks: 0
            },
            "a host"
        )
    );
    assert!(
        typing_prompt(xenadmin_core::TypedText::review("x").unwrap(), "a host")
            .starts_with("Type 1 character into a host as key presses?")
    );
    assert!(
        h.find(view::TYPING_REVIEW).is_some(),
        "the review is on screen"
    );
    let name = tab(&h).target.name.clone();
    let shown = h.node(view::TYPING_PROMPT).text.unwrap_or_default();
    assert_eq!(shown, typing_prompt(tab(&h).review().unwrap(), &name));
    assert!(shown.starts_with("Type 29 characters, with 2 line breaks sent as Enter, into "));
    assert!(
        !h.has_text("Secret") && !h.has_text("second line"),
        "nothing on screen shows the text"
    );

    // The pending text belongs to the console it was read for.
    tab_mut(&mut h).serial += 1;
    assert!(
        h.find(view::TYPING_REVIEW).is_none(),
        "another console does not offer the earlier text"
    );
    tab_mut(&mut h).confirm_typing();
    assert!(tab(&h).typed.is_none(), "and confirming it types nothing");

    // Cancelling or confirming ends the review.
    tab_mut(&mut h).arm_typing("ls -la");
    h.click(view::CANCEL_TYPING);
    assert!(tab(&h).typed.is_none());
    assert!(h.find(view::TYPING_REVIEW).is_none());
    tab_mut(&mut h).arm_typing("ls -la");
    // No stream in a synthetic pool: "Type it" says why it cannot, and a
    // confirmation that got through anyway types nothing and says so.
    assert_eq!(
        h.try_click(view::CONFIRM_TYPING),
        Err(super::recovery::Waiting::Connection.message().into())
    );
    tab_mut(&mut h).confirm_typing();
    assert!(tab(&h).typed.is_none());
    assert_eq!(
        tab(&h).message.as_deref(),
        Some("The console is not connected; nothing was typed.")
    );
}

/// Rune: `console_view.rs`, the model half. What the canvas is told in each
/// mode; the rectangles are the GPUI test's.
#[test]
fn the_console_canvas_is_laid_out_in_fit_and_actual_size_modes() {
    let h = Harness::scene("console-frame");
    let canvas_node = h.node(view::CANVAS);
    assert_eq!(canvas_node.role, "custom");
    assert_eq!(canvas_node.label.as_deref(), Some("console-canvas"));
    assert_eq!(canvas_node.value.as_deref(), Some(r#"{"session":"tab"}"#));
    assert_eq!(canvas::desktop_size(&h.model, Seat::Tab), Some([800, 600]));
    assert!(!canvas::actual_size(&h.model, Seat::Tab));
    assert_eq!(
        canvas::content_size(&h.model, Seat::Tab),
        None,
        "fit fills the panel"
    );
    let fit = canvas::geometry(&h.model, Seat::Tab, [1000.0, 600.0]).unwrap();
    assert_eq!(fit.scale, 1.0);
    assert_eq!(fit.offset, [100.0, 0.0]);

    let h = Harness::scene("console-actual-size");
    assert!(h.find(view::CANVAS).is_some());
    assert!(canvas::actual_size(&h.model, Seat::Tab));
    assert_eq!(
        canvas::content_size(&h.model, Seat::Tab),
        Some([800.0, 600.0]),
        "actual size shows the frame at its own dimensions"
    );
    let actual = canvas::geometry(&h.model, Seat::Tab, [800.0, 600.0]).unwrap();
    assert_eq!(
        (actual.scale, actual.offset),
        (1.0, [0.0, 0.0]),
        "the frame starts at the view's origin"
    );
    assert_eq!(
        local(&h, commands::ACTUAL_SIZE).and_then(|item| item.checked),
        Some(true)
    );
}

// ---------------------------------------------------------------------------
// console_recovery_tests.rs
// ---------------------------------------------------------------------------

/// Rune: `console_recovery_tests.rs`. The two window sizes were layout.
#[test]
fn automatic_reconnect_toggle_and_pinned_identity_render_at_both_window_sizes() {
    let mut h = Harness::scene("console-recovery");
    let id = tab(&h).target.vm;
    assert_eq!(h.node(view::AUTO_RECONNECT).checked, Some(true));
    assert!(h.find(view::RECOVERY_STATUS).is_none());
    h.toggle(view::AUTO_RECONNECT);
    assert!(!tab(&h).recovery.enabled);
    assert_eq!(h.node(view::AUTO_RECONNECT).checked, Some(false));
    assert_eq!(tab(&h).target.vm, id);
    assert!(!tab(&h).input_ready());
    // Pinned for the sweep of decision 10 to find.
    assert_eq!(
        h.node(view::RECOVERY_STATUS).text.as_deref(),
        Some("Automatic reconnect is off. Use Reconnect when the guest or host is available.")
    );
    assert_eq!(
        h.node(view::STATUS).text.as_deref(),
        Some("Console closed. Reconnect when the guest or host is available.")
    );
}

// ---------------------------------------------------------------------------
// console_export.rs
// ---------------------------------------------------------------------------

/// Rune: `console_export.rs`.
#[test]
fn chooser_captures_one_frame_and_cancel_or_console_switch_does_not_write() {
    let directory = Scratch::new("xenadmin-png-dialog");
    let mut h = Harness::scene("console-recovery");
    let red = [255, 0, 0, 255];
    set_snapshot(&mut h.model, Seat::Tab, pixel(red, ConsoleState::Connected));
    let pick = |h: &mut Harness| {
        h.click(commands::SAVE_SCREENSHOT.0);
        let intents = &h.model.intents;
        let Some(Intent::ChooseFile(request)) = intents.last() else {
            panic!("expected a save prompt, got {intents:?}");
        };
        assert_eq!(request.mode, FileMode::Save);
        assert_eq!(request.title, "Save console screenshot");
        assert_eq!(request.suggested_name.as_deref(), Some("vm-console.png"));
        assert_eq!(request.filters[0].name, "PNG image");
        assert_eq!(request.filters[0].extensions, ["png"]);
    };
    pick(&mut h);
    assert!(tab(&h).export.pending);
    assert_eq!(
        h.try_click(commands::SAVE_SCREENSHOT.0),
        Err(commands::EXPORTING.into()),
        "one prompt at a time"
    );
    h.answer_file(None);
    assert!(!tab(&h).export.pending);

    pick(&mut h);
    let path = directory.0.join("frame.png");
    // The console moves on while the prompt is open; what is saved is the
    // frame of the press.
    set_snapshot(
        &mut h.model,
        Seat::Tab,
        pixel([0, 255, 0, 255], ConsoleState::Connected),
    );
    h.answer_file(Some(path.clone()));
    assert_eq!(
        tab(&h).message.as_deref(),
        Some("Saving the captured console frame…")
    );
    for _ in 0..500 {
        if tab_mut(&mut h).poll(0.0).changed {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!tab(&h).export.pending);
    assert_eq!(
        tab(&h).message,
        Some(format!("Console screenshot saved to {}", path.display()))
    );
    assert_eq!(image::open(&path).unwrap().to_rgba8().as_raw(), &red);

    pick(&mut h);
    tab_mut(&mut h).reconnecting();
    let stale = directory.0.join("stale.png");
    h.answer_file(Some(stale.clone()));
    assert!(!stale.exists());
    assert!(!tab(&h).export.pending);
}

// ---------------------------------------------------------------------------
// host_console.rs
// ---------------------------------------------------------------------------

/// Rune: `host_console.rs`. With a host console open the selection IS the
/// host (Rune faked that through `State::selected`), and nothing of a guest
/// is offered: no Open Console, none of the guest chords. The Rune
/// assertions that the details pane had no `open-properties` and no
/// `open-snapshots` button are retired with the details pane.
#[test]
fn host_console_keeps_host_details_and_never_offers_guest_power_or_device_actions() {
    let mut h = Harness::scene("host-console");
    let target = tab(&h).target.clone();
    let host = target.host.expect("a host console");
    let scope = h.model.scope().single().cloned().expect("one object");
    assert_eq!(scope.kind, ObjectKind::Host);
    assert_eq!(scope.host_id(), Some(host));
    assert_eq!(scope, target.object());
    assert!(h.model.tabs().contains(&TabId::Console));
    assert_eq!(h.model.active_tab(), TabId::Console);
    assert_eq!(h.eligibility(commands::OPEN.0), Eligibility::Hidden);
    assert_eq!(
        h.eligibility(commands::SEND_CTRL_ALT_DEL.0),
        Eligibility::Hidden
    );
    let row: Vec<_> = h.local_row().into_iter().map(|item| item.id).collect();
    for guest_only in [
        commands::OPEN,
        commands::OPEN_HOST,
        commands::SEND_CTRL_ALT_DEL,
        commands::SEND_CTRL_ALT_F1,
        commands::SEND_ALT_TAB,
    ] {
        assert!(!row.contains(&guest_only), "{guest_only:?} in {row:?}");
    }
    assert!(row.contains(&commands::SEND_CTRL_C) && row.contains(&commands::SEND_CTRL_D));
    assert_eq!(
        local(&h, commands::SEND_CLIPBOARD).map(|item| item.label),
        Some("Send Clipboard to Host".into())
    );
    assert_eq!(
        local(&h, commands::COPY_CLIPBOARD).map(|item| item.label),
        Some("Copy Host Clipboard".into())
    );
    assert!(target.heading().starts_with("Host text console · "));
    assert!(matches!(
        h.tab().blocks().first(),
        Some(Block::Heading(title)) if *title == target.heading()
    ));

    // The control domain the host names is what makes a host console.
    let mut inventory = h.model.inventory.take().unwrap().as_ref().clone();
    assert_eq!(commands::control_domain(&inventory, host), Some(target.vm));
    inventory
        .details
        .hosts
        .get_mut(&host)
        .unwrap()
        .control_domain = None;
    assert_eq!(commands::control_domain(&inventory, host), None);
    h.model.inventory = Some(Arc::new(inventory));
    assert_eq!(tab(&h).target.vm, target.vm);
}

// ---------------------------------------------------------------------------
// console_windows.rs
// ---------------------------------------------------------------------------

/// Rune: `console_windows.rs`, the model half: the session moves whole, each
/// window has its own target and picture, input state is per window, and
/// closing one leaves the other.
#[test]
fn independent_console_windows_keep_targets_frames_and_input_state_separate() {
    let mut h = Harness::scene("console-recovery");
    h.take_intents();
    let targets = guests(&h.model);
    let (first, second) = (targets[0].clone(), targets[1].clone());

    super::open_synthetic(&mut h.model, first.clone());
    set_snapshot(&mut h.model, Seat::Tab, sample(40));
    // The main window holds a button when the console moves.
    let mut main_input = CanvasInput::new();
    main_input.set_bounds(640.0, 360.0);
    press(&mut h, Seat::Tab, &mut main_input, PointerButton::Right, true);
    assert_eq!(main_input.buttons(), 4);
    let generation = canvas::frame_generation(&h.model, Seat::Tab);
    assert_ne!(generation, 0);
    let serial = tab(&h).serial;
    let a = windows::detach(&mut h.model).expect("a free window");
    assert!(super::tab_target(&h.model).is_none());
    assert_eq!(canvas::frame_generation(&h.model, Seat::Tab), 0);
    assert!(canvas::frame(&h.model, Seat::Tab).is_none());
    let moved = session(&h.model, Seat::Window(a)).unwrap();
    assert_ne!(moved.serial, serial, "replies on their way are void");
    assert_eq!(
        canvas::frame_generation(&h.model, Seat::Window(a)),
        generation,
        "the picture moved with the console"
    );
    // The new window starts with nothing held.
    let mut a_input = CanvasInput::new();
    canvas::pointer_move(&mut h.model, Seat::Window(a), &mut a_input, 1.0, 1.0);
    assert_eq!(a_input.buttons(), 0);

    super::open_synthetic(&mut h.model, second.clone());
    set_snapshot(&mut h.model, Seat::Tab, sample(90));
    let b = windows::detach(&mut h.model).expect("a second free window");
    assert_ne!(a, b);
    let requests = open_requests(&mut h);
    assert_eq!(
        requests.iter().map(|r| r.id).collect::<Vec<_>>(),
        [a, b],
        "one Open request for each"
    );
    assert_eq!(requests[0].title, view::window(&h.model, a).title);
    assert_eq!(Some(requests[1].lease), windows::lease(&h.model, b));

    // The main window moves on to something else entirely.
    h.model.load_demo(3, 48);
    let vm_of = |h: &Harness, id| session(&h.model, Seat::Window(id)).map(|s| s.target.vm);
    assert_eq!(vm_of(&h, a), Some(first.vm));
    assert_eq!(vm_of(&h, b), Some(second.vm));
    let tint = |h: &Harness, id| canvas::frame(&h.model, Seat::Window(id)).unwrap().rgba[0];
    assert_eq!((tint(&h, a), tint(&h, b)), (40, 90));
    assert_ne!(
        canvas::frame_generation(&h.model, Seat::Window(a)),
        canvas::frame_generation(&h.model, Seat::Window(b))
    );
    assert_eq!(
        view::window(&h.model, a).title,
        format!("{} · XenAdmin RS console", first.name)
    );

    // Each window holds its own input; releasing one leaves the other.
    let mut b_input = CanvasInput::new();
    press(&mut h, Seat::Window(a), &mut a_input, PointerButton::Right, true);
    press(&mut h, Seat::Window(b), &mut b_input, PointerButton::Middle, true);
    assert_eq!((a_input.buttons(), b_input.buttons()), (4, 2));
    commands::run(&mut h.model, Seat::Window(a), Control::ReleaseInput);
    canvas::pointer_move(&mut h.model, Seat::Window(a), &mut a_input, 6.0, 6.0);
    canvas::pointer_move(&mut h.model, Seat::Window(b), &mut b_input, 6.0, 6.0);
    assert_eq!((a_input.buttons(), b_input.buttons()), (0, 2));

    // A window's own "Close console", then the front end's confirmation.
    let nodes = window_nodes(&h, a);
    assert!(keyed(&nodes, "close-console").is_some_and(|node| node.disabled.is_none()));
    assert!(
        keyed(&nodes, "detach-console").is_none(),
        "a window cannot be detached again"
    );
    let lease = windows::lease(&h.model, a).unwrap();
    h.take_intents();
    commands::run(&mut h.model, Seat::Window(a), Control::Close);
    assert!(session(&h.model, Seat::Window(a)).is_none());
    assert!(matches!(
        h.take_intents().as_slice(),
        [Intent::ConsoleWindow(ConsoleWindowRequest::Close(id))] if *id == a
    ));
    assert!(windows::closed(&mut h.model, a, lease));
    assert_eq!(vm_of(&h, b), Some(second.vm));
    assert_eq!(windows::open_windows(&h.model).len(), 1);
    let closed = view::window(&h.model, a);
    assert_eq!(closed.title, view::WINDOW_TITLE);
    assert!(matches!(&closed.blocks[0], Block::Heading(title) if title == "Console closed"));
}

/// Rune: `console_windows.rs`.
#[test]
fn reusable_slot_rejects_old_adoption_close_and_focus_requests() {
    let mut h = Harness::scene("console-recovery");
    let target = tab(&h).target.clone();
    let first = windows::detach(&mut h.model).unwrap();
    let old = windows::lease(&h.model, first).unwrap();
    windows::close(&mut h.model, first);
    assert!(windows::lease(&h.model, first).is_none());
    // Close must be acknowledged before reuse: the next console goes
    // elsewhere while the first window is still on its way out.
    super::open_synthetic(&mut h.model, target.clone());
    let elsewhere = windows::detach(&mut h.model).unwrap();
    assert_ne!(elsewhere, first);
    assert!(
        !windows::closed(&mut h.model, first, old + 1000),
        "not this lease"
    );
    assert!(windows::closed(&mut h.model, first, old));
    super::open_synthetic(&mut h.model, target);
    let again = windows::detach(&mut h.model).unwrap();
    assert_eq!(again, first, "the slot is free again");
    let new = windows::lease(&h.model, first).unwrap();
    assert_ne!(old, new);
    // The old window's late close cannot end the new console.
    assert!(!windows::closed(&mut h.model, first, old));
    assert_eq!(windows::lease(&h.model, first), Some(new));
    assert!(session(&h.model, Seat::Window(first)).is_some());
    // A window that shows nothing is not raised.
    h.take_intents();
    windows::show(&mut h.model, ConsoleWindowId(8));
    assert!(h.take_intents().is_empty());
    windows::show(&mut h.model, first);
    assert!(matches!(
        h.take_intents().as_slice(),
        [Intent::ConsoleWindow(ConsoleWindowRequest::Activate(id))] if *id == first
    ));
}

/// Rune: `console_windows.rs`.
#[test]
fn window_capacity_is_bounded_and_closing_returns_the_slot() {
    let mut h = Harness::scene("console-recovery");
    let target = tab(&h).target.clone();
    assert!(windows::can_detach(&h.model));
    assert_eq!(
        h.eligibility(commands::UNDOCK.0),
        Eligibility::Enabled,
        "a synthetic console can be detached, as in Rune"
    );
    for _ in 0..windows::LIMIT {
        super::open_synthetic(&mut h.model, target.clone());
        assert!(windows::detach(&mut h.model).is_some());
    }
    super::open_synthetic(&mut h.model, target);
    assert!(!windows::can_detach(&h.model));
    assert_eq!(
        h.try_click(commands::UNDOCK.0),
        Err(windows::LIMIT_MESSAGE.into())
    );
    assert_eq!(windows::detach(&mut h.model), None);
    assert_eq!(h.model.error.as_deref(), Some(windows::LIMIT_MESSAGE));
    assert!(super::tab_target(&h.model).is_some(), "the console stays");

    let third = ConsoleWindowId(3);
    let lease = windows::lease(&h.model, third).unwrap();
    windows::close(&mut h.model, third);
    assert!(!windows::can_detach(&h.model));
    assert!(windows::closed(&mut h.model, third, lease));
    assert!(windows::can_detach(&h.model));
    windows::close_all(&mut h.model);
    assert!(windows::open_windows(&h.model).is_empty());
    assert!(super::tab_target(&h.model).is_some());
}

/// Rune: `console_windows.rs`. The manager dialog is the Window menu now:
/// one entry per open window, and Close All Consoles. Its two window sizes
/// were layout.
#[test]
fn manager_is_reachable_with_eight_targets_at_compact_and_wide_sizes() {
    let mut h = Harness::with(Model::demo(3, 48));
    assert_eq!(
        h.try_invoke(commands::CLOSE_ALL_WINDOWS.0),
        Err("No console window is open.".into())
    );
    assert!(h.menu_item(commands::WINDOWS[0].0).is_none());
    let ids = detach_guests(&mut h, windows::LIMIT);
    assert_eq!(ids, windows::ids().collect::<Vec<_>>());
    h.take_intents();
    let names: Vec<_> = guests(&h.model)
        .into_iter()
        .take(windows::LIMIT)
        .map(|target| target.heading())
        .collect();
    for (index, command) in commands::WINDOWS.iter().enumerate() {
        let item = h.menu_item(command.0).expect("an entry for each window");
        assert_eq!(item.label, names[index]);
        assert!(item.disabled.is_none());
    }
    h.invoke(commands::WINDOWS[7].0);
    assert!(matches!(
        h.take_intents().as_slice(),
        [Intent::ConsoleWindow(ConsoleWindowRequest::Activate(
            ConsoleWindowId(8)
        ))]
    ));
    let lease = windows::lease(&h.model, ConsoleWindowId(8)).unwrap();
    windows::close(&mut h.model, ConsoleWindowId(8));
    assert!(h.menu_item(commands::WINDOWS[7].0).is_none());
    assert!(windows::closed(&mut h.model, ConsoleWindowId(8), lease));
    h.take_intents();
    h.invoke(commands::CLOSE_ALL_WINDOWS.0);
    assert!(windows::open_windows(&h.model).is_empty());
    let closes = h
        .take_intents()
        .into_iter()
        .filter(|intent| matches!(intent, Intent::ConsoleWindow(ConsoleWindowRequest::Close(_))))
        .count();
    assert_eq!(closes, 7);
    for command in commands::WINDOWS {
        assert!(h.menu_item(command.0).is_none());
    }
}

/// Rune: `console_windows.rs`.
#[test]
fn moving_console_invalidates_old_file_dialog_without_disabling_new_window_export() {
    let directory = Scratch::new("xenadmin-png-moved");
    let mut h = Harness::scene("console-recovery");
    set_snapshot(&mut h.model, Seat::Tab, sample(30));
    h.click(commands::SAVE_SCREENSHOT.0);
    assert!(tab(&h).export.pending);
    let id = windows::detach(&mut h.model).unwrap();
    let path = directory.0.join("obsolete.png");
    h.answer_file(Some(path.clone()));
    assert!(!path.exists());
    let seat = Seat::Window(id);
    assert!(!session(&h.model, seat).unwrap().export.pending);
    let nodes = window_nodes(&h, id);
    assert!(
        keyed(&nodes, "save-console-screenshot").is_some_and(|node| node.disabled.is_none()),
        "the window can save a screenshot of its own"
    );
    commands::run(&mut h.model, seat, Control::SaveScreenshot);
    assert!(session(&h.model, seat).unwrap().export.pending);
    assert!(
        h.model
            .intents
            .iter()
            .any(|intent| matches!(intent, Intent::ChooseFile(_)))
    );
    h.answer_file(None);
    assert!(!session(&h.model, seat).unwrap().export.pending);
}

// ---------------------------------------------------------------------------
// settings.rs: the test of the scene this area registers for it
// ---------------------------------------------------------------------------

/// Rune: `settings.rs` `compact_console_details_returns_to_same_pinned_guest`.
/// The details are the General tab now; going there and back shows the same
/// console.
#[test]
fn compact_console_details_returns_to_same_pinned_guest() {
    let mut h = Harness::scene("console-details");
    assert_eq!(h.model.active_tab(), TabId::General);
    let target = tab(&h).target.clone();
    assert_eq!(h.model.scope().single(), Some(&target.object()));
    h.open_tab(TabId::Console);
    assert_eq!(tab(&h).target, target);
    assert!(h.find(view::CANVAS).is_some());
    h.open_tab(TabId::General);
    h.open_tab(TabId::Console);
    assert_eq!(tab(&h).target, target);
}

// ---------------------------------------------------------------------------
// What the port changed, and the hazards of the map
// ---------------------------------------------------------------------------

#[test]
fn the_tab_row_is_the_rune_toolbar_and_every_disabled_control_says_why() {
    let h = Harness::scene("console-frame");
    let row = h.local_row();
    let ids: Vec<_> = row.iter().map(|item| item.id).collect();
    assert_eq!(
        ids,
        [
            commands::RECONNECT,
            commands::ACTUAL_SIZE,
            commands::FULLSCREEN,
            commands::UNDOCK,
            commands::RELEASE_INPUT,
            commands::SAVE_SCREENSHOT,
            commands::SEND_CTRL_ALT_DEL,
            commands::SEND_CTRL_ALT_F1,
            commands::SEND_ALT_TAB,
            commands::SEND_CLIPBOARD,
            commands::TYPE_CLIPBOARD,
            commands::COPY_CLIPBOARD,
            commands::CLOSE,
        ]
    );
    let labels: Vec<_> = row.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Reconnect",
            "Actual Size",
            "Fullscreen",
            "Open in Separate Window",
            "Release Input",
            "Save Screenshot…",
            "Send Ctrl+Alt+Del",
            "Ctrl+Alt+F1",
            "Alt+Tab",
            "Send Clipboard to VM",
            "Type Clipboard as Keys…",
            "Copy VM Clipboard",
            "Close Console",
        ]
    );
    assert!(!labels.iter().any(|label| label.contains("right button")));
    // A synthetic pool has no connection: whatever would send something
    // waits for one, and says so. Rune disabled the buttons silently.
    let waiting = super::recovery::Waiting::Connection.message();
    let sends = [
        commands::SEND_CTRL_ALT_DEL,
        commands::SEND_CTRL_ALT_F1,
        commands::SEND_ALT_TAB,
        commands::SEND_CLIPBOARD,
        commands::TYPE_CLIPBOARD,
        commands::COPY_CLIPBOARD,
    ];
    for item in &row {
        let expected = sends.contains(&item.id).then_some(waiting);
        assert_eq!(item.disabled.as_deref(), expected, "{:?}", item.id);
    }
    assert_eq!(
        local(&h, commands::ACTUAL_SIZE).and_then(|item| item.checked),
        Some(false)
    );
    assert_eq!(
        h.menu_item(commands::SEND_CTRL_ALT_DEL.0)
            .and_then(|item| item.disabled),
        Some(waiting.into()),
        "the VM menu has it too"
    );
    assert_eq!(
        h.node(view::STATUS).text.as_deref(),
        Some("Connected directly to the host console")
    );
    assert_eq!(h.node(view::HELP).text.as_deref(), Some(view::HELP_TEXT));
    assert!(view::HELP_TEXT.contains("Clipboard exchange is experimental"));
    let nodes = flatten_blocks("tab", &h.tab().blocks());
    assert_eq!(duplicate_keys(&nodes), Vec::<String>::new());
    assert_eq!(
        nodes.iter().filter(|node| node.role == "custom").count(),
        1,
        "one canvas"
    );

    // No picture, stream closed: no screenshot, and Reconnect is offered.
    let h = Harness::scene("console-recovery");
    assert_eq!(
        local(&h, commands::SAVE_SCREENSHOT).and_then(|item| item.disabled),
        Some(commands::NO_FRAME.into())
    );
    assert_eq!(
        local(&h, commands::RECONNECT).and_then(|item| item.disabled),
        None
    );
}

#[test]
fn the_controls_change_the_session_and_ask_the_front_end() {
    let mut h = Harness::scene("console-frame");
    let before = tab(&h).input_epoch();
    h.click(commands::ACTUAL_SIZE.0);
    assert!(tab(&h).actual_size);
    assert_ne!(tab(&h).input_epoch(), before, "input is released first");
    h.click(commands::ACTUAL_SIZE.0);
    assert!(!tab(&h).actual_size);

    // Fullscreen is the front end's: the tab's console asks for the main
    // window, once, and input is released first.
    let epoch = tab(&h).input_epoch();
    h.click(commands::FULLSCREEN.0);
    assert_ne!(tab(&h).input_epoch(), epoch);
    assert_eq!(
        format!("{:?}", h.take_intents()),
        "[ToggleFullscreen(Main)]"
    );
    assert!(super::take_fullscreen_requests(&mut h.model).is_empty());

    let epoch = tab(&h).input_epoch();
    h.click(commands::RELEASE_INPUT.0);
    assert_ne!(tab(&h).input_epoch(), epoch);

    // Reconnect in a synthetic pool: there is no snapshot to open from, and
    // the refusal is on the shared error line as in Rune.
    tab_mut(&mut h).recovery.attempts = 4;
    h.click(commands::RECONNECT.0);
    assert_eq!(h.model.error, Some(xenadmin_core::Error::Stale.to_string()));
    assert_eq!(tab(&h).recovery.attempts, 0, "the budget starts over");

    // The guest offered clipboard text; the stream is not ready, so it
    // still cannot be copied.
    let mut offered = (**tab(&h).snapshot().unwrap()).clone();
    offered.guest_clipboard = Some("from the guest".into());
    set_snapshot(&mut h.model, Seat::Tab, offered);
    assert!(local(&h, commands::COPY_CLIPBOARD).is_some_and(|item| item.disabled.is_some()));

    h.click(commands::CLOSE.0);
    assert!(super::tab_target(&h.model).is_none());
    assert_eq!(
        h.node(view::CLOSED).text.as_deref(),
        Some("No console is open for this VM.")
    );
    let row = h.local_row();
    assert_eq!(row.len(), 2, "{row:?}");
    assert_eq!(row[0].id, commands::OPEN);
    assert_eq!(row[0].label, "Open Console");
    assert_eq!(row[0].disabled.as_deref(), Some(commands::DEMO));
    assert_eq!(row[1].id, commands::SEND_CTRL_ALT_DEL);
    assert_eq!(row[1].disabled.as_deref(), Some(commands::NOT_OPEN));
}

#[test]
fn opening_needs_a_guest_or_a_server_with_a_running_control_domain() {
    let mut h = Harness::scene("host-console");
    super::close_tab(&mut h.model);
    let host = h.model.scope().clone();
    // Demo first, as Rune's `s.demo ||`.
    assert_eq!(
        h.eligibility(commands::OPEN_HOST.0),
        Eligibility::disabled(commands::DEMO)
    );
    assert_eq!(
        h.node(view::CLOSED).text.as_deref(),
        Some("No text console is open for this server.")
    );
    assert!(
        h.context_menu(&host)
            .iter()
            .any(|item| item.id == commands::OPEN_HOST && item.label == "Open Host Text Console")
    );
    // A guest: Open Console on its context menu; not for a server or
    // several guests.
    let targets = guests(&h.model);
    let guest = Target::one(targets[0].object());
    assert!(
        h.context_menu(&guest)
            .iter()
            .any(|item| item.id == commands::OPEN)
    );
    assert_eq!(
        h.model.eligibility(commands::OPEN, &host),
        Eligibility::Hidden
    );
    assert_eq!(
        h.model.eligibility(commands::OPEN_HOST, &guest),
        Eligibility::Hidden
    );
    let both = Target::many([targets[0].object(), targets[1].object()]);
    assert_eq!(
        h.model.eligibility(commands::OPEN, &both),
        Eligibility::Hidden
    );
    // The control domain itself is not a guest.
    let control = Target::one(ObjectRef::vm(
        targets[0].pool,
        xenadmin_core::VmId::from(uuid::Uuid::from_u128(900)),
    ));
    assert_eq!(
        h.model.eligibility(commands::OPEN, &control),
        Eligibility::Hidden
    );
    // Opening without a connection is refused by the model as well.
    assert!(super::open(&mut h.model, &targets[0].object()).is_err());
    assert!(super::tab_target(&h.model).is_none());
    // Pinned for the sweep of decision 10.
    assert_eq!(
        commands::READ_ONLY,
        "Read-only connection: changes and interactive consoles are disabled."
    );
}

#[test]
fn the_tabs_console_belongs_to_the_selected_object_and_windows_outlive_it() {
    let mut h = Harness::scene("console-frame");
    let targets = guests(&h.model);
    let shown = tab(&h).target.clone();
    let other = targets
        .iter()
        .find(|target| target.vm != shown.vm)
        .unwrap()
        .clone();
    // Detach one console, then open the tab's again.
    let window = windows::detach(&mut h.model).unwrap();
    scenes::show(&mut h.model, shown.clone());
    assert_eq!(super::tab_target(&h.model), Some(&shown));
    assert!(
        !(super::spec().poll)(&mut h.model, 1.0),
        "a quiet console is no reason to render again"
    );
    h.tick(0.1);
    assert_eq!(super::tab_target(&h.model), Some(&shown));

    // Another object selected: the tab's console ends, the window's stays.
    h.select(Target::one(other.object()));
    assert!(super::tab_target(&h.model).is_none());
    let window_vm = |h: &Harness| session(&h.model, Seat::Window(window)).map(|s| s.target.vm);
    assert_eq!(window_vm(&h), Some(shown.vm));
    // Another session shown: the same.
    scenes::show(&mut h.model, other.clone());
    h.model.load_demo(2, 8);
    assert!(super::tab_target(&h.model).is_none());
    assert_eq!(window_vm(&h), Some(shown.vm));
    // The Console tab of an object without an open console is still there.
    let pool: PoolId = h.model.inventory.as_ref().unwrap().pool_id;
    let guest = guests(&h.model)[0].clone();
    assert_eq!(guest.pool, pool);
    h.select(Target::one(guest.object()));
    h.open_tab(TabId::Console);
    assert!(h.find(view::CLOSED).is_some());
    assert!(h.find(view::CANVAS).is_none());
}

#[test]
fn a_frame_has_a_generation_that_is_never_reused_and_nothing_stale_survives_a_reopen() {
    let mut h = Harness::scene("console-recovery");
    assert_eq!(canvas::frame_generation(&h.model, Seat::Tab), 0);
    assert!(canvas::frame(&h.model, Seat::Tab).is_none());
    assert!(canvas::desktop_size(&h.model, Seat::Tab).is_none());

    let first = Arc::new(pixel([1, 2, 3, 255], ConsoleState::Connected));
    tab_mut(&mut h).set_snapshot(Some(first.clone()));
    let one = canvas::frame_generation(&h.model, Seat::Tab);
    assert_ne!(one, 0);
    let frame = canvas::frame(&h.model, Seat::Tab).unwrap();
    assert_eq!((frame.generation, frame.width, frame.height), (one, 1, 1));
    assert!(
        Arc::ptr_eq(&frame.rgba, &first.frame.as_ref().unwrap().rgba),
        "the pixels are shared, not copied"
    );

    // Another snapshot with the same frame (a text is being typed): the
    // same picture, no upload.
    let mut same = (*first).clone();
    same.typing = 12;
    tab_mut(&mut h).set_snapshot(Some(Arc::new(same)));
    assert_eq!(canvas::frame_generation(&h.model, Seat::Tab), one);
    assert_eq!(
        h.node(view::TYPING_PROGRESS).text.as_deref(),
        Some("Typing as keys: 12 characters left.")
    );

    // A new frame: a new generation.
    set_snapshot(
        &mut h.model,
        Seat::Tab,
        pixel([9, 9, 9, 255], ConsoleState::Connected),
    );
    let two = canvas::frame_generation(&h.model, Seat::Tab);
    assert!(two != one && two != 0);

    // The stream is replaced: no picture until the new one sends a frame.
    tab_mut(&mut h).set_snapshot(None);
    assert_eq!(canvas::frame_generation(&h.model, Seat::Tab), 0);
    assert!(canvas::frame(&h.model, Seat::Tab).is_none());

    // Another console in the same tab never repeats a generation, so the
    // canvas cannot mistake it for the one before.
    let target = tab(&h).target.clone();
    super::open_synthetic(&mut h.model, target);
    assert_eq!(canvas::frame_generation(&h.model, Seat::Tab), 0);
    set_snapshot(
        &mut h.model,
        Seat::Tab,
        pixel([1, 2, 3, 255], ConsoleState::Connected),
    );
    let three = canvas::frame_generation(&h.model, Seat::Tab);
    assert!(three != one && three != two && three != 0);
    assert_eq!(
        canvas::frame_generation(&h.model, Seat::Window(ConsoleWindowId(1))),
        0
    );
}

#[test]
fn the_canvas_input_state_follows_the_session_and_focus_loss_releases() {
    let mut h = Harness::scene("console-frame");
    let mut input = CanvasInput::new();
    input.set_bounds(800.0, 600.0);
    assert_eq!(input.bounds(), [800.0, 600.0]);
    press(&mut h, Seat::Tab, &mut input, PointerButton::Left, true);
    press(&mut h, Seat::Tab, &mut input, PointerButton::Right, true);
    assert_eq!(input.buttons(), 5, "a right-button drag is held natively");
    assert_eq!(input.pointer(), [5.0, 5.0]);
    press(&mut h, Seat::Tab, &mut input, PointerButton::Left, false);
    assert_eq!(input.buttons(), 4);

    // Focus lost: nothing is held, and the session released in the guest.
    let epoch = tab(&h).input_epoch();
    canvas::focus_lost(&mut h.model, Seat::Tab, &mut input);
    assert_eq!(input.buttons(), 0);
    assert!(input.held_keys().is_empty());
    assert_ne!(tab(&h).input_epoch(), epoch);

    // A control releases the guest's input: the window forgets what it held
    // at its next event.
    press(&mut h, Seat::Tab, &mut input, PointerButton::Middle, true);
    assert_eq!(input.buttons(), 2);
    h.click(commands::RELEASE_INPUT.0);
    canvas::pointer_move(&mut h.model, Seat::Tab, &mut input, 2.0, 2.0);
    assert_eq!(input.buttons(), 0);

    // The wheel is the console's, except Shift with the wheel in Actual
    // size, which pans the view.
    let turn = |h: &mut Harness, input: &mut CanvasInput, shift| {
        canvas::wheel(
            &mut h.model,
            Seat::Tab,
            input,
            [10.0, 10.0],
            [0.0, 40.0],
            false,
            shift,
        )
    };
    assert!(turn(&mut h, &mut input, true));
    tab_mut(&mut h).actual_size = true;
    assert!(!turn(&mut h, &mut input, true));
    assert!(turn(&mut h, &mut input, false));

    // Without a stream a key is not the console's (Rune: `console.is_none()`).
    assert!(!canvas::key(
        &mut h.model,
        Seat::Tab,
        &mut input,
        30,
        true,
        false,
        canvas::Modifiers::default()
    ));
    assert!(!canvas::keysym_key(
        &mut h.model,
        Seat::Tab,
        &mut input,
        0x61,
        true
    ));
    assert!(input.held_keys().is_empty());

    // Text that arrives as text is reviewed, never typed straight away.
    canvas::text(&mut h.model, Seat::Tab, &mut input, "uname -a\n");
    assert!(h.find(view::TYPING_REVIEW).is_some());
    assert!(!h.has_text("uname"));

    // No console at the seat: every call is a no-op.
    let nowhere = Seat::Window(ConsoleWindowId(5));
    canvas::pointer_move(&mut h.model, nowhere, &mut input, 1.0, 1.0);
    canvas::focus_lost(&mut h.model, nowhere, &mut input);
    assert!(!canvas::wheel(
        &mut h.model,
        nowhere,
        &mut input,
        [0.0, 0.0],
        [0.0, 40.0],
        false,
        false
    ));
    assert!(canvas::geometry(&h.model, nowhere, [10.0, 10.0]).is_none());
}

#[test]
fn a_seat_is_named_in_the_canvas_parameters() {
    for seat in [
        Seat::Tab,
        Seat::Window(ConsoleWindowId(1)),
        Seat::Window(ConsoleWindowId(8)),
    ] {
        assert_eq!(Seat::parse(&seat.param()), Some(seat));
    }
    assert_eq!(Seat::Tab.param(), "tab");
    assert_eq!(Seat::Window(ConsoleWindowId(3)).param(), "window-3");
    for bad in ["", "window-0", "window-9", "window-x", "main"] {
        assert_eq!(Seat::parse(bad), None, "{bad}");
    }
}

#[test]
fn the_window_scenes_describe_windows_with_their_own_controls() {
    let mut h = Harness::scene("console-window");
    let id = ConsoleWindowId(1);
    let requests = open_requests(&mut h);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].id, id);
    assert_eq!(Some(requests[0].lease), windows::lease(&h.model, id));
    assert!(super::tab_target(&h.model).is_none());
    assert!(h.find(view::CLOSED).is_some(), "the tab shows no console");
    let window = view::window(&h.model, id);
    assert_eq!(window.lease, windows::lease(&h.model, id));
    assert!(window.title.ends_with(" · XenAdmin RS console"));
    let nodes = window_nodes(&h, id);
    assert_eq!(duplicate_keys(&nodes), Vec::<String>::new());
    for key in [
        "close-console",
        view::STATUS,
        view::AUTO_RECONNECT,
        "reconnect-console",
        "console-actual-size",
        "console-fullscreen",
        "console-release-input",
        "save-console-screenshot",
        "console-ctrl-alt-delete",
        "console-ctrl-alt-f1",
        "console-alt-tab",
        "send-console-clipboard",
        "type-console-clipboard",
        "copy-console-clipboard",
        view::CANVAS,
        view::HELP,
    ] {
        assert!(keyed(&nodes, key).is_some(), "{key}");
    }
    for key in ["detach-console", "console-ctrl-c", "console-ctrl-d"] {
        assert!(keyed(&nodes, key).is_none(), "{key}");
    }
    let label = |nodes: &[Node], key: &str| keyed(nodes, key).and_then(|node| node.label.clone());
    assert_eq!(
        keyed(&nodes, view::CANVAS).and_then(|node| node.value.clone()),
        Some(r#"{"session":"window-1"}"#.into())
    );
    assert_eq!(
        label(&nodes, "console-actual-size"),
        Some("Actual size".into())
    );
    assert_eq!(
        label(&nodes, "send-console-clipboard"),
        Some("Send clipboard to VM".into())
    );
    assert_eq!(
        canvas::desktop_size(&h.model, Seat::Window(id)),
        Some([640, 360])
    );
    commands::run(&mut h.model, Seat::Window(id), Control::ActualSize);
    assert_eq!(
        label(&window_nodes(&h, id), "console-actual-size"),
        Some("Fit to panel".into())
    );
    h.take_intents();
    commands::run(&mut h.model, Seat::Window(id), Control::Fullscreen);
    assert_eq!(
        format!("{:?}", h.take_intents()),
        format!("[ToggleFullscreen(Console({id:?}))]")
    );

    let mut h = Harness::scene("multiple-consoles");
    assert_eq!(open_requests(&mut h).len(), 2);
    let open = windows::open_windows(&h.model);
    assert_eq!(open.len(), 2);
    assert_ne!(open[0].target.vm, open[1].target.vm);
    let tint = |id| canvas::frame(&h.model, Seat::Window(id)).unwrap().rgba[0];
    assert_eq!((tint(open[0].id), tint(open[1].id)), (40, 90));
    assert!(h.menu_item(commands::WINDOWS[0].0).is_some());
    assert!(h.menu_item(commands::WINDOWS[1].0).is_some());
    assert!(h.menu_item(commands::WINDOWS[2].0).is_none());
    assert!(
        h.menu_item(commands::CLOSE_ALL_WINDOWS.0)
            .is_some_and(|item| item.disabled.is_none())
    );
}
