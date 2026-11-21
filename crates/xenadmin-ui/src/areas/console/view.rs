//! What the console shows: the body of the Console tab, and the content of
//! a detached window.
//!
//! Both are the same blocks from one function ([`body`]). The tab and a
//! detached window offer the actions as commands: the tab on its local
//! action row, the window on its own row ([`super::commands::window_row`]).
//! A window's row runs them against that window. Fit, fullscreen, release,
//! a host console's own chords and the automatic reconnect stay controls
//! of the body.
//!
//! Keys. Rune keys kept: `console-canvas`, `console-auto-reconnect`,
//! `console-typing-review`, `confirm-console-typing`,
//! `cancel-console-typing`, `console-typing-progress`, and on a window's
//! buttons `close-console`, `reconnect-console`, `save-console-screenshot`,
//! `send-console-clipboard`, `type-console-clipboard`,
//! `copy-console-clipboard` (`detach-console` belongs to the tab, where it
//! is the command `console.undock`). New keys, for elements Rune left
//! unkeyed: `console-status`, `console-recovery-status`,
//! `console-typing-prompt`, `console-message`, `console-help`,
//! `console-error`, `console-closed`, and on a window's buttons
//! `console-actual-size`, `console-fullscreen`, `console-release-input`,
//! `console-ctrl-alt-delete`, `console-ctrl-alt-f1`, `console-alt-tab`,
//! `console-ctrl-c`, `console-ctrl-d`.
//!
//! The canvas is ONE custom element: name `console-canvas`, key
//! `console-canvas`, parameters `{"session": "tab"}` or
//! `{"session": "window-3"}` ([`Seat::param`]). It takes the height the
//! other blocks leave.

use xenadmin_core::TypedText;

use super::commands::{self, Control};
use super::session::{Seat, Session};
use super::{session_mut, windows};
use crate::prelude::*;

pub const CANVAS: &str = "console-canvas";
pub const STATUS: &str = "console-status";
pub const RECOVERY_STATUS: &str = "console-recovery-status";
pub const AUTO_RECONNECT: &str = "console-auto-reconnect";
pub const TYPING_REVIEW: &str = "console-typing-review";
pub const TYPING_PROMPT: &str = "console-typing-prompt";
pub const CONFIRM_TYPING: &str = "confirm-console-typing";
pub const CANCEL_TYPING: &str = "cancel-console-typing";
pub const TYPING_PROGRESS: &str = "console-typing-progress";
pub const MESSAGE: &str = "console-message";
pub const HELP: &str = "console-help";
pub const ERROR: &str = "console-error";
pub const CLOSED: &str = "console-closed";

/// The caption under the canvas, word for word from Rune.
pub const HELP_TEXT: &str = "Click the console to type. Focus loss releases input. In Actual size, Shift+wheel pans the view. Clipboard exchange is experimental: it needs guest tools that support it and has not worked with the Linux guests tried. Typing the clipboard as keys works with any guest, for up to 2,000 characters of plain US-keyboard text.";

/// The title of a detached window that shows no console.
pub const WINDOW_TITLE: &str = "XenAdmin RS console";

/// The Console tab of a VM or a server: its console when it is open,
/// otherwise one line; the local row then offers Open Console.
pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Console {
        return None;
    }
    let object = scope.single()?;
    let closed = match object.kind {
        ObjectKind::Vm => "No console is open for this VM.",
        ObjectKind::Host => "No text console is open for this server.",
        _ => return None,
    };
    let blocks = match m
        .areas
        .console
        .tab
        .as_ref()
        .filter(|s| s.target.object() == *object)
    {
        Some(s) => body(m, Seat::Tab, s),
        None => blocks![caption(closed).key(CLOSED)],
    };
    Some(TabBody::Blocks(blocks))
}

/// What a detached window shows.
pub struct WindowView {
    pub id: ConsoleWindowId,
    /// The lease of the console it shows; `None` when it shows none.
    pub lease: Option<u64>,
    /// "{name} · XenAdmin RS console".
    pub title: String,
    pub blocks: Vec<Block>,
}

/// The content of a detached window. A window that shows no console (its
/// console was closed and the window is still there) says so, as in Rune.
pub fn window(m: &Model, id: ConsoleWindowId) -> WindowView {
    match windows::session(m, id) {
        Some(s) => WindowView {
            id,
            lease: windows::lease(m, id),
            title: s.target.window_title(),
            blocks: body(m, Seat::Window(id), s),
        },
        None => WindowView {
            id,
            lease: None,
            title: WINDOW_TITLE.into(),
            blocks: blocks![
                heading("Console closed"),
                caption("Open a console from the main window.").key(CLOSED),
            ],
        },
    }
}

/// The question put before a text is typed. It gives the amount, never the text.
pub fn typing_prompt(review: TypedText, target: &str) -> String {
    let characters = match review.characters {
        1 => "1 character".to_owned(),
        n => format!("{n} characters"),
    };
    let breaks = match review.line_breaks {
        0 => String::new(),
        1 => ", with 1 line break sent as Enter,".to_owned(),
        n => format!(", with {n} line breaks sent as Enter,"),
    };
    format!(
        "Type {characters}{breaks} into {target} as key presses? The text is not shown here. Punctuation comes out as written only if the guest uses a US keyboard layout."
    )
}

fn control_button(m: &Model, seat: Seat, s: &Session, control: Control) -> Option<Button> {
    let reason = match commands::control_state(m, seat, control) {
        Eligibility::Hidden => return None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Enabled => None,
    };
    Some(
        button(
            control.key(),
            control.label(s.actual_size, s.target.side()),
            move |m| commands::run(m, seat, control),
        )
        .disabled_opt(reason),
    )
}

/// The review of a text waiting to be typed: how much there is, never the
/// text. Only for the stream the text was read for.
fn typing_review(seat: Seat, s: &Session) -> Option<Group> {
    let review = s.review()?;
    Some(
        group(
            "Type clipboard text as keys",
            blocks![
                caption(typing_prompt(review, &s.target.name)).key(TYPING_PROMPT),
                actions([button(CANCEL_TYPING, "Cancel", move |m| {
                    if let Some(s) = session_mut(m, seat) {
                        s.typed = None;
                    }
                })]),
            ],
        )
        .key(TYPING_REVIEW),
    )
}

/// The blocks of one console, top to bottom as in Rune: heading, the state
/// of the stream, what recovery says, the reconnect choice, (in a window)
/// the three rows of controls, the typing review, the status line, the
/// typing progress, the canvas, the help.
fn body(m: &Model, seat: Seat, s: &Session) -> Vec<Block> {
    let mut out = blocks![heading(s.target.heading())];
    if seat != Seat::Tab {
        out.extend(control_button(m, seat, s, Control::Close).map(Block::from));
    }
    out.extend(blocks![
        caption(s.state_line()).key(STATUS),
        s.recovery
            .status()
            .map(|status| caption(status).key(RECOVERY_STATUS)),
        checkbox(
            AUTO_RECONNECT,
            "Reconnect automatically",
            s.recovery.enabled,
            move |m| {
                if let Some(s) = session_mut(m, seat) {
                    s.recovery.enabled = !s.recovery.enabled;
                    s.release();
                }
            }
        ),
    ]);
    if seat != Seat::Tab {
        for row in Control::ROWS {
            let buttons: Vec<Button> = row
                .iter()
                .filter(|control| !commands::WINDOW_ROW.contains(control))
                .filter_map(|control| control_button(m, seat, s, *control))
                .collect();
            if !buttons.is_empty() {
                out.push(actions(buttons));
            }
        }
    }
    out.extend(blocks![
        typing_review(seat, s),
        s.message
            .as_ref()
            .map(|message| caption(message.clone()).key(MESSAGE)),
        (s.typing_left() > 0).then(|| {
            caption(format!(
                "Typing as keys: {} characters left.",
                s.typing_left()
            ))
            .key(TYPING_PROGRESS)
        }),
        custom(CANVAS, serde_json::json!({ "session": seat.param() })).key(CANVAS),
        caption(HELP_TEXT).key(HELP),
        s.error
            .as_ref()
            .map(|error| alert(error.clone()).key(ERROR)),
    ]);
    out
}
