//! Area `console`: the console of a VM and the text console of a server, in
//! the Console tab and in up to eight detached windows. Plan package P3.3.
//!
//! Rune modules: `console_view.rs`, `console_recovery.rs`,
//! `console_recovery_tests.rs`, `console_export.rs`, `host_console.rs`,
//! `console_windows.rs`. Tabs: Console.
//!
//! This is the model half. The pixel canvas, its keyboard and pointer
//! handlers and the detached windows themselves are bespoke GPUI code,
//! written from [`canvas`] (the seam) and
//! `/home/brendan/src/xenadmin-rs-wt/notes/progress/console-seam.md`.
//!
//! # What other areas and the front end call
//!
//! | Function | For |
//! | --- | --- |
//! | command `vm.console` / `server.console` | selecting the object, showing its Console tab and opening its console (cross-pool's "Open Destination Console") |
//! | [`open`] | the same without touching the selection |
//! | [`close_tab`] | ending the tab's console |
//! | [`tab_target`] | which console the tab shows |
//! | [`session`], [`Seat`] | reading a session (the front end) |
//! | [`canvas`]: everything `pub` | the `console-canvas` element |
//! | [`view::window`], [`windows::closed`], [`windows::close_all`], [`windows::WindowPayload`] | the detached windows |
//! | `Intent::ToggleFullscreen` | fullscreen: the area asks, the front end toggles the window |
//!
//! # Where the Rune console went
//!
//! In Rune "Open console" replaced the inventory with the console of a
//! pinned guest, and "Back to inventory" ended it. The console is the
//! Console tab of the object now:
//!
//! | Rune | Now |
//! | --- | --- |
//! | "Open console" in the VM details | selecting the Console tab of the VM (the hook `on_tab_change`), when the VM is eligible; also command `vm.console`: the tab's local row and the VM's context menu |
//! | "Open host text console" in the host details | the same for a server; command `server.console` |
//! | "Back to inventory" (`close-console`) | command `console.close`; selecting another object also ends the tab's console (`on_scope_change`) |
//! | the toolbar's buttons | commands on the tab's local row, see [`commands`] |
//! | checkbox "Reconnect automatically" | the same checkbox, in the tab body |
//! | the typing review, the status lines, the canvas, the help caption | blocks of the tab body, see [`view`] |
//! | "Open in separate window" | command `console.undock` |
//! | "Console windows (n)" and its manager dialog | the Window menu: one entry per open window (`window.console-1` to `-8`) and `window.console.close-all` |
//! | a detached window's own toolbar | the same controls as buttons, see [`view::window`] |
//!
//! No surface: nothing here is a dialog, so nothing carries the
//! experimental notice (Rune showed none for the console either; that
//! clipboard exchange is experimental is said in the tab's last caption).
//!
//! # Commands
//!
//! | Id | Label | Where |
//! | --- | --- | --- |
//! | `vm.console` | Open Console | Console tab row; context menu of a VM |
//! | `server.console` | Open Host Text Console | Console tab row; context menu of a server |
//! | `console.reconnect` | Reconnect | Console tab row |
//! | `console.actual-size` | Actual Size (checked) | Console tab row |
//! | `console.fullscreen` | Fullscreen | Console tab row |
//! | `console.undock` | Open in Separate Window | Console tab row |
//! | `console.release-input` | Release Input | Console tab row |
//! | `console.save-screenshot` | Save Screenshot… | Console tab row |
//! | `vm.send-ctrl-alt-del` | Send Ctrl+Alt+Del | VM menu; Console tab row (guest) |
//! | `console.send-ctrl-alt-f1`, `console.send-alt-tab` | Ctrl+Alt+F1, Alt+Tab | Console tab row (guest) |
//! | `console.send-ctrl-c`, `console.send-ctrl-d` | Ctrl+C, Ctrl+D | Console tab row (server) |
//! | `console.send-clipboard` | Send Clipboard to VM / Host | Console tab row |
//! | `console.type-clipboard` | Type Clipboard as Keys… | Console tab row |
//! | `console.copy-remote-clipboard` | Copy VM / Host Clipboard | Console tab row |
//! | `console.close` | Close Console | Console tab row |
//! | `window.console-1` … `window.console-8` | the console's heading | Window menu, one per open window |
//! | `window.console.close-all` | Close All Consoles | Window menu |
//!
//! # Scenes
//!
//! `console-recovery`, `console-frame`, `console-actual-size`,
//! `console-typing-review`, `console-details`, `host-console`,
//! `multiple-consoles`, `console-window`. See [`scenes`].
//!
//! # What has no successor
//!
//! - "Hold right button": decision 0006, a right-button drag is native.
//! - The drawn guest cursor and the hidden local cursor: decision 0006.
//! - The console windows manager dialog (`console-windows`,
//!   `show-console-window-N`, `close-console-window-N`,
//!   `console-windows-close`): the Window menu lists the windows; a window
//!   is closed from itself or with Close All Consoles.
//! - `console_view::host`, by which Rune treated the host as selected while
//!   its console was open: the host IS the selection now.
//! - `console_view::release` as a call other modules made before opening
//!   their dialogs: the canvas reports that it lost the keyboard
//!   ([`canvas::focus_lost`]).
//! - "Cannot open the save dialog: {error}": the file prompt's reply does
//!   not distinguish a failed prompt from a cancel.
//! - The keysym the toolkit derived from the host's keyboard layout: keys
//!   arrive as evdev codes (decision 0006). See [`canvas::us_keysym`].
//! - The pinned console that survived any selection: the tab's console
//!   belongs to the selected object. A console that must outlive the
//!   selection is detached into a window.
//!
//! # The files, in the order to read them
//!
//! - [`session`]: one console session, its lifecycle and its poll.
//! - [`recovery`]: the reconnect state machine (pure).
//! - [`export`]: the screenshot.
//! - [`canvas`]: the seam with the canvas element; input mapping.
//! - [`windows`]: the eight slots of the detached windows.
//! - [`commands`]: the controls and the command table.
//! - [`view`]: the tab body and a window's content.
//! - [`scenes`], `tests`.

use xenadmin_core::{Error, ResourceTarget};

use crate::prelude::*;

pub mod canvas;
pub mod commands;
pub mod export;
pub mod recovery;
pub mod scenes;
pub mod session;
pub mod view;
pub mod windows;

#[cfg(test)]
mod tests;

pub use session::{ConsoleTarget, Seat, Session};

/// The state of the area. `m.areas.console`.
#[derive(Default)]
pub struct State {
    /// The console the Console tab shows. It belongs to the object that is
    /// the tree scope.
    pub(crate) tab: Option<Session>,
    pub(crate) windows: windows::Registry,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        tab: view::tab,
        poll,
        on_scope_change: scope_changed,
        on_tab_change: tab_changed,
        on_session_change: close_tab,
        ..AreaSpec::new("console")
    }
}

/// The session shown at `seat`, if there is one.
pub fn session(m: &Model, seat: Seat) -> Option<&Session> {
    match seat {
        Seat::Tab => m.areas.console.tab.as_ref(),
        Seat::Window(id) => windows::session(m, id),
    }
}

pub(crate) fn session_mut(m: &mut Model, seat: Seat) -> Option<&mut Session> {
    match seat {
        Seat::Tab => m.areas.console.tab.as_mut(),
        Seat::Window(id) => windows::session_mut(m, id),
    }
}

/// What the tab's console is pinned to.
pub fn tab_target(m: &Model) -> Option<&ConsoleTarget> {
    m.areas.console.tab.as_ref().map(Session::target)
}

/// Ends the tab's console: its input is released in the guest, the stream
/// is closed and the session is dropped. The detached windows are not
/// touched: each holds its own clone of its connection, and stays when the
/// main window shows another pool (the Rune behaviour).
pub fn close_tab(m: &mut Model) {
    if let Some(mut session) = m.areas.console.tab.take() {
        session.close();
    }
}

/// The tree scope changed: the tab's console belonged to the object that
/// was selected. When the Console tab is the one shown for the new object
/// too, its console is opened, as it is when the tab is chosen.
fn scope_changed(m: &mut Model) {
    let stays = m
        .areas
        .console
        .tab
        .as_ref()
        .is_some_and(|s| m.scope().single() == Some(&s.target.object()));
    if !stays {
        close_tab(m);
    }
    show_scope_console(m);
}

/// The active tab changed. Leaving the Console tab lets go of what the
/// console holds down in its guest; the console itself stays until the
/// scope changes or it is closed, so coming back does not reconnect.
/// Choosing the Console tab opens the console of the scope object: what
/// pressing the Rune "Open console" button did.
fn tab_changed(m: &mut Model, from: TabId, to: TabId) {
    if from == TabId::Console
        && let Some(s) = &mut m.areas.console.tab
    {
        s.release();
    }
    if to == TabId::Console {
        show_scope_console(m);
    }
}

/// Opens the console of the scope object when the Console tab is shown and
/// the object is eligible: the command `vm.console` or `server.console`
/// would be enabled for it (one guest or one server, a current connection
/// with write access, a console that is not the one shown already). An
/// object that is not eligible opens nothing and says nothing: the tab's
/// row has the command with the reason. A failed open is on the shared
/// error line, as the command's is.
fn show_scope_console(m: &mut Model) {
    if m.active_tab() != TabId::Console {
        return;
    }
    let scope = m.scope().clone();
    let Some(object) = scope.single() else {
        return;
    };
    let command = match object.kind {
        ObjectKind::Vm => commands::OPEN,
        ObjectKind::Host => commands::OPEN_HOST,
        _ => return,
    };
    if m.eligibility(command, &scope) == Eligibility::Enabled {
        m.error = open(m, object).err().map(|error| error.to_string());
    }
}

/// A failed open or reconnect: the tab's goes to the shared error line (and
/// a success clears that line, as in Rune); a window's is shown in it.
pub(crate) fn set_error(m: &mut Model, seat: Seat, error: Option<String>) {
    match seat {
        Seat::Tab => m.error = error,
        Seat::Window(_) => {
            if let Some(s) = session_mut(m, seat) {
                s.error = error;
            }
        }
    }
}

/// Opens the console of a VM, or the text console of a server, as the
/// tab's console (the Rune `State::open_console` and `host_console::open`).
/// A console the tab showed before is ended first. The selection is not
/// touched: the caller makes the object the scope (the commands do).
pub fn open(m: &mut Model, object: &ObjectRef) -> Result<(), Error> {
    let pool = object.pool.ok_or(Error::NoConsole)?;
    let snapshot = m.snapshot_for(pool).ok_or(Error::Stale)?;
    let (target, connection, console) = match object.kind {
        ObjectKind::Vm => {
            let vm = object.vm_id().ok_or(Error::NoConsole)?;
            let observed = snapshot.observe_vm(vm)?;
            let inventory = snapshot.inventory().ok_or(Error::Stale)?;
            let name = inventory.vms.get(&vm).ok_or(Error::Stale)?.name.clone();
            let target = ConsoleTarget {
                pool: inventory.pool_id,
                vm,
                name,
                host: None,
            };
            let connection = m.connection_for(pool).ok_or(Error::Closed)?;
            let console = connection.open_console(observed)?;
            (target, connection, console)
        }
        ObjectKind::Host => {
            let host = object.host_id().ok_or(Error::NoConsole)?;
            let observed = snapshot.observe_resource(ResourceTarget::Host(host))?;
            let inventory = snapshot.inventory().ok_or(Error::Stale)?;
            let vm = commands::control_domain(inventory, host).ok_or(Error::NoConsole)?;
            let name = inventory
                .hosts
                .get(&host)
                .ok_or(Error::Stale)?
                .name
                .clone();
            let target = ConsoleTarget {
                pool: inventory.pool_id,
                vm,
                name,
                host: Some(host),
            };
            let connection = m.connection_for(pool).ok_or(Error::Closed)?;
            let console = connection.open_host_console(observed, vm)?;
            (target, connection, console)
        }
        _ => return Err(Error::NoConsole),
    };
    close_tab(m);
    let mut session = Session::opened(target, m, Some(connection), false);
    session.attach(console);
    m.areas.console.tab = Some(session);
    Ok(())
}

/// The tab's console in a synthetic pool: no connection and no stream. What
/// the Rune scenes did by calling `opened` without a core handle.
pub(crate) fn open_synthetic(m: &mut Model, target: ConsoleTarget) {
    close_tab(m);
    let session = Session::opened(target, m, None, true);
    m.areas.console.tab = Some(session);
}

fn earlier(wake: &mut Option<f64>, seconds: f64) {
    *wake = Some(wake.map_or(seconds, |wake| wake.min(seconds)));
}

/// Every wake and tick: each session reads its writer, its connection and
/// its stream, and recovery advances on the poll clock.
///
/// Returns `true` only when something the blocks or the controls show
/// changed. A new frame alone does not: the canvas compares
/// [`canvas::frame_generation`] after every poll, so frame traffic does not
/// re-render the shell.
fn poll(m: &mut Model, clock: f64) -> bool {
    let mut changed = false;
    let mut wake = None;
    let mut tab_error = None;
    let state = &mut m.areas.console;
    if let Some(s) = &mut state.tab {
        let polled = s.poll(clock);
        changed |= polled.changed;
        if let Some(seconds) = polled.wake {
            earlier(&mut wake, seconds);
        }
        tab_error = polled.reopened.map(Result::err);
    }
    for slot in &mut state.windows.slots {
        let windows::Slot::Open { session, .. } = slot else {
            continue;
        };
        let polled = session.poll(clock);
        changed |= polled.changed;
        if let Some(seconds) = polled.wake {
            earlier(&mut wake, seconds);
        }
        if let Some(result) = polled.reopened {
            session.error = result.err();
        }
        // A window's connection may be another than the scope session's,
        // whose changes wake the model; bounded polling, as in Rune.
        if session.connection.is_some() {
            earlier(&mut wake, 0.25);
        }
    }
    if let Some(error) = tab_error {
        m.error = error;
    }
    if let Some(seconds) = wake {
        m.request_tick(seconds);
    }
    changed
}
