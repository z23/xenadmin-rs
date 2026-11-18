//! The detached console windows: eight slots, which session each one
//! shows, and how one opens and closes. The bookkeeping of the Rune
//! `console_windows.rs`.
//!
//! In Rune each window was an application of its own with its own state,
//! and a console was handed over to it in a packet. Here every session
//! stays in the one model: detaching moves the tab's session into a slot,
//! and the front end is asked through [`Intent::ConsoleWindow`] to open,
//! raise or close the window of that slot. What the window shows it reads
//! from the model ([`super::view::window`]).
//!
//! The lease protocol is kept, because opening and closing a real window is
//! still not instantaneous:
//!
//! - every detach gives the slot a new lease, which the Open request
//!   carries ([`WindowPayload`]);
//! - a slot the model closed is `Closing` and cannot be taken again until
//!   the front end confirms with [`closed`] that the window is gone;
//! - [`closed`] with a lease that is not the slot's does nothing, so the
//!   close of an old window can never end the console of a new one.

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::session::{ConsoleTarget, Seat, Session};
use crate::prelude::*;

/// The number of detached windows (the Rune `LIMIT`).
pub const LIMIT: usize = 8;

/// Why a console cannot be detached when every slot is taken. In Rune the
/// button was disabled without it and `detach` put it on the error line.
pub const LIMIT_MESSAGE: &str =
    "Close an existing console window before opening another (maximum eight).";

#[derive(Default)]
pub(super) enum Slot {
    #[default]
    Free,
    Open {
        lease: u64,
        session: Box<Session>,
    },
    /// The model closed the console; the front end has not yet confirmed
    /// that the window is gone.
    Closing {
        lease: u64,
    },
}

#[derive(Default)]
pub struct Registry {
    pub(super) slots: [Slot; LIMIT],
}

/// What an Open request carries: `Intent::ConsoleWindow(Open { id, payload })`
/// with `payload` an `Rc<WindowPayload>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPayload {
    pub id: ConsoleWindowId,
    /// The lease of this opening. The front end hands it back in [`closed`].
    pub lease: u64,
    /// The window title: "{name} · XenAdmin RS console".
    pub title: String,
}

/// One open window, for the Window menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowEntry {
    pub id: ConsoleWindowId,
    pub lease: u64,
    pub target: ConsoleTarget,
}

fn next_lease() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The window ids, in slot order: 1 to 8.
pub fn ids() -> impl Iterator<Item = ConsoleWindowId> {
    (1..=LIMIT as u32).map(ConsoleWindowId)
}

fn index(id: ConsoleWindowId) -> Option<usize> {
    (1..=LIMIT as u32)
        .contains(&id.0)
        .then(|| id.0 as usize - 1)
}

pub(super) fn session(m: &Model, id: ConsoleWindowId) -> Option<&Session> {
    match m.areas.console.windows.slots.get(index(id)?)? {
        Slot::Open { session, .. } => Some(session),
        _ => None,
    }
}

pub(super) fn session_mut(m: &mut Model, id: ConsoleWindowId) -> Option<&mut Session> {
    match m.areas.console.windows.slots.get_mut(index(id)?)? {
        Slot::Open { session, .. } => Some(session),
        _ => None,
    }
}

/// The lease of the console the window shows now, if it shows one.
pub fn lease(m: &Model, id: ConsoleWindowId) -> Option<u64> {
    match m.areas.console.windows.slots.get(index(id)?)? {
        Slot::Open { lease, .. } => Some(*lease),
        _ => None,
    }
}

/// The open windows in slot order.
pub fn open_windows(m: &Model) -> Vec<WindowEntry> {
    ids()
        .filter_map(|id| {
            Some(WindowEntry {
                id,
                lease: lease(m, id)?,
                target: session(m, id)?.target.clone(),
            })
        })
        .collect()
}

/// The tab's console can move to a window: there is one (with a stream, or
/// in a synthetic pool) and a slot is free.
pub fn can_detach(m: &Model) -> bool {
    let state = &m.areas.console;
    state
        .tab
        .as_ref()
        .is_some_and(|s| s.console.is_some() || s.demo)
        && state
            .windows
            .slots
            .iter()
            .any(|slot| matches!(slot, Slot::Free))
}

/// Moves the tab's console into the first free window and asks the front
/// end to open it. The whole session moves: the stream, its latest frame,
/// the recovery counters and the choice of automatic reconnect. Input is
/// released in the guest first, and a save prompt still open is void.
pub fn detach(m: &mut Model) -> Option<ConsoleWindowId> {
    if !can_detach(m) {
        m.error = Some(LIMIT_MESSAGE.into());
        return None;
    }
    let state = &mut m.areas.console;
    let free = state
        .windows
        .slots
        .iter()
        .position(|slot| matches!(slot, Slot::Free))?;
    let mut session = state.tab.take()?;
    session.moving_window();
    let id = ConsoleWindowId(free as u32 + 1);
    let lease = next_lease();
    let title = session.target.window_title();
    state.windows.slots[free] = Slot::Open {
        lease,
        session: Box::new(session),
    };
    m.send(Intent::ConsoleWindow(ConsoleWindowRequest::Open {
        id,
        payload: Some(Rc::new(WindowPayload { id, lease, title })),
    }));
    Some(id)
}

/// Raises the window, when it shows a console (the Rune "Show window").
pub fn show(m: &mut Model, id: ConsoleWindowId) {
    if lease(m, id).is_some() {
        m.send(Intent::ConsoleWindow(ConsoleWindowRequest::Activate(id)));
    }
}

/// The model closes the window's console: its "Close console" control and
/// "Close All Consoles". Input is released and the stream ends now; the
/// slot stays taken until the front end confirms with [`closed`].
pub fn close(m: &mut Model, id: ConsoleWindowId) {
    let Some(index) = index(id) else {
        return;
    };
    let slot = &mut m.areas.console.windows.slots[index];
    let Slot::Open { lease, session } = slot else {
        return;
    };
    session.close();
    let lease = *lease;
    *slot = Slot::Closing { lease };
    m.send(Intent::ConsoleWindow(ConsoleWindowRequest::Close(id)));
}

/// Closes every window's console (the Rune `close_all`). The front end runs
/// it when the main window closes, before `Model::close`.
pub fn close_all(m: &mut Model) {
    for id in ids() {
        close(m, id);
    }
}

/// The front end reports that the window of this lease is gone: the user
/// closed it, or the front end carried out a Close request. An open console
/// of that lease ends here (input released, stream closed); a slot waiting
/// for the confirmation is free again. Another lease: nothing happens.
/// Returns whether the slot was this lease's.
pub fn closed(m: &mut Model, id: ConsoleWindowId, lease: u64) -> bool {
    let Some(index) = index(id) else {
        return false;
    };
    let slot = &mut m.areas.console.windows.slots[index];
    match slot {
        Slot::Open {
            lease: current,
            session,
        } if *current == lease => {
            session.close();
            *slot = Slot::Free;
            true
        }
        Slot::Closing { lease: current } if *current == lease => {
            *slot = Slot::Free;
            true
        }
        _ => false,
    }
}

/// The seat of a window, when it shows a console.
pub fn seat(m: &Model, id: ConsoleWindowId) -> Option<Seat> {
    lease(m, id).map(|_| Seat::Window(id))
}
