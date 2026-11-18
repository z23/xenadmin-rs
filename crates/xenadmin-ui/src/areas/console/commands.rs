//! The console's controls and the command table.
//!
//! A [`Control`] is one button of the Rune console toolbar: what it is
//! called, when it can be pressed and what it does, for a session wherever
//! it is shown. The Console tab offers the controls as commands on its
//! local action row (this file); a detached window, which has no command
//! table, offers the same controls as buttons ([`super::view`]). Both go
//! through [`control_state`] and [`run`], so they cannot drift apart.
//!
//! Eligibility of the tab's commands:
//!
//! - `Hidden` unless the target is the object whose console the tab shows.
//!   Until a console is open the row has one command, Open Console (Open
//!   Host Text Console for a server).
//! - `Hidden` for a control that the kind of console does not have: the
//!   three guest chords on a host's text console, Ctrl+C and Ctrl+D on a
//!   guest's.
//! - `Disabled` with a reason where Rune disabled the button. Rune gave no
//!   reason for a control that needs a connected console; the reason here
//!   is the status line the console shows at that moment.
//!
//! "Hold right button" is gone (decision 0006: a right-button drag reaches
//! the guest natively).

use xenadmin_core::{ConsoleInput, HostId, Inventory, PoolId, PowerState, VmId};

use super::session::Seat;
use super::{export, session, session_mut, windows};
use crate::prelude::*;

const AREA: &str = "console";

pub const OPEN: CommandId = CommandId("vm.console");
pub const OPEN_HOST: CommandId = CommandId("server.console");
pub const RECONNECT: CommandId = CommandId("console.reconnect");
pub const ACTUAL_SIZE: CommandId = CommandId("console.actual-size");
pub const FULLSCREEN: CommandId = CommandId("console.fullscreen");
pub const UNDOCK: CommandId = CommandId("console.undock");
pub const RELEASE_INPUT: CommandId = CommandId("console.release-input");
pub const SAVE_SCREENSHOT: CommandId = CommandId("console.save-screenshot");
pub const SEND_CTRL_ALT_DEL: CommandId = CommandId("vm.send-ctrl-alt-del");
pub const SEND_CTRL_ALT_F1: CommandId = CommandId("console.send-ctrl-alt-f1");
pub const SEND_ALT_TAB: CommandId = CommandId("console.send-alt-tab");
pub const SEND_CTRL_C: CommandId = CommandId("console.send-ctrl-c");
pub const SEND_CTRL_D: CommandId = CommandId("console.send-ctrl-d");
pub const SEND_CLIPBOARD: CommandId = CommandId("console.send-clipboard");
pub const TYPE_CLIPBOARD: CommandId = CommandId("console.type-clipboard");
pub const COPY_CLIPBOARD: CommandId = CommandId("console.copy-remote-clipboard");
pub const CLOSE: CommandId = CommandId("console.close");
pub const CLOSE_ALL_WINDOWS: CommandId = CommandId("window.console.close-all");
/// The Window menu's entry of each detached window, by slot.
pub const WINDOWS: [CommandId; windows::LIMIT] = [
    CommandId("window.console-1"),
    CommandId("window.console-2"),
    CommandId("window.console-3"),
    CommandId("window.console-4"),
    CommandId("window.console-5"),
    CommandId("window.console-6"),
    CommandId("window.console-7"),
    CommandId("window.console-8"),
];

/// Rune's caption beside the disabled "Open console" button.
pub const READ_ONLY: &str = "Read-only connection: changes and interactive consoles are disabled.";
/// Rune disabled "Open console" without a reason when the connection was
/// not current; the words follow the power commands' line for the same
/// condition.
pub const NOT_CURRENT: &str =
    "The connection is not current. Consoles stay disabled until it reconnects.";
/// Rune disabled the host console button in demo mode without a reason.
pub const DEMO: &str = "Demo mode has no consoles to open.";
/// Rune disabled the host console button without a reason when the host's
/// control domain was missing or not running.
pub const NO_CONTROL_DOMAIN: &str = "This server's control domain is not running or is not known.";
/// `vm.send-ctrl-alt-del` in the VM menu, for a VM whose console is not the
/// one the Console tab shows.
pub const NOT_OPEN: &str = "Open this VM's console to send keys to it.";
pub const CONNECTED: &str = "The console is connected.";
pub const EXPORTING: &str = "A console screenshot is already being saved.";
pub const NO_FRAME: &str = "The console has not shown a picture yet.";
pub const NO_CLIPBOARD: &str = "The guest has not offered any clipboard text.";
pub const CLIPBOARD_SENT: &str =
    "Clipboard text sent. The guest receives it only if its tools support clipboard exchange.";
pub const CLIPBOARD_COPIED: &str = "Remote clipboard copied locally.";

/// One control of the console toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Reconnect,
    ActualSize,
    Fullscreen,
    Undock,
    ReleaseInput,
    SaveScreenshot,
    CtrlAltDelete,
    CtrlAltF1,
    AltTab,
    CtrlC,
    CtrlD,
    SendClipboard,
    TypeClipboard,
    CopyClipboard,
    Close,
}

impl Control {
    /// The rows of the Rune toolbar, in order.
    pub const ROWS: [&'static [Control]; 3] = [
        &[
            Control::Reconnect,
            Control::ActualSize,
            Control::Fullscreen,
            Control::Undock,
            Control::ReleaseInput,
            Control::SaveScreenshot,
        ],
        &[
            Control::CtrlAltDelete,
            Control::CtrlAltF1,
            Control::AltTab,
            Control::CtrlC,
            Control::CtrlD,
        ],
        &[
            Control::SendClipboard,
            Control::TypeClipboard,
            Control::CopyClipboard,
        ],
    ];

    /// The key of the control as a button in a detached window: the Rune
    /// key where the Rune button had one.
    pub fn key(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect-console",
            Self::ActualSize => "console-actual-size",
            Self::Fullscreen => "console-fullscreen",
            Self::Undock => "detach-console",
            Self::ReleaseInput => "console-release-input",
            Self::SaveScreenshot => "save-console-screenshot",
            Self::CtrlAltDelete => "console-ctrl-alt-delete",
            Self::CtrlAltF1 => "console-ctrl-alt-f1",
            Self::AltTab => "console-alt-tab",
            Self::CtrlC => "console-ctrl-c",
            Self::CtrlD => "console-ctrl-d",
            Self::SendClipboard => "send-console-clipboard",
            Self::TypeClipboard => "type-console-clipboard",
            Self::CopyClipboard => "copy-console-clipboard",
            Self::Close => "close-console",
        }
    }

    /// The Rune label of the button.
    pub fn label(self, actual_size: bool, side: &str) -> String {
        match self {
            Self::Reconnect => "Reconnect".into(),
            Self::ActualSize if actual_size => "Fit to panel".into(),
            Self::ActualSize => "Actual size".into(),
            Self::Fullscreen => "Fullscreen".into(),
            Self::Undock => "Open in separate window".into(),
            Self::ReleaseInput => "Release input".into(),
            Self::SaveScreenshot => "Save screenshot…".into(),
            Self::CtrlAltDelete => "Ctrl+Alt+Delete".into(),
            Self::CtrlAltF1 => "Ctrl+Alt+F1".into(),
            Self::AltTab => "Alt+Tab".into(),
            Self::CtrlC => "Ctrl+C".into(),
            Self::CtrlD => "Ctrl+D".into(),
            Self::SendClipboard => format!("Send clipboard to {side}"),
            Self::TypeClipboard => "Type clipboard as keys".into(),
            Self::CopyClipboard => format!("Copy {side} clipboard"),
            Self::Close => "Close console".into(),
        }
    }

    /// The keysyms of a chord control, pressed in order and released in
    /// reverse.
    fn chord(self) -> Option<Vec<u32>> {
        Some(match self {
            Self::CtrlAltDelete => vec![0xffe3, 0xffe9, 0xffff],
            Self::CtrlAltF1 => vec![0xffe3, 0xffe9, 0xffbe],
            Self::AltTab => vec![0xffe9, 0xff09],
            Self::CtrlC => vec![0xffe3, 0x63],
            Self::CtrlD => vec![0xffe3, 0x64],
            _ => return None,
        })
    }
}

/// Whether the control can be pressed for the session at `seat`. `Hidden`:
/// there is no such session, or its kind of console has no such control.
pub fn control_state(m: &Model, seat: Seat, control: Control) -> Eligibility {
    let Some(s) = session(m, seat) else {
        return Eligibility::Hidden;
    };
    let host = s.target.host.is_some();
    let ready = || Eligibility::enabled_unless(s.not_ready_reason());
    match control {
        Control::Reconnect => Eligibility::enabled_if(!s.input_ready(), CONNECTED),
        Control::ActualSize | Control::Fullscreen | Control::ReleaseInput | Control::Close => {
            Eligibility::Enabled
        }
        Control::Undock if seat != Seat::Tab => Eligibility::Hidden,
        Control::Undock => Eligibility::enabled_if(windows::can_detach(m), windows::LIMIT_MESSAGE),
        Control::SaveScreenshot if s.export.pending => Eligibility::disabled(EXPORTING),
        Control::SaveScreenshot => Eligibility::enabled_if(s.desktop_size().is_some(), NO_FRAME),
        Control::CtrlAltDelete | Control::CtrlAltF1 | Control::AltTab if host => {
            Eligibility::Hidden
        }
        Control::CtrlC | Control::CtrlD if !host => Eligibility::Hidden,
        Control::CtrlAltDelete
        | Control::CtrlAltF1
        | Control::AltTab
        | Control::CtrlC
        | Control::CtrlD
        | Control::SendClipboard
        | Control::TypeClipboard => ready(),
        Control::CopyClipboard => match ready() {
            Eligibility::Enabled => Eligibility::enabled_if(
                s.snapshot
                    .as_ref()
                    .is_some_and(|x| x.guest_clipboard.is_some()),
                NO_CLIPBOARD,
            ),
            refused => refused,
        },
    }
}

/// Presses the control. Every control releases the guest's input first, as
/// its Rune button did; the ones that send something are refused again by
/// the session when it is not ready.
pub fn run(m: &mut Model, seat: Seat, control: Control) {
    if control == Control::Undock {
        if seat == Seat::Tab {
            windows::detach(m);
        }
        return;
    }
    if control == Control::Close {
        match seat {
            Seat::Tab => super::close_tab(m),
            Seat::Window(id) => windows::close(m, id),
        }
        return;
    }
    if control == Control::SaveScreenshot {
        export::choose(m, seat);
        return;
    }
    let Some(s) = session_mut(m, seat) else {
        return;
    };
    match control {
        Control::Reconnect => {
            let error = s.manual_reconnect().err().map(|error| error.to_string());
            super::set_error(m, seat, error);
        }
        Control::ActualSize => {
            s.release();
            s.actual_size = !s.actual_size;
        }
        Control::Fullscreen => {
            s.release();
            m.toggle_fullscreen(match seat {
                Seat::Tab => WindowRef::Main,
                Seat::Window(id) => WindowRef::Console(id),
            });
        }
        Control::ReleaseInput => s.release(),
        Control::SendClipboard | Control::TypeClipboard => {
            s.release();
            let serial = s.serial;
            m.read_clipboard(move |m, text| {
                let Some(s) = session_mut(m, seat) else {
                    return;
                };
                if s.console.is_none() || s.serial != serial {
                    return;
                }
                let text = text.unwrap_or_default();
                if control == Control::TypeClipboard {
                    s.arm_typing(&text);
                } else {
                    s.message = Some(CLIPBOARD_SENT.into());
                    s.send(ConsoleInput::Clipboard(text));
                }
            });
        }
        Control::CopyClipboard => {
            s.release();
            if let Some(text) = s
                .snapshot
                .as_ref()
                .and_then(|x| x.guest_clipboard.as_ref())
                .map(|text| text.to_string())
            {
                s.message = Some(CLIPBOARD_COPIED.into());
                m.copy_text(text);
            }
        }
        _ => {
            if let Some(keys) = control.chord() {
                s.release();
                s.send(ConsoleInput::KeyChord(keys));
            }
        }
    }
}

/// The target is the object whose console the Console tab shows.
pub fn shows(m: &Model, target: &Target) -> bool {
    m.areas
        .console
        .tab
        .as_ref()
        .is_some_and(|s| target.single() == Some(&s.target.object()))
}

fn tab_state(m: &Model, target: &Target, control: Control) -> Eligibility {
    if !shows(m, target) {
        return Eligibility::Hidden;
    }
    control_state(m, Seat::Tab, control)
}

fn tab_run(m: &mut Model, target: &Target, control: Control) {
    if shows(m, target) {
        run(m, Seat::Tab, control);
    }
}

/// The pool is shown through a synthetic session (the demo).
fn synthetic(m: &Model, pool: PoolId) -> bool {
    match crate::session::session_for_pool(m, pool) {
        Some(id) if m.workspace.selected == Some(id) => m.demo,
        Some(id) => m.workspace.synthetic.contains_key(&id),
        None => false,
    }
}

/// Why no console can be opened in this pool: an interactive console needs
/// a current connection with write access.
fn refusal(m: &Model, pool: PoolId) -> Option<&'static str> {
    if m.can_mutate_pool(pool) {
        None
    } else if synthetic(m, pool) {
        Some(DEMO)
    } else if m.connection_for(pool).is_some_and(|c| c.is_read_only()) {
        Some(READ_ONLY)
    } else {
        Some(NOT_CURRENT)
    }
}

/// The control domain whose console is the host's text console: the VM the
/// host names, when it is a control domain resident and running there.
pub fn control_domain(inventory: &Inventory, host: HostId) -> Option<VmId> {
    let id = inventory.details.hosts.get(&host)?.control_domain?;
    inventory
        .vms
        .get(&id)
        .filter(|v| {
            v.is_control_domain
                && !v.is_template
                && v.host == Some(host)
                && v.power_state == PowerState::Running
        })
        .map(|v| v.id)
}

fn open_state(m: &Model, target: &Target) -> Eligibility {
    let Some(object) = target.single_of(ObjectKind::Vm) else {
        return Eligibility::Hidden;
    };
    let (Some(pool), Some(vm)) = (object.pool, object.vm_id()) else {
        return Eligibility::Hidden;
    };
    let guest = m.inventory_for(pool).is_some_and(|inventory| {
        inventory
            .vms
            .get(&vm)
            .is_some_and(|v| !v.is_template && !v.is_control_domain)
    });
    if !guest || shows(m, target) {
        return Eligibility::Hidden;
    }
    Eligibility::enabled_unless(refusal(m, pool))
}

fn open_host_state(m: &Model, target: &Target) -> Eligibility {
    let Some(object) = target.single_of(ObjectKind::Host) else {
        return Eligibility::Hidden;
    };
    let (Some(pool), Some(host)) = (object.pool, object.host_id()) else {
        return Eligibility::Hidden;
    };
    let Some(inventory) = m.inventory_for(pool) else {
        return Eligibility::Hidden;
    };
    if !inventory.hosts.contains_key(&host) || shows(m, target) {
        return Eligibility::Hidden;
    }
    if let Some(reason) = refusal(m, pool) {
        return Eligibility::disabled(reason);
    }
    Eligibility::enabled_if(
        control_domain(&inventory, host).is_some(),
        NO_CONTROL_DOMAIN,
    )
}

/// Open Console and Open Host Text Console: the object becomes the scope,
/// its Console tab is shown and the console is opened there. A failure is
/// on the shared error line, as in Rune.
fn open(m: &mut Model, target: &Target) {
    let Some(object) = target.single().cloned() else {
        return;
    };
    m.select(Target::one(object.clone()));
    m.set_active_tab(TabId::Console);
    // Showing the Console tab opens the console of an eligible object
    // already (`on_tab_change`); a failure of that is on the error line.
    if !shows(m, target) {
        m.error = super::open(m, &object)
            .err()
            .map(|error| error.to_string());
    }
}

fn ctrl_alt_del_state(m: &Model, target: &Target) -> Eligibility {
    if target.single_of(ObjectKind::Vm).is_none() {
        return Eligibility::Hidden;
    }
    if !shows(m, target) {
        return Eligibility::disabled(NOT_OPEN);
    }
    control_state(m, Seat::Tab, Control::CtrlAltDelete)
}

fn window_state<const SLOT: u32>(m: &Model, _: &Target) -> Eligibility {
    if windows::lease(m, ConsoleWindowId(SLOT)).is_some() {
        Eligibility::Enabled
    } else {
        Eligibility::Hidden
    }
}

fn window_show<const SLOT: u32>(m: &mut Model, _: &Target) {
    windows::show(m, ConsoleWindowId(SLOT));
}

fn window_label<const SLOT: u32>(m: &Model, _: &Target) -> Option<String> {
    windows::session(m, ConsoleWindowId(SLOT)).map(|s| s.target.heading())
}

fn tab_side(m: &Model) -> Option<&'static str> {
    m.areas.console.tab.as_ref().map(|s| s.target.side())
}

macro_rules! window_command {
    ($slot:literal) => {
        CommandSpec::new(
            WINDOWS[$slot - 1].0,
            concat!("Console Window ", $slot),
            AREA,
        )
        .menu(place::window_console($slot))
        .eligibility(window_state::<$slot>)
        .invoke(window_show::<$slot>)
        .dynamic_label(window_label::<$slot>)
    };
}

macro_rules! tab_command {
    ($id:expr, $label:literal, $order:literal, $control:expr) => {
        CommandSpec::new($id.0, $label, AREA)
            .local(TabId::Console, $order)
            .local_scopes(&[ObjectKind::Host, ObjectKind::Vm])
            .eligibility(|m, target| tab_state(m, target, $control))
            .invoke(|m, target| tab_run(m, target, $control))
    };
}

/// The local action row of the Console tab in the order of the Rune
/// toolbar, then the Window menu.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        // The plan's Menus table has no row for either; they are on the
        // context menus and, until a console is open, alone on the tab's row.
        CommandSpec::new(OPEN.0, "Open Console", AREA)
            .context(&[ObjectKind::Vm])
            .local(TabId::Console, 10)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(open_state)
            .invoke(open),
        CommandSpec::new(OPEN_HOST.0, "Open Host Text Console", AREA)
            .context(&[ObjectKind::Host])
            .local(TabId::Console, 11)
            .local_scopes(&[ObjectKind::Host])
            .eligibility(open_host_state)
            .invoke(open),
        tab_command!(RECONNECT, "Reconnect", 20, Control::Reconnect),
        tab_command!(ACTUAL_SIZE, "Actual Size", 30, Control::ActualSize).checked(|m| {
            m.areas
                .console
                .tab
                .as_ref()
                .is_some_and(|s| s.actual_size)
        }),
        tab_command!(FULLSCREEN, "Fullscreen", 40, Control::Fullscreen),
        tab_command!(UNDOCK, "Open in Separate Window", 50, Control::Undock),
        tab_command!(RELEASE_INPUT, "Release Input", 60, Control::ReleaseInput),
        tab_command!(
            SAVE_SCREENSHOT,
            "Save Screenshot…",
            70,
            Control::SaveScreenshot
        ),
        // The plan's label and menu row; also on the tab's row, where Rune
        // had the button "Ctrl+Alt+Delete".
        CommandSpec::new(SEND_CTRL_ALT_DEL.0, "Send Ctrl+Alt+Del", AREA)
            .menu(place::VM_SEND_CTRL_ALT_DEL)
            .local(TabId::Console, 80)
            .local_scopes(&[ObjectKind::Vm])
            .eligibility(ctrl_alt_del_state)
            .invoke(|m, target| tab_run(m, target, Control::CtrlAltDelete)),
        tab_command!(SEND_CTRL_ALT_F1, "Ctrl+Alt+F1", 90, Control::CtrlAltF1),
        tab_command!(SEND_ALT_TAB, "Alt+Tab", 100, Control::AltTab),
        tab_command!(SEND_CTRL_C, "Ctrl+C", 110, Control::CtrlC),
        tab_command!(SEND_CTRL_D, "Ctrl+D", 120, Control::CtrlD),
        tab_command!(
            SEND_CLIPBOARD,
            "Send Clipboard to VM",
            130,
            Control::SendClipboard
        )
        .dynamic_label(|m, _| {
            tab_side(m)
                .filter(|side| *side == "host")
                .map(|_| "Send Clipboard to Host".into())
        }),
        tab_command!(
            TYPE_CLIPBOARD,
            "Type Clipboard as Keys…",
            140,
            Control::TypeClipboard
        ),
        tab_command!(
            COPY_CLIPBOARD,
            "Copy VM Clipboard",
            150,
            Control::CopyClipboard
        )
        .dynamic_label(|m, _| {
            tab_side(m)
                .filter(|side| *side == "host")
                .map(|_| "Copy Host Clipboard".into())
        }),
        // Rune: "Back to inventory". The console also closes when another
        // object is selected.
        tab_command!(CLOSE, "Close Console", 200, Control::Close),
        window_command!(1),
        window_command!(2),
        window_command!(3),
        window_command!(4),
        window_command!(5),
        window_command!(6),
        window_command!(7),
        window_command!(8),
        CommandSpec::new(CLOSE_ALL_WINDOWS.0, "Close All Consoles", AREA)
            .menu(place::WINDOW_CLOSE_ALL_CONSOLES)
            .eligibility(|m, _| {
                Eligibility::enabled_if(
                    !windows::open_windows(m).is_empty(),
                    "No console window is open.",
                )
            })
            .invoke(|m, _| windows::close_all(m)),
    ]
}
