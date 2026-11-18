//! The scenes of the area, under their Rune names.
//!
//! Every scene starts from the demo pool, so no scene has a stream: the
//! console is a synthetic session, and its snapshot is set by hand as the
//! Rune scenes set `console_snapshot`.
//!
//! | Scene | Shows |
//! | --- | --- |
//! | `console-recovery` | the Console tab of the first running guest; stream closed, no picture |
//! | `console-frame` | the same with a synthetic 800 by 600 picture, connected, in Fit |
//! | `console-actual-size` | the same in Actual size |
//! | `console-typing-review` | the same with a text waiting to be typed |
//! | `console-details` | `console-recovery` with the General tab in front: the console stays open behind it (Rune: the details pane beside the console) |
//! | `host-console` | the Console tab of the first server, its text console closed |
//! | `multiple-consoles` | two consoles detached into windows 1 and 2; the Window menu lists them (Rune: the manager dialog) |
//! | `console-window` | one console detached into window 1, with a 640 by 360 picture: what the front end renders is that WINDOW (`view::window(m, ConsoleWindowId(1))`) |
//!
//! `multiple-consoles` and `console-window` leave their Open requests in the
//! outbox, as a user's "Open in Separate Window" would.

use std::sync::Arc;

use xenadmin_core::{ConsoleFrame, ConsoleSnapshot, ConsoleState, PowerState, VmId};

use super::session::{ConsoleTarget, Seat};
use super::{session_mut, windows};
use crate::prelude::*;

const AREA: &str = "console";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("console-recovery", AREA, recovery_scene),
        SceneSpec::new("console-frame", AREA, frame_scene),
        SceneSpec::new("console-actual-size", AREA, actual_size_scene),
        SceneSpec::new("console-typing-review", AREA, typing_scene),
        SceneSpec::new("console-details", AREA, details_scene),
        SceneSpec::new("host-console", AREA, host_scene),
        SceneSpec::new("multiple-consoles", AREA, multiple_scene),
        SceneSpec::new("console-window", AREA, window_scene),
    ]
}

fn snapshot(state: ConsoleState, frame: Option<ConsoleFrame>, extended_keys: bool) -> ConsoleSnapshot {
    ConsoleSnapshot {
        state,
        frame: frame.map(Arc::new),
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys,
        typing: 0,
    }
}

/// Sets the snapshot of the session at `seat`, as a poll would.
pub(crate) fn set_snapshot(m: &mut Model, seat: Seat, snapshot: ConsoleSnapshot) {
    if let Some(s) = session_mut(m, seat) {
        s.set_snapshot(Some(Arc::new(snapshot)));
    }
}

/// Makes the target's object the scope, shows its Console tab and opens a
/// synthetic console there.
pub(crate) fn show(m: &mut Model, target: ConsoleTarget) {
    m.select(Target::one(target.object()));
    m.set_active_tab(TabId::Console);
    super::open_synthetic(m, target);
}

/// The guests of the demo pool that can have a console, as targets.
pub(crate) fn guests(m: &Model) -> Vec<ConsoleTarget> {
    let Some(inventory) = m.inventory.as_ref() else {
        return Vec::new();
    };
    inventory
        .vms
        .values()
        .filter(|vm| !vm.is_control_domain && !vm.is_template)
        .map(|vm| ConsoleTarget {
            pool: inventory.pool_id,
            vm: vm.id,
            name: vm.name.clone(),
            host: None,
        })
        .collect()
}

pub fn recovery_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(inventory) = m.inventory.clone() else {
        return;
    };
    let Some(vm) = inventory.vms.values().find(|vm| {
        vm.power_state == PowerState::Running && !vm.is_control_domain && !vm.is_template
    }) else {
        return;
    };
    show(
        m,
        ConsoleTarget {
            pool: inventory.pool_id,
            vm: vm.id,
            name: vm.name.clone(),
            host: None,
        },
    );
    set_snapshot(m, Seat::Tab, snapshot(ConsoleState::Closed, None, false));
}

/// A connected console showing a synthetic 800 by 600 frame. The recovery scene has no
/// frame, so without this the headless renders never draw the console canvas itself.
pub fn frame_scene(m: &mut Model) {
    recovery_scene(m);
    let (width, height) = (800_u16, 600_u16);
    let mut rgba = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for y in 0..height {
        for x in 0..width {
            // A light border marks the frame's edges; the gradient shows its orientation.
            let edge = x < 4 || y < 4 || x >= width - 4 || y >= height - 4;
            let (r, g, b) = if edge {
                (230, 230, 230)
            } else {
                (
                    (u32::from(x) * 255 / u32::from(width)) as u8,
                    (u32::from(y) * 255 / u32::from(height)) as u8,
                    96,
                )
            };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let frame = ConsoleFrame {
        width,
        height,
        rgba: rgba.into(),
    };
    set_snapshot(
        m,
        Seat::Tab,
        snapshot(ConsoleState::Connected, Some(frame), true),
    );
}

/// The same frame at its own size inside the panning view.
pub fn actual_size_scene(m: &mut Model) {
    frame_scene(m);
    if let Some(s) = session_mut(m, Seat::Tab) {
        s.actual_size = true;
    }
}

/// The console with clipboard text waiting to be typed as keys.
pub fn typing_scene(m: &mut Model) {
    frame_scene(m);
    if let Some(s) = session_mut(m, Seat::Tab) {
        s.arm_typing("echo first line\necho second line\n");
    }
}

/// Rune: the compact window's details pane over an open console. The
/// details are the General tab now; the console stays open behind it.
pub fn details_scene(m: &mut Model) {
    recovery_scene(m);
    m.set_active_tab(TabId::General);
}

/// The text console of the first server. The demo pool has no control
/// domain, so one is added: a running control domain resident on that
/// server, which the server names as its own.
pub fn host_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(mut inventory) = m.inventory.take().map(|i| i.as_ref().clone()) else {
        return;
    };
    let (Some(host), Some(mut vm)) = (
        inventory.hosts.values().next().cloned(),
        inventory
            .vms
            .values()
            .find(|v| v.power_state == PowerState::Running)
            .cloned(),
    ) else {
        m.inventory = Some(Arc::new(inventory));
        return;
    };
    vm.id = VmId::from(uuid::Uuid::from_u128(900));
    vm.is_control_domain = true;
    vm.host = Some(host.id);
    inventory
        .details
        .hosts
        .entry(host.id)
        .or_default()
        .control_domain = Some(vm.id);
    let target = ConsoleTarget {
        pool: inventory.pool_id,
        vm: vm.id,
        name: host.name.clone(),
        host: Some(host.id),
    };
    inventory.vms.insert(vm.id, vm);
    m.inventory = Some(Arc::new(inventory));
    show(m, target);
    set_snapshot(m, Seat::Tab, snapshot(ConsoleState::Closed, None, false));
}

/// A 640 by 360 picture tinted `tint`, on a closed stream (the Rune
/// `sample_frame`).
pub(crate) fn sample(tint: u8) -> ConsoleSnapshot {
    let width = 640u16;
    let height = 360u16;
    let rgba = (0..usize::from(width) * usize::from(height))
        .flat_map(|n| {
            [
                tint,
                (n % usize::from(width) / 5) as u8,
                (n / usize::from(width) / 3) as u8,
                255,
            ]
        })
        .collect::<Vec<_>>();
    snapshot(
        ConsoleState::Closed,
        Some(ConsoleFrame {
            width,
            height,
            rgba: rgba.into(),
        }),
        false,
    )
}

/// The consoles of the first two guests, each detached into a window (the
/// Rune `launch_demo`). The main window is left on the pool.
pub fn multiple_scene(m: &mut Model) {
    m.load_demo(3, 48);
    for (index, target) in guests(m).into_iter().take(2).enumerate() {
        super::open_synthetic(m, target);
        set_snapshot(m, Seat::Tab, sample(40 + index as u8 * 50));
        windows::detach(m);
    }
}

/// One detached console window (the Rune scene of the child window).
pub fn window_scene(m: &mut Model) {
    recovery_scene(m);
    set_snapshot(m, Seat::Tab, sample(60));
    windows::detach(m);
}
