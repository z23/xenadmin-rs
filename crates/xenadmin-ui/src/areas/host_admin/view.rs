//! The open surface, described. Nothing here changes the model: it is
//! rebuilt on every render from the editor and the model as they are.
//!
//! One editor, two shapes: a change with a dialog of its own is a `Dialog`
//! titled with the change; a change that is a setting of the server is the
//! current page of the `Properties` dialog. Inside, both read the same way,
//! in the Rune order: the draft and a preview of what it would do, or the
//! review; the preview notice in a synthetic pool; the error; and a footer
//! of Close, Back, and Review or Confirm.
//!
//! Keys and labels are the Rune ones. `host-close` and `host-back` are new:
//! Rune gave those two buttons no key.

use xenadmin_core::{PermissionAssessment, ResourceTarget};

use super::change::{self, Editor, Kind, Page};
use super::commands::{MULTIPATH_WORKFLOW, POWER_ON_SETTINGS};
use crate::prelude::*;

/// The id of the server's Properties dialog.
pub const PROPERTIES: &str = "host-properties";

/// The key of the review, so `host-change-review-stale` is its stale line.
pub const REVIEW: &str = "host-change-review";

const PREVIEW: &str = "Synthetic preview · no host changes can be submitted";

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses, MODEL-API 4).
const DEMO: &str = "Demo mode cannot change infrastructure.";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(w) = &m.areas.host_admin.editor else {
        return Vec::new();
    };
    let surface = match w.kind.page() {
        Some(page) => Surface::properties(
            PROPERTIES,
            "Properties",
            Page::ALL.map(|page| (page.id(), page.label())),
            page.id(),
            show_page,
        ),
        None => Surface::dialog(dialog_id(w.kind), w.kind.title()),
    };
    // The permission line belongs to the review: before it there is no
    // operation to assess.
    let permission = w
        .review
        .as_ref()
        .map(|op| permissions::assess_for(m, w.pool(), &op.permission_requirements()));
    let surface = surface
        .subtitle(format!("{} ({})", w.label(), w.target.uuid()))
        .experimental(w.kind.experimental())
        .body(body(m, w))
        .footer(footer(m, w, permission.as_ref()))
        .on_close(super::close);
    vec![match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }]
}

/// The id of the dialog of a change that is not a Properties page.
fn dialog_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Maintenance => "host-enter-maintenance",
        Kind::Evacuate => "host-evacuate",
        Kind::Exit => "host-exit-maintenance",
        Kind::Reboot => "host-reboot",
        Kind::Shutdown => "host-shutdown",
        Kind::Restart => "host-restart-toolstack",
        Kind::PowerOn => "host-power-on",
        Kind::Memory => "host-dom0-memory",
        Kind::Forget => "host-forget",
        // Not `pool-autostart`: that was the key of the Rune button, which is
        // now the command `pool.vm-autostart`.
        Kind::Autostart => "pool-vm-autostart",
        Kind::WakeOnLan | Kind::DisablePowerOn | Kind::Logging | Kind::Multipath => PROPERTIES,
    }
}

/// The page list: turning the page starts that page's change afresh, as
/// choosing another change in the Rune chooser did.
fn show_page(m: &mut Model, id: &str) {
    let Some(w) = &mut m.areas.host_admin.editor else {
        return;
    };
    let kind = match Page::from_id(id) {
        Some(Page::Multipathing) => Kind::Multipath,
        Some(Page::LogDestination) => Kind::Logging,
        Some(Page::PowerOn) => w.power_on_kind(),
        None => return,
    };
    w.select(kind);
}

fn body(m: &Model, w: &Editor) -> Vec<Block> {
    blocks![
        // A page says which change it is; a dialog has it as its title.
        w.kind.page().map(|_| heading(w.kind.title())),
        match &w.review {
            Some(op) => blocks![review(REVIEW, change::lines(w, op)).stale(change::stale(m, w))],
            None => blocks![draft(w), preview(w), links(m, w)],
        },
        change::synthetic(m, w).then(|| notice(NoticeKind::Preview, PREVIEW).key("host-preview")),
        w.error.clone().map(|error| alert(error).key("host-error")),
    ]
}

/// What the user enters before the review, for the changes that have it.
fn draft(w: &Editor) -> Vec<Block> {
    match w.kind {
        Kind::Memory => blocks![setting(w, "dom0 memory (MiB)")],
        Kind::Logging => blocks![setting(
            w,
            "Remote syslog hostname or IP (blank for local only)"
        )],
        Kind::Forget => blocks![checkbox(
            "host-forget-off",
            "I independently verified that this host is powered off",
            w.acknowledged,
            |m| {
                if let Some(w) = &mut m.areas.host_admin.editor {
                    w.acknowledged = !w.acknowledged;
                    w.error = None;
                }
            }
        )],
        // The two Rune changes of the power-on mode are one choice here.
        // Each option keeps the key of the chooser button it replaces.
        Kind::WakeOnLan | Kind::DisablePowerOn => {
            let mode = |kind: Kind| {
                option(mode_id(kind), kind.title()).key(format!("host-choice-{}", kind.title()))
            };
            blocks![
                choice(
                    "host-power-on-mode",
                    [mode(Kind::WakeOnLan), mode(Kind::DisablePowerOn)],
                    Some(mode_id(w.kind)),
                    |m, id| {
                        let kind = [Kind::WakeOnLan, Kind::DisablePowerOn]
                            .into_iter()
                            .find(|kind| mode_id(*kind) == id);
                        if let Some(kind) = kind {
                            change::select(m, kind);
                        }
                    }
                )
                .label("Remote power-on")
            ]
        }
        _ => Vec::new(),
    }
}

fn mode_id(kind: Kind) -> &'static str {
    if kind == Kind::WakeOnLan {
        "wake-on-lan"
    } else {
        "disabled"
    }
}

/// The one text draft. Enter asks for the review.
fn setting(w: &Editor, label: &str) -> Field {
    input("host-setting-value", label, &w.value, |m, text| {
        if let Some(w) = &mut m.areas.host_admin.editor {
            w.value = text.to_owned();
            w.error = None;
        }
    })
    .on_submit(change::review)
}

/// Before the review is asked for, the draft already shows what it would
/// do: the lines the review will have, and why it would be refused if it
/// would be.
fn preview(w: &Editor) -> Vec<Block> {
    let refusal = |reason: String| caption(reason).tone(Tone::Warning).key("host-unavailable");
    match change::operation(w) {
        Ok(op) => {
            let availability = op.availability(&w.initial, w.target);
            blocks![
                change::lines(w, &op)
                    .into_iter()
                    .map(caption)
                    .collect::<Vec<_>>(),
                (!availability.is_available()).then(|| refusal(availability.explanation())),
            ]
        }
        Err(reason) => blocks![refusal(reason)],
    }
}

/// The ways from a page to a workflow of another area, which in Rune were
/// buttons of the chooser beside the simple change.
fn links(m: &Model, w: &Editor) -> Vec<Block> {
    match w.kind.page() {
        Some(Page::Multipathing) => blocks![link(
            m,
            w,
            "host-multipath-workflow",
            "Multipathing with storage reconnection…",
            MULTIPATH_WORKFLOW,
        )],
        Some(Page::PowerOn) => blocks![link(
            m,
            w,
            "host-bmc-settings",
            "BMC / power-on settings…",
            POWER_ON_SETTINGS,
        )],
        _ => Vec::new(),
    }
}

/// A button that runs another area's command on this server. It follows
/// that command's eligibility: absent while the command is hidden (or its
/// area has not registered it), disabled with the command's reason. This
/// surface gives way to the one the command opens, as it did in Rune (the
/// `open` of each of those modules cleared `host_workspace`); when the
/// command is refused after all, it stays and `m.error` has the reason.
pub(super) fn link(
    m: &Model,
    w: &Editor,
    key: &str,
    label: &str,
    command: CommandId,
) -> Option<Button> {
    let ResourceTarget::Host(host) = w.target else {
        return None;
    };
    let target = Target::one(ObjectRef::host(w.pool(), host));
    let reason = match m.eligibility(command, &target) {
        Eligibility::Hidden => return None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Enabled => None,
    };
    Some(
        button(key, label, move |m| {
            let this = m.areas.host_admin.editor.take();
            if !m.invoke(command, &target) {
                m.areas.host_admin.editor = this;
            }
        })
        .disabled_opt(reason),
    )
}

fn footer(m: &Model, w: &Editor, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let close = button("host-close", "Close", super::close);
    if w.review.is_none() {
        return vec![
            close,
            button("host-review", "Review", change::review).primary(),
        ];
    }
    let confirm = button("host-confirm", "Confirm host change", change::submit)
        .danger()
        // The first reason wins: stale, then a known denial, then the demo.
        .disabled_opt(change::stale(m, w))
        .disabled_opt(permission.and_then(permissions::denial))
        .disabled_if(change::synthetic(m, w), DEMO);
    // A review that was opened by its command has no draft to go back to.
    if w.kind.has_draft() || w.kind.page().is_some() {
        vec![close, button("host-back", "Back", change::back), confirm]
    } else {
        vec![close, confirm]
    }
}
