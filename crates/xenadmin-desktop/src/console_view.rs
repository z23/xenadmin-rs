use super::{State, caption, heading};
use rune::{El, Key, KeyEvent, Modifier, Phase, PhysicalInput, SurfaceFit, Tx, col, el, row};
use rune_kit::{Button, ButtonVariant, Kit};
use xenadmin_core::{ConsoleInput, ConsoleState};

pub(super) fn release(s: &mut State) {
    if let Some(console) = &s.console {
        let _ = console.input(ConsoleInput::ReleaseAll);
    }
    s.console_buttons = 0;
    s.console_keys.clear();
}

pub(super) fn view(s: &State, k: &Kit) -> El {
    let status = match s.console_snapshot.as_ref().map(|s| &s.state) {
        Some(ConsoleState::Connected) => "Connected directly to the host console".into(),
        Some(ConsoleState::Failed(error)) => error.clone(),
        Some(ConsoleState::Closed) => "Console closed. Reopen it from the VM details.".into(),
        _ => "Opening the host console…".into(),
    };
    let canvas = el()
        .key("console-canvas")
        .role("application")
        .label("VM console. Click to send keyboard and pointer input")
        .focusable(true)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(rune::Color::BLACK)
        .pixels_fit(&s.console_pixels, SurfaceFit::Fit)
        .on_layout_tx(|s: &mut State, rect, tx| {
            s.console_bounds = [rect[2], rect[3]];
            tx.live_only();
        })
        .on_key(|s: &mut State, event, tx| key(s, event, tx, true))
        .on_key_up(|s: &mut State, event, tx| key(s, event, tx, false))
        .on_physical_input(|s: &mut State, event, _| {
            if matches!(event, PhysicalInput::FocusLost) {
                release(s);
            }
            false
        })
        .on_pointer_move_local(|s: &mut State, event, tx| {
            pointer(s, event.x, event.y, s.console_buttons);
            tx.live_only();
        })
        .on_drag(|s: &mut State, event, tx| {
            s.console_buttons = if matches!(event.phase, Phase::Up) {
                0
            } else {
                1
            };
            if event.cancelled() {
                release(s);
            } else {
                pointer(s, event.x, event.y, s.console_buttons);
            }
            tx.live_only();
        })
        .on_secondary_press_local(|s: &mut State, event, _| {
            pointer(s, event.x, event.y, 4);
            pointer(s, event.x, event.y, 0);
        });
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(heading("VM console", k))
                .child(
                    Button::new("Back to inventory")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut State| {
                            release(s);
                            s.console = None;
                            s.console_snapshot = None;
                        })
                        .build(k)
                        .key("close-console"),
                ),
        )
        .child(caption(status, k).wrap())
        .child(canvas)
        .child(caption(
            "Click the console for keyboard input. Leaving the window releases held keys.",
            k,
        ))
}

fn pointer(s: &State, x: f32, y: f32, buttons: u8) {
    let Some(console) = &s.console else {
        return;
    };
    let Some(frame) = s.console_snapshot.as_ref().and_then(|s| s.frame.as_ref()) else {
        return;
    };
    let [width, height] = s.console_bounds;
    let scale = (width / f32::from(frame.width)).min(height / f32::from(frame.height));
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let x = ((x - (width - f32::from(frame.width) * scale) * 0.5) / scale)
        .clamp(0.0, f32::from(frame.width - 1)) as u16;
    let y = ((y - (height - f32::from(frame.height) * scale) * 0.5) / scale)
        .clamp(0.0, f32::from(frame.height - 1)) as u16;
    let _ = console.input(ConsoleInput::Pointer { x, y, buttons });
}

fn key(s: &mut State, event: &KeyEvent, tx: &mut Tx, down: bool) -> bool {
    let Some(console) = &s.console else {
        return false;
    };
    let mut symbol = keysym(&event.key);
    if let Some(physical) = tx.physical_key() {
        if down {
            if let Some((_, existing)) = s.console_keys.iter().find(|(key, _)| *key == physical) {
                symbol = Some(*existing);
            } else if let Some(symbol) = symbol {
                s.console_keys.push((physical, symbol));
            }
        } else if let Some(index) = s.console_keys.iter().position(|(key, _)| *key == physical) {
            symbol = Some(s.console_keys.remove(index).1);
        }
    }
    if let Some(keysym) = symbol {
        let down = if let Key::Modifier { pressed, .. } = event.key {
            pressed
        } else {
            down
        };
        if let Err(error) = console.input(ConsoleInput::Key { keysym, down }) {
            s.error = Some(error.to_string());
        }
        tx.live_only();
        true
    } else {
        false
    }
}

fn keysym(key: &Key) -> Option<u32> {
    Some(match key {
        Key::Text(value) => {
            let c = value.chars().next()? as u32;
            if c <= 0xff { c } else { 0x0100_0000 | c }
        }
        Key::Command(c) => *c as u32,
        Key::Enter => 0xff0d,
        Key::Backspace => 0xff08,
        Key::Tab => 0xff09,
        Key::Escape => 0xff1b,
        Key::Delete => 0xffff,
        Key::Insert => 0xff63,
        Key::Left => 0xff51,
        Key::Up => 0xff52,
        Key::Right => 0xff53,
        Key::Down => 0xff54,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::Function(n @ 1..=24) => 0xffbd + u32::from(*n),
        Key::Modifier { modifier, .. } => match modifier {
            Modifier::ShiftLeft => 0xffe1,
            Modifier::ShiftRight => 0xffe2,
            Modifier::ControlLeft => 0xffe3,
            Modifier::ControlRight => 0xffe4,
            Modifier::AltLeft => 0xffe9,
            Modifier::AltRight => 0xffea,
            Modifier::SuperLeft => 0xffeb,
            Modifier::SuperRight => 0xffec,
            _ => return None,
        },
        _ => return None,
    })
}
