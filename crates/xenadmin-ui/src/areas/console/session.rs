//! One console session: everything one console holds, whether it is shown
//! in the Console tab or in a detached window.
//!
//! This is the Rune `ConsoleUi` together with the fields that lay beside it
//! in the Rune `State` (`console`, `console_snapshot`) and the two things a
//! detached window's own `State` held for it: a clone of the connection and
//! that connection's latest snapshot. Because a session carries those two
//! itself, it does not care which pool the tree shows: a detached window
//! keeps working while the main window browses another pool, as in Rune.
//!
//! What is NOT here is the input state of the window that shows the
//! session (held keys, button mask, wheel remainders, canvas bounds). That
//! is [`super::canvas::CanvasInput`], owned by the view. The session tells
//! the view when to forget what it holds through [`Session::input_epoch`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use xenadmin_core::{
    Connection, ConnectionSnapshot, Console, ConsoleInput, ConsoleSnapshot, ConsoleState, Error,
    HostId, PoolId, ResourceTarget, TypedText, VmId,
};

use super::export::Export;
use super::recovery::{Binding, Recovery, Waiting, inventory_binding};
use crate::model::Waker;
use crate::prelude::*;

/// What a console is pinned to. `host: Some(h)` means the text console of
/// host `h`; `vm` is then that host's control domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleTarget {
    pub pool: PoolId,
    pub vm: VmId,
    pub name: String,
    pub host: Option<HostId>,
}

impl ConsoleTarget {
    /// The object in the tree whose Console tab shows this console.
    pub fn object(&self) -> ObjectRef {
        match self.host {
            Some(host) => ObjectRef::host(self.pool, host),
            None => ObjectRef::vm(self.pool, self.vm),
        }
    }

    /// The heading of the console (the Rune heading).
    pub fn heading(&self) -> String {
        format!(
            "{} · {}",
            if self.host.is_some() {
                "Host text console"
            } else {
                "Console"
            },
            self.name
        )
    }

    /// The title of a detached window showing this console.
    pub fn window_title(&self) -> String {
        format!("{} · XenAdmin RS console", self.name)
    }

    /// "host" or "VM", as the clipboard buttons name the other side.
    pub fn side(&self) -> &'static str {
        if self.host.is_some() { "host" } else { "VM" }
    }
}

/// Where a session is shown: the Console tab of the main window, or one of
/// the eight detached windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Seat {
    Tab,
    Window(ConsoleWindowId),
}

impl Seat {
    /// The value of the `session` parameter of the `console-canvas` element:
    /// `"tab"` or `"window-3"`.
    pub fn param(self) -> String {
        match self {
            Self::Tab => "tab".into(),
            Self::Window(id) => format!("window-{}", id.0),
        }
    }

    /// The seat a `session` parameter names.
    pub fn parse(param: &str) -> Option<Self> {
        if param == "tab" {
            return Some(Self::Tab);
        }
        let slot: u32 = param.strip_prefix("window-")?.parse().ok()?;
        (1..=super::windows::LIMIT as u32)
            .contains(&slot)
            .then_some(Self::Window(ConsoleWindowId(slot)))
    }
}

/// Clipboard text that waits for the person to confirm that it is typed as keys. It
/// belongs to the console that was open when it was read, and it is never displayed.
pub(super) struct PendingText {
    pub(super) serial: u64,
    pub(super) text: zeroize::Zeroizing<String>,
    pub(super) review: TypedText,
}

pub struct Session {
    pub(super) target: ConsoleTarget,
    pub(super) recovery: Recovery,
    /// The identity of "this open stream" for asynchronous replies: a
    /// clipboard read, a save prompt, a pending typed text. Process-wide
    /// unique; it changes on open, reconnect, detach and a recovery close.
    pub(super) serial: u64,
    pub(super) actual_size: bool,
    /// One status line under the controls.
    pub(super) message: Option<String>,
    /// A failed open or reconnect of a detached window's console. The tab's
    /// goes to the shared error line, as in Rune.
    pub(super) error: Option<String>,
    pub(super) export: Export,
    pub(super) typed: Option<PendingText>,
    /// The session's own clone of the connection. `None` in a synthetic
    /// pool (the scenes).
    pub(super) connection: Option<Connection>,
    /// That connection's latest snapshot, refreshed on every poll.
    pub(super) pool: Option<Arc<ConnectionSnapshot>>,
    /// Dropping the handle cancels the stream.
    pub(super) console: Option<Console>,
    pub(super) snapshot: Option<Arc<ConsoleSnapshot>>,
    /// See [`super::canvas::frame_generation`].
    pub(super) frame_generation: u64,
    /// Counts [`Session::release`]: with `serial`, the input epoch.
    pub(super) releases: u64,
    /// The pool is synthetic: there is no stream and there never will be.
    pub(super) demo: bool,
    pub(super) waker: Option<Waker>,
}

fn next_serial() -> u64 {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    SERIAL.fetch_add(1, Ordering::Relaxed)
}

fn next_generation() -> u64 {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// What one poll of a session did.
#[derive(Default)]
pub(super) struct Polled {
    /// Something the blocks show changed. A new frame alone is not that.
    pub(super) changed: bool,
    /// Poll again after this many seconds.
    pub(super) wake: Option<f64>,
    /// Recovery reopened the stream, and how that went.
    pub(super) reopened: Option<Result<(), String>>,
}

/// Everything the blocks and the eligibility of the controls are built
/// from, so a poll can say whether any of it changed.
#[derive(PartialEq)]
struct Shown {
    state: String,
    recovery: Recovery,
    message: Option<String>,
    error: Option<String>,
    typing: usize,
    clipboard: bool,
    frame: Option<[u16; 2]>,
    ready: bool,
    exporting: bool,
    reviewing: bool,
}

impl Session {
    /// A session for a newly opened target (the Rune `opened` and
    /// `opened_host`). The console defaults are read here and only here: a
    /// later change of the options does not touch an open console.
    pub(super) fn opened(
        target: ConsoleTarget,
        m: &Model,
        connection: Option<Connection>,
        demo: bool,
    ) -> Self {
        let options = &m.preferences.options;
        let mut session = Self {
            target,
            recovery: Recovery::with_enabled(options.console_reconnect),
            serial: next_serial(),
            actual_size: options.console_actual_size,
            message: None,
            error: None,
            export: Export::default(),
            typed: None,
            pool: connection.as_ref().map(Connection::latest),
            connection,
            console: None,
            snapshot: None,
            frame_generation: 0,
            releases: 0,
            demo,
            waker: m.waker.clone(),
        };
        session.recovery.binding = session.binding().ok();
        session
    }

    pub fn target(&self) -> &ConsoleTarget {
        &self.target
    }

    /// Fit (false) or Actual size (true).
    pub fn actual_size(&self) -> bool {
        self.actual_size
    }

    /// The latest snapshot of the stream.
    pub fn snapshot(&self) -> Option<&Arc<ConsoleSnapshot>> {
        self.snapshot.as_ref()
    }

    /// Changes whenever the view must forget the keys and buttons it holds:
    /// the session released them in the guest, or it is another stream now.
    pub fn input_epoch(&self) -> (u64, u64) {
        (self.serial, self.releases)
    }

    /// Takes a newly opened stream.
    pub(super) fn attach(&mut self, console: Console) {
        if let Some(waker) = &self.waker {
            console.notify_on_change(waker.clone());
        }
        self.console = Some(console);
        self.set_snapshot(None);
    }

    /// Stores a snapshot of the stream. A frame that is another frame than
    /// the one before (or no frame at all) gets a new generation, so the
    /// canvas uploads exactly when the picture changed and never shows the
    /// picture of a stream that is gone.
    pub(super) fn set_snapshot(&mut self, snapshot: Option<Arc<ConsoleSnapshot>>) {
        let old = self.snapshot.as_ref().and_then(|s| s.frame.as_ref());
        let new = snapshot.as_ref().and_then(|s| s.frame.as_ref());
        let same = match (old, new) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.frame_generation = if new.is_some() { next_generation() } else { 0 };
        }
        self.snapshot = snapshot;
    }

    /// What the inventory says about the pinned target (the Rune `binding`).
    pub(super) fn binding(&self) -> Result<Binding, Waiting> {
        let connection = self.connection.as_ref().ok_or(Waiting::Connection)?;
        if connection.is_read_only() {
            return Err(Waiting::ReadOnly);
        }
        let snapshot = self.pool.as_ref().ok_or(Waiting::Connection)?;
        let inventory = snapshot
            .inventory()
            .filter(|i| snapshot.is_current() && i.pool_id == self.target.pool)
            .ok_or(Waiting::Connection)?;
        inventory_binding(inventory, &self.target, snapshot.generation())
    }

    /// Input reaches the guest only while this holds. It gates every
    /// [`Session::send`] and every control that sends something.
    pub fn input_ready(&self) -> bool {
        !self.recovery.needs_reopen
            && self.recovery.waiting.is_none()
            && matches!(
                self.snapshot.as_ref().map(|s| &s.state),
                Some(ConsoleState::Connected)
            )
            && self.binding().is_ok_and(|current| {
                self.recovery
                    .binding
                    .as_ref()
                    .is_some_and(|opened| opened.same_stream(&current))
            })
    }

    /// Why input does not reach the guest: what the status lines say. Rune
    /// disabled the controls without a reason; a disabled control carries
    /// one here, and it is the line the console already shows.
    pub fn not_ready_reason(&self) -> Option<String> {
        if self.input_ready() {
            return None;
        }
        if let Err(waiting) = self.binding() {
            return Some(waiting.message().into());
        }
        if let Some(status) = self.recovery.status() {
            return Some(status);
        }
        if matches!(
            self.snapshot.as_ref().map(|s| &s.state),
            Some(ConsoleState::Connected)
        ) {
            return Some(NOT_CONNECTED.into());
        }
        Some(self.state_line())
    }

    /// The first status line: the state of the stream.
    pub fn state_line(&self) -> String {
        match self.snapshot.as_ref().map(|x| &x.state) {
            Some(ConsoleState::Connected) => "Connected directly to the host console".into(),
            Some(ConsoleState::Failed(error)) => {
                format!(
                    "Console disconnected: {error}. Reconnect when the guest or host is available."
                )
            }
            Some(ConsoleState::Closed) => {
                "Console closed. Reconnect when the guest or host is available.".into()
            }
            _ => "Opening the host console…".into(),
        }
    }

    /// Characters of a typed text that are still to be sent.
    pub fn typing_left(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |s| s.typing)
    }

    /// The review that is on screen: a pending text of this very stream.
    pub(super) fn review(&self) -> Option<TypedText> {
        self.typed
            .as_ref()
            .filter(|pending| pending.serial == self.serial)
            .map(|pending| pending.review)
    }

    fn shown(&self) -> Shown {
        Shown {
            state: self.state_line(),
            recovery: self.recovery.clone(),
            message: self.message.clone(),
            error: self.error.clone(),
            typing: self.typing_left(),
            clipboard: self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.guest_clipboard.is_some()),
            frame: self.desktop_size(),
            ready: self.input_ready(),
            exporting: self.export.pending,
            reviewing: self.review().is_some(),
        }
    }

    /// The guest's desktop size: that of the latest frame.
    pub fn desktop_size(&self) -> Option<[u16; 2]> {
        let frame = self.snapshot.as_ref()?.frame.as_ref()?;
        Some([frame.width, frame.height])
    }

    /// Releases every key and button in the guest and tells the view to
    /// forget what it holds (the Rune `release`).
    pub(super) fn release(&mut self) {
        if let Some(console) = &self.console {
            let _ = console.input(ConsoleInput::ReleaseAll);
        }
        self.releases += 1;
    }

    /// Sends input when the console is ready for it; a refusal becomes the
    /// status line (the Rune `send`).
    pub(super) fn send(&mut self, input: ConsoleInput) {
        if !self.input_ready() {
            return;
        }
        if let Some(console) = &self.console
            && let Err(error) = console.input(input)
        {
            self.message = Some(error.to_string());
        }
    }

    /// The session is about to show another stream of the same target.
    pub(super) fn reconnecting(&mut self) {
        self.release();
        self.serial = next_serial();
        self.message = None;
        self.export = Export::default();
        self.recovery.binding = self.binding().ok();
    }

    /// The session moves to a detached window: input is released in the
    /// guest, replies still on their way are void, and so is a save prompt
    /// that is still open; a writer that already started may finish.
    pub(super) fn moving_window(&mut self) {
        self.release();
        self.serial = next_serial();
        if self.export.pending && !self.export.writing() {
            self.export = Export::default();
        }
        self.error = None;
    }

    /// Opens a new stream for the pinned target (the Rune
    /// `State::reconnect_console`). On an error the old, closed handle
    /// stays.
    pub(super) fn reconnect(&mut self) -> Result<(), Error> {
        if let Some(connection) = &self.connection {
            self.pool = Some(connection.latest());
        }
        let snapshot = self.pool.clone().ok_or(Error::Stale)?;
        if snapshot
            .inventory()
            .is_none_or(|inventory| inventory.pool_id != self.target.pool)
        {
            return Err(Error::Stale);
        }
        let connection = self.connection.clone().ok_or(Error::Closed)?;
        let console = match self.target.host {
            Some(host) => connection.open_host_console(
                snapshot.observe_resource(ResourceTarget::Host(host))?,
                self.target.vm,
            )?,
            None => connection.open_console(snapshot.observe_vm(self.target.vm)?)?,
        };
        self.reconnecting();
        self.attach(console);
        Ok(())
    }

    /// The Reconnect control: the retry budget starts over, the choice of
    /// automatic reconnect stays.
    pub(super) fn manual_reconnect(&mut self) -> Result<(), Error> {
        self.recovery = Recovery::with_enabled(self.recovery.enabled);
        self.reconnect()
    }

    /// Reviews clipboard text for typing as keys and asks for confirmation, or says why it
    /// cannot be typed.
    pub(super) fn arm_typing(&mut self, text: &str) {
        match TypedText::review(text) {
            Ok(review) => {
                self.typed = Some(PendingText {
                    serial: self.serial,
                    text: zeroize::Zeroizing::new(text.to_owned()),
                    review,
                });
                self.message = None;
            }
            Err(error) => {
                self.typed = None;
                self.message = Some(error.to_string());
            }
        }
    }

    pub(super) fn confirm_typing(&mut self) {
        let Some(pending) = self.typed.take() else {
            return;
        };
        if pending.serial != self.serial {
            return;
        }
        self.release();
        if !self.input_ready() {
            self.message = Some("The console is not connected; nothing was typed.".into());
            return;
        }
        self.message = Some(
            "Typing. A key press, Release input or leaving the console stops what is left.".into(),
        );
        self.send(ConsoleInput::TypeText(pending.text.to_string()));
    }

    /// The console is being closed for good: input is released first, then
    /// the stream is closed. Dropping the session afterwards cancels it.
    pub(super) fn close(&mut self) {
        self.release();
        if let Some(console) = &self.console {
            console.close();
        }
    }

    /// One poll: the screenshot writer, the connection's snapshot, the
    /// stream's snapshot, then recovery (the order of the Rune polls).
    pub(super) fn poll(&mut self, clock: f64) -> Polled {
        let before = self.shown();
        let mut polled = Polled::default();
        if let Some(message) = self.export.poll() {
            self.message = Some(message);
        }
        if let Some(connection) = &self.connection {
            let latest = connection.latest();
            if self
                .pool
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, &latest))
            {
                self.pool = Some(latest);
            }
        }
        if let Some(console) = &self.console {
            let latest = console.latest();
            if self
                .snapshot
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(old, &latest))
            {
                self.set_snapshot(Some(latest));
            }
        }
        let mut acted = false;
        if self.console.is_some() {
            let observation = self.binding();
            let phase = self
                .snapshot
                .as_ref()
                .map(|s| s.state.clone())
                .unwrap_or(ConsoleState::Connecting);
            let decision = self.recovery.advance(observation, &phase, clock);
            if decision.close {
                self.release();
                if let Some(console) = &self.console {
                    console.close();
                }
                self.serial = next_serial();
                self.export = Export::default();
            }
            if decision.reopen {
                polled.reopened = Some(self.reconnect().map_err(|error| error.to_string()));
            }
            polled.wake = decision.wake.map(|seconds| seconds.max(0.001));
            acted = decision.close || decision.reopen;
        }
        polled.changed = acted || before != self.shown();
        polled
    }
}

/// The reason of a control that needs a connected console when the stream
/// says it is connected but is not the one the target has now.
pub const NOT_CONNECTED: &str = "The console is not connected.";
