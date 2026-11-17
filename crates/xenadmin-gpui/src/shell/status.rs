//! The status bar, from `Model::status()`.
//!
//! One line: the connection state and freshness of the scope session, the
//! count of operations in progress, the count of unknown outcomes, and what
//! is selected. The two counts open the Notifications view at its Events.
//!
//! Above the line, each on a line of its own because it can be a sentence
//! or two: the demo banner, the notice about hosts outside the qualified
//! release, why the selection moved by itself, and the shared error line.

use xenadmin_ui::nav::NavView;
use xenadmin_ui::surface::Tone;
use xenadmin_ui::tabs::TabId;

use crate::kit::*;
use crate::render::{Ui, pressable, tone_color};
use crate::store::dispatch;

/// The id of the status area: the notices and the line.
pub const STATUS_KEY: &str = "status-bar";
/// The id of the connection state.
pub const CONNECTION_KEY: &str = "status-connection";
/// The id of the count of operations in progress.
pub const OPERATIONS_KEY: &str = "status-operations";
/// The id of the count of unknown outcomes.
pub const UNKNOWN_KEY: &str = "status-unknown-outcomes";
/// The id of the selection.
pub const SELECTION_KEY: &str = "status-selection";
/// The id of the demo banner.
pub const DEMO_KEY: &str = "demo-banner";
/// The id of the notice about hosts outside the qualified release.
pub const HOST_NOTICE_KEY: &str = "host-version-notice";
/// The id of the notice that says why the selection moved.
pub const NOTICE_KEY: &str = "selection-notice";
/// The id of the error line.
pub const ERROR_KEY: &str = "connection-error";

/// The scope of the status bar's focus handles in the widget cache.
const SCOPE: &str = "status";

fn count(number: usize, one: &str, many: &str) -> String {
    if number == 1 {
        format!("1 {one}")
    } else {
        format!("{number} {many}")
    }
}

/// One notice line above the status line.
fn notice(
    key: &'static str,
    role: Role,
    icon: IconName,
    tone: Tone,
    text: String,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    let color = tone_color(tone, cx);
    let text = SharedString::from(text);
    h_flex()
        .id(key)
        .test_support()
        .role(role)
        .aria_label(text.clone())
        .w_full()
        .items_start()
        .gap_2()
        .px_3()
        .py_1()
        .border_t_1()
        .border_color(border)
        .text_xs()
        .child(
            Icon::new(icon)
                .small()
                .flex_none()
                .mt_0p5()
                .text_color(color),
        )
        .child(div().flex_1().min_w_0().text_color(color).child(text))
        .into_any_element()
}

/// A count that opens the Notifications view at its Events.
fn events_link(ui: &Ui, key: &'static str, text: String, tone: Tone, cx: &mut App) -> AnyElement {
    let handle = ui.focus_handle(SCOPE, key, cx);
    let theme = cx.theme();
    let hover = theme.accent;
    let ring = theme.ring;
    let radius = theme.radius;
    let color = tone_color(tone, cx);
    let store = ui.store.clone();
    let text = SharedString::from(text);
    pressable(key, &handle, move |_, cx| {
        dispatch(&store, cx, |m| {
            m.set_nav_view(NavView::Notifications);
            m.set_active_tab(TabId::Events);
        });
    })
    .role(Role::Link)
    .aria_label(text.clone())
    .aria_description("Opens the events of the Notifications view")
    .flex_none()
    .px_1p5()
    .rounded(radius)
    .border_1()
    .border_color(gpui_kit::transparent_black())
    .text_color(color)
    .hover(move |style| style.bg(hover))
    .focus_visible(move |style| style.border_color(ring))
    .child(text)
    .into_any_element()
}

/// The status area: the notices, then the status line.
pub fn status_bar(ui: &Ui, cx: &mut App) -> AnyElement {
    let status = ui.store.read(cx).model().status();
    let mut notices = Vec::new();
    if let Some(banner) = status.demo {
        notices.push(notice(
            DEMO_KEY,
            Role::Status,
            IconName::Info,
            Tone::Muted,
            banner.to_owned(),
            cx,
        ));
    }
    if let Some(text) = status.host_notice.clone() {
        notices.push(notice(
            HOST_NOTICE_KEY,
            Role::Status,
            IconName::TriangleAlert,
            Tone::Warning,
            text,
            cx,
        ));
    }
    if let Some(text) = status.notice.clone() {
        notices.push(notice(
            NOTICE_KEY,
            Role::Status,
            IconName::Info,
            Tone::Normal,
            text,
            cx,
        ));
    }
    if let Some(text) = status.error.clone() {
        notices.push(notice(
            ERROR_KEY,
            Role::Alert,
            IconName::CircleAlert,
            Tone::Danger,
            text,
            cx,
        ));
    }
    let operations = events_link(
        ui,
        OPERATIONS_KEY,
        count(status.operations, "operation", "operations"),
        if status.operations > 0 {
            Tone::Normal
        } else {
            Tone::Muted
        },
        cx,
    );
    let unknown = events_link(
        ui,
        UNKNOWN_KEY,
        count(
            status.unknown_outcomes,
            "unknown outcome",
            "unknown outcomes",
        ),
        if status.unknown_outcomes > 0 {
            Tone::Warning
        } else {
            Tone::Muted
        },
        cx,
    );
    let connection = SharedString::from(status.connection.clone());
    let selection = SharedString::from(status.selection.clone());
    let line = StatusBar::new()
        .left(
            div()
                .id(CONNECTION_KEY)
                .test_support()
                .role(Role::Status)
                .aria_label(connection.clone())
                .min_w_0()
                .truncate()
                .child(connection),
        )
        .left(Separator::vertical().h_3())
        .left(operations)
        .left(unknown)
        .right(
            div()
                .id(SELECTION_KEY)
                .test_support()
                .role(Role::Status)
                .aria_label(if selection.is_empty() {
                    SharedString::from("Nothing selected")
                } else {
                    selection.clone()
                })
                .min_w_0()
                .truncate()
                .child(selection),
        );
    v_flex()
        .id(STATUS_KEY)
        .test_support()
        .role(Role::Group)
        .aria_label("Status")
        .w_full()
        .flex_none()
        .children(notices)
        .child(line)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::count;

    #[test]
    fn a_count_is_said_in_the_singular_for_one() {
        assert_eq!(count(0, "operation", "operations"), "0 operations");
        assert_eq!(count(1, "operation", "operations"), "1 operation");
        assert_eq!(
            count(3, "unknown outcome", "unknown outcomes"),
            "3 unknown outcomes"
        );
    }
}
