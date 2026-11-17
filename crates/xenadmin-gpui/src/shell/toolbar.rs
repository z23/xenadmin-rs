//! The toolbar: the commands with a toolbar slot, in the groups of the
//! plan (Back and Forward; Add Server, New Storage, New VM; the power
//! commands that apply now).
//!
//! `AppStore::toolbar` resolves the groups on the model's target. A power
//! command that is not eligible is not in its group at all; any other
//! disabled command stays, greyed, with its reason as its tooltip.

use xenadmin_ui::command::{self, MenuItem, ToolbarGroup};

use super::{icons, menus};
use crate::kit::*;
use crate::render::Ui;
use crate::store::AppStore;

/// The id of the toolbar.
pub const TOOLBAR_KEY: &str = "toolbar";

/// One toolbar button: its id is the command id. Back and Forward show
/// their icons only; the others an icon and the label.
fn toolbar_button(store: &Entity<AppStore>, item: &MenuItem) -> Button {
    let command = item.id;
    let label = SharedString::from(item.label.clone());
    let icon = icons::command_icon(command);
    let compact = command::find(command)
        .and_then(|spec| spec.toolbar)
        .is_some_and(|slot| slot.group == ToolbarGroup::Navigation);
    let button = Button::new(SharedString::from(command.0));
    let button = match icon {
        Some(icon) if compact => button
            .icon(Icon::new(icon))
            .accessibility_label(label.clone()),
        Some(icon) => button.icon(Icon::new(icon)).label(label.clone()),
        None => button.label(label.clone()),
    };
    // An icon alone needs its name in the tooltip; a disabled button says
    // why.
    let tooltip = match (&item.disabled, compact && icon.is_some()) {
        (Some(_), true) => Some(SharedString::from(menus::item_name(item))),
        (Some(reason), false) => Some(SharedString::from(reason.clone())),
        (None, true) => Some(label),
        (None, false) => None,
    };
    let button = match tooltip {
        Some(tooltip) => button.tooltip(tooltip),
        None => button,
    };
    let button = match item.checked {
        Some(checked) => button.selected(checked),
        None => button,
    };
    if item.disabled.is_some() {
        button.disabled(true)
    } else {
        let store = store.clone();
        button.on_click(move |_, _, cx| {
            store.update(cx, |store, cx| store.invoke(command, cx));
        })
    }
}

/// The toolbar row.
pub fn toolbar(ui: &Ui, cx: &App) -> AnyElement {
    let groups = ui.store.read(cx).toolbar();
    let theme = cx.theme();
    let border = theme.border;
    let mut bar = Toolbar::new("toolbar-items");
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            bar = bar.content(Separator::vertical().h_4().mx_1());
        }
        for item in group {
            bar = bar.child(toolbar_button(&ui.store, item));
        }
    }
    h_flex()
        .id(TOOLBAR_KEY)
        .test_support()
        .role(Role::Toolbar)
        .aria_label("Toolbar")
        .w_full()
        .flex_none()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(border)
        .child(bar)
        .into_any_element()
}
