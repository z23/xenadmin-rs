//! Asking a Wayland compositor to show the main window, for a later start
//! of the client.
//!
//! A second start hands its request (and the activation token its launcher
//! gave it, if any) to the running client, which must then ask the
//! compositor for attention. GPUI's `Window::activate_window` does ask, but
//! on Wayland it puts the serial of the last key or pointer press on the
//! token it requests, and it cannot be given a token from outside. A
//! compositor built on wlroots refuses a token with a serial unless the
//! window still has the keyboard or the pointer, and then ignores the
//! activation without a word: in the driven session of 2026-11-19 (sway
//! 1.12) a second start showed nothing at all once the pointer was over
//! another window (`docs/reports/2026-11-19-gpui-vm-session.md`).
//!
//! So the request is also made here, on the display GPUI's window uses, in
//! the way of [`crate::wayland_keys`]: another event queue on the same
//! `wl_display`, whose socket GPUI reads.
//!
//! - The token of the later start, when it has one, is passed to
//!   `xdg_activation_v1.activate` as it came: it is the one a compositor may
//!   honour with focus.
//! - A token without a serial is then requested and used. The compositor
//!   cannot grant focus for it and marks the window as asking for attention,
//!   wherever the pointer is.
//!
//! [`Activator::poll`] must be called until it says nothing is outstanding:
//! the answers arrive when GPUI has read the socket.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::protocol::{wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};

/// The longest forwarded token that is passed on. The tokens compositors
/// make are tens of characters; a Wayland request holds 4096 bytes in all,
/// and one that cannot be sent is the end of the display connection.
pub const TOKEN_LIMIT: usize = 256;

/// The token a later start forwarded, if it is one: launchers export an
/// empty variable as readily as none. A token longer than [`TOKEN_LIMIT`]
/// is none either: the request for attention goes out without it.
pub fn forwarded_token(token: Option<&str>) -> Option<&str> {
    token
        .map(str::trim)
        .filter(|token| !token.is_empty() && token.len() <= TOKEN_LIMIT)
}

struct State {
    activation: Option<xdg_activation_v1::XdgActivationV1>,
    /// Tokens the compositor has answered with, not yet used.
    granted: Vec<String>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "xdg_activation_v1"
            && state.activation.is_none()
        {
            state.activation = Some(registry.bind(name, 1, qh, ()));
        }
    }
}

impl Dispatch<xdg_activation_v1::XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &xdg_activation_v1::XdgActivationV1,
        _: xdg_activation_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        token: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token: granted } = event {
            state.granted.push(granted);
            token.destroy();
        }
    }
}

/// The requests for attention of one window.
pub struct Activator {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    surface: wl_surface::WlSurface,
    app_id: String,
    /// Requests that wait for the compositor's activation global.
    waiting: Vec<Option<String>>,
    /// Token requests the compositor has not answered.
    asked: usize,
}

impl Activator {
    /// For the Wayland window `window`. An error means there is nothing to
    /// do here: the test platform, a headless render, X11.
    #[allow(unsafe_code)]
    pub fn from_window(
        window: &(impl HasDisplayHandle + HasWindowHandle),
        app_id: &str,
    ) -> Result<Self, String> {
        let display = window
            .display_handle()
            .map_err(|error| format!("the window has no display handle: {error}"))?;
        let RawDisplayHandle::Wayland(display) = display.as_raw() else {
            return Err("the window display is not Wayland".to_string());
        };
        let surface = window
            .window_handle()
            .map_err(|error| format!("the window has no window handle: {error}"))?;
        let RawWindowHandle::Wayland(surface) = surface.as_raw() else {
            return Err("the window is not a Wayland surface".to_string());
        };
        // SAFETY: both pointers are the ones GPUI's Wayland backend
        // published for this window: a live `wl_display` and a live
        // `wl_surface` of it. The window must outlive this value. The
        // wrapper does not disconnect the display, does not read its socket
        // and does not destroy the surface.
        unsafe { Self::from_raw(display.display.as_ptr(), surface.surface.as_ptr(), app_id) }
    }

    /// # Safety
    ///
    /// `display` must be a live `wl_display` and `surface` a live
    /// `wl_surface` of that display, both outliving the returned value.
    #[allow(unsafe_code)]
    pub unsafe fn from_raw(
        display: *mut std::ffi::c_void,
        surface: *mut std::ffi::c_void,
        app_id: &str,
    ) -> Result<Self, String> {
        if display.is_null() || surface.is_null() {
            return Err("the Wayland display or surface pointer is null".into());
        }
        // SAFETY: the caller guarantees `display` is a live wl_display. The
        // wrapper does not take its socket.
        let backend =
            unsafe { wayland_client::backend::Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        // SAFETY: the caller guarantees `surface` is a live wl_surface of
        // that display. The id made here does not own the proxy.
        let id = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                surface.cast(),
            )
        }
        .map_err(|error| format!("the window surface is not a wl_surface: {error}"))?;
        let surface = wl_surface::WlSurface::from_id(&connection, id)
            .map_err(|error| format!("the window surface cannot be used: {error}"))?;
        let queue = connection.new_event_queue();
        let _registry = connection.display().get_registry(&queue.handle(), ());
        connection
            .flush()
            .map_err(|error| format!("Wayland flush failed: {error}"))?;
        Ok(Self {
            connection,
            queue,
            state: State {
                activation: None,
                granted: Vec::new(),
            },
            surface,
            app_id: app_id.to_string(),
            waiting: Vec::new(),
            asked: 0,
        })
    }

    /// Queues one request for attention. `token` is what the later start
    /// forwarded.
    pub fn request(&mut self, token: Option<&str>) {
        self.waiting
            .push(forwarded_token(token).map(str::to_string));
    }

    /// Makes the requests that can be made now and uses the tokens that
    /// have arrived. Returns whether anything is still outstanding.
    pub fn poll(&mut self) -> Result<bool, String> {
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| format!("Wayland dispatch_pending failed: {error}"))?;
        if let Some(activation) = &self.state.activation {
            for forwarded in self.waiting.drain(..) {
                if let Some(forwarded) = forwarded {
                    activation.activate(forwarded, &self.surface);
                }
                let token = activation.get_activation_token(&self.queue.handle(), ());
                token.set_app_id(self.app_id.clone());
                token.set_surface(&self.surface);
                token.commit();
                self.asked += 1;
            }
            for granted in self.state.granted.drain(..) {
                activation.activate(granted, &self.surface);
                self.asked = self.asked.saturating_sub(1);
            }
        }
        self.connection
            .flush()
            .map_err(|error| format!("Wayland flush failed: {error}"))?;
        Ok(!self.waiting.is_empty() || self.asked > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_forwarded_token_is_no_token() {
        assert_eq!(forwarded_token(None), None);
        assert_eq!(forwarded_token(Some("")), None);
        assert_eq!(forwarded_token(Some("  \n")), None);
        assert_eq!(forwarded_token(Some(" abc123 ")), Some("abc123"));
    }

    #[test]
    fn a_forwarded_token_too_long_for_a_request_is_no_token() {
        let longest = "t".repeat(TOKEN_LIMIT);
        assert_eq!(forwarded_token(Some(&longest)), Some(longest.as_str()));
        // What the core accepts from a second start: up to 4096 bytes.
        let long = "t".repeat(4096);
        assert_eq!(forwarded_token(Some(&long)), None);
        assert_eq!(forwarded_token(Some(&"t".repeat(TOKEN_LIMIT + 1))), None);
    }

    #[test]
    #[allow(unsafe_code)]
    fn null_pointers_are_refused() {
        // SAFETY: the function rejects a null pointer before any Wayland
        // call. This test does not wrap a live display.
        let Err(error) = (unsafe {
            Activator::from_raw(std::ptr::null_mut(), std::ptr::null_mut(), "xenadmin-rs")
        }) else {
            panic!("null pointers must fail");
        };
        assert!(error.contains("null"), "{error}");
    }
}
