//! Second `wl_keyboard` for spike S1, route A.
//!
//! GPUI 0.3.12 turns a Wayland key into a `Keystroke` and drops the evdev
//! code. Modifier keys are not forwarded as key events. This module binds
//! another keyboard on a display the caller already owns, or on its own
//! connection when `WAYLAND_DISPLAY` is set, and records `Key` events with
//! the evdev code still attached.
//!
//! It does not read the socket of a foreign display. The owner reads, then
//! [`ForeignKeyboard::dispatch_pending`] drains this queue. An own connection
//! may `roundtrip` because it owns its socket.
//!
//! `xenadmin-keys` is not created. The migration plan adds that crate only
//! after route A is selected, and this spike does not select it.

use std::ffi::c_void;

use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};

/// One key transition from `wl_keyboard::Event::Key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedKey {
    pub evdev: u32,
    pub down: bool,
}

struct KeyboardState {
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    events: Vec<RecordedKey>,
}

impl KeyboardState {
    fn new() -> Self {
        Self {
            seat: None,
            keyboard: None,
            events: Vec::new(),
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for KeyboardState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "wl_seat"
        {
            state.seat = Some(registry.bind(name, version.min(7), qh, ()));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for KeyboardState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if state.keyboard.is_none() && matches!(event, wl_seat::Event::Capabilities { .. }) {
            state.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for KeyboardState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            state: key_state,
            ..
        } = event
        {
            let down = match key_state {
                WEnum::Value(wl_keyboard::KeyState::Pressed) => true,
                WEnum::Value(wl_keyboard::KeyState::Released) => false,
                WEnum::Value(_) | WEnum::Unknown(_) => return,
            };
            state.events.push(RecordedKey { evdev: key, down });
        }
    }
}

/// A keyboard listener and the queue it dispatches.
pub struct ForeignKeyboard {
    connection: Connection,
    queue: EventQueue<KeyboardState>,
    state: KeyboardState,
}

impl ForeignKeyboard {
    /// Connect with `WAYLAND_DISPLAY`. This is a separate client, not route A.
    /// It can show whether a compositor emits `wl_keyboard::Key` at all.
    pub fn connect_own() -> Result<Self, String> {
        let connection = Connection::connect_to_env()
            .map_err(|error| format!("WAYLAND_DISPLAY connection failed: {error}"))?;
        Self::from_connection(connection)
    }

    /// Bind a second keyboard on a `wl_display` another client already reads.
    ///
    /// # Safety
    /// `display` must remain a live `wl_display` for the life of this value.
    /// The pointer is wrapped, not owned: this function does not disconnect
    /// it and does not read its socket. The display owner must keep reading,
    /// and the caller drains this queue with [`Self::dispatch_pending`].
    #[allow(unsafe_code)]
    pub unsafe fn from_foreign_display(display: *mut c_void) -> Result<Self, String> {
        if display.is_null() {
            return Err("the Wayland display pointer is null".into());
        }
        // SAFETY: the caller guarantees `display` is a live wl_display.
        // `from_foreign_display` wraps that pointer and does not take the
        // socket. The cast only changes the pointer type. No other memory
        // is touched here.
        let backend =
            unsafe { wayland_client::backend::Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, String> {
        let queue = connection.new_event_queue();
        let handle = queue.handle();
        let _registry = connection.display().get_registry(&handle, ());
        Ok(Self {
            connection,
            queue,
            state: KeyboardState::new(),
        })
    }

    /// Read this connection's socket until the seat keyboard is bound.
    /// Foreign displays must not call this.
    pub fn roundtrip_own(&mut self) -> Result<(), String> {
        self.queue
            .roundtrip(&mut self.state)
            .map_err(|error| format!("Wayland roundtrip failed: {error}"))?;
        self.queue
            .roundtrip(&mut self.state)
            .map_err(|error| format!("Wayland roundtrip failed: {error}"))?;
        Ok(())
    }

    pub fn keyboard_bound(&self) -> bool {
        self.state.keyboard.is_some()
    }

    /// Dispatch events the owner has already read into this queue.
    pub fn dispatch_pending(&mut self) -> Result<Vec<RecordedKey>, String> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| format!("Wayland dispatch_pending failed: {error}"))?;
        Ok(std::mem::take(&mut self.state.events))
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_wayland_display_is_an_error() {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return;
        }
        let Err(error) = ForeignKeyboard::connect_own() else {
            panic!("an absent WAYLAND_DISPLAY must fail");
        };
        assert!(
            error.contains("WAYLAND_DISPLAY"),
            "unexpected error: {error}"
        );
    }

    #[test]
    #[allow(unsafe_code)]
    fn a_null_foreign_display_is_refused() {
        // SAFETY: the pointer is null and the function rejects it before any
        // Wayland call. This test does not wrap a live display.
        let Err(error) = (unsafe { ForeignKeyboard::from_foreign_display(std::ptr::null_mut()) })
        else {
            panic!("a null display must fail");
        };
        assert!(error.contains("null"), "{error}");
    }
}
