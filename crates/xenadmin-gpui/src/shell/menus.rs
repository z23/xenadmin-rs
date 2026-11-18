//! The menu bar: the application menus built from the resolved command
//! table.
//!
//! `AppStore::menu_bar` gives the ten menus for the model's target, hidden
//! commands omitted and disabled ones with their reason. They become GPUI
//! application menus whose items carry [`RunCommand`], and the Kit's
//! `AppMenuBar` draws them in the window.
//!
//! The context menu of a row is the model's too (`Model::context_menu`):
//! its items are built here and the shell presents it.
//!
//! Three steps make a change visible (kit-platform.md, 3.3): `cx.set_menus`,
//! the copy in the Kit's `GlobalState`, and `AppMenuBar::reload`. The reload
//! closes an open menu, so the shell does all three only when the resolved
//! menus differ from the ones it installed last.

use std::cell::Cell;
use std::rc::Rc;

use xenadmin_ui::command::{MenuEntry, MenuItem, MenuView};

use super::actions::RunCommand;
use crate::kit::*;
use crate::store::{AppStore, dispatch};

/// Where the next context menu opens. Whoever asks the model for a context
/// menu from a pointer press or from a row says where first; the shell
/// reads it when it presents the menu the model has open.
#[derive(Clone, Default)]
pub struct MenuAnchor {
    position: Rc<Cell<Option<Point<Pixels>>>>,
    requests: Rc<Cell<u64>>,
}

impl MenuAnchor {
    /// The menu about to be asked for opens at `position`.
    pub fn request(&self, position: Point<Pixels>) {
        self.position.set(Some(position));
        self.requests.set(self.requests.get() + 1);
    }

    /// The position of the last request, once.
    pub fn take_position(&self) -> Option<Point<Pixels>> {
        self.position.take()
    }

    /// How many menus were asked for with a position. A presented menu
    /// that is older than the last request is replaced.
    pub fn requests(&self) -> u64 {
        self.requests.get()
    }
}

/// One item of a context menu. A disabled item carries its reason in its
/// name, as in the menu bar. Choosing it runs the command through the
/// model's context menu, which checks the captured selection again; a
/// refusal is said in the error line.
pub fn context_item(store: &Entity<AppStore>, item: &MenuItem) -> PopupMenuItem {
    let command = item.id;
    let store = store.clone();
    let mut entry =
        PopupMenuItem::new(SharedString::from(item_name(item))).disabled(item.disabled.is_some());
    if let Some(checked) = item.checked {
        entry = entry.checked(checked);
    }
    entry.on_click(move |_, _, cx| {
        dispatch(&store, cx, move |m| {
            if let Err(refused) = m.invoke_context(command) {
                m.error = Some(refused.to_string());
            }
        });
    })
}

/// What a menu shows when no command of it applies to the selection. The
/// ten menus keep their places whatever is selected.
pub const NO_COMMANDS: &str = "No commands for this selection";

/// The name of a menu item. The Kit's menus have no tooltip, so a disabled
/// item says why in its name.
pub fn item_name(item: &MenuItem) -> String {
    match item.disabled.as_deref() {
        Some(reason) if !reason.is_empty() => format!("{} — {reason}", item.label),
        _ => item.label.clone(),
    }
}

fn app_item(item: &MenuItem) -> AppMenuItem {
    let entry = AppMenuItem::action(item_name(item), RunCommand { id: item.id.0 })
        .disabled(item.disabled.is_some());
    match item.checked {
        Some(checked) => entry.checked(checked),
        None => entry,
    }
}

fn app_menu(view: &MenuView) -> AppMenu {
    let mut items = Vec::new();
    for (index, group) in view.groups.iter().enumerate() {
        if index > 0 && !group.is_empty() {
            items.push(AppMenuItem::separator());
        }
        for entry in group {
            items.push(match entry {
                MenuEntry::Item(item) => app_item(item),
                MenuEntry::Submenu { label, items } => {
                    AppMenuItem::submenu(AppMenu::new(*label).items(items.iter().map(app_item)))
                }
            });
        }
    }
    if items.is_empty() {
        items.push(AppMenuItem::action(NO_COMMANDS, RunCommand { id: "" }).disabled(true));
    }
    AppMenu::new(view.title).items(items)
}

/// The application menus of the resolved menu bar.
pub fn app_menus(views: &[MenuView]) -> Vec<AppMenu> {
    views.iter().map(app_menu).collect()
}

/// Steps one and two: the platform's menus and the copy the Kit's bar
/// reads. The caller reloads the bar afterwards.
pub fn install(views: &[MenuView], cx: &mut App) {
    cx.set_menus(app_menus(views));
    // Not `cx.get_menus()`: a platform without application menus keeps
    // none, and the bar in the window is the only one on Linux.
    let owned: Vec<OwnedMenu> = app_menus(views).into_iter().map(AppMenu::owned).collect();
    GlobalState::global_mut(cx).set_app_menus(owned);
}

#[cfg(test)]
mod tests {
    use xenadmin_ui::Model;
    use xenadmin_ui::command;

    use super::*;

    fn names(menu: &OwnedMenu) -> Vec<(String, bool, bool)> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                OwnedMenuItem::Action {
                    name,
                    disabled,
                    checked,
                    ..
                } => Some((name.clone(), *disabled, *checked)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_ten_menus_keep_their_places_and_say_why_an_item_is_disabled() {
        let model = Model::demo(3, 48);
        let menus: Vec<OwnedMenu> = app_menus(&command::menu_bar(&model))
            .into_iter()
            .map(AppMenu::owned)
            .collect();
        let titles: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
        assert_eq!(
            titles,
            [
                "File",
                "View",
                "Pool",
                "Server",
                "VM",
                "Storage",
                "Templates",
                "Tools",
                "Window",
                "Help"
            ]
        );
        let view = names(&menus[1]);
        // A toggle carries its state; a disabled command its reason.
        assert!(view.contains(&("Toolbar".into(), false, true)));
        assert!(view.contains(&(
            "Reset Interface Size — The interface is at its normal size.".into(),
            true,
            false
        )));
        // No menu is empty: one without a command for the selection says so.
        for menu in &menus {
            assert!(!menu.items.is_empty(), "{}", menu.name);
        }
    }

    #[test]
    fn a_submenu_and_the_separators_between_groups_are_kept() {
        let item = |id: &'static str, label: &str| MenuItem {
            id: command::CommandId(id),
            label: label.into(),
            shortcut: None,
            disabled: None,
            checked: None,
        };
        let view = MenuView {
            menu: command::Menu::Vm,
            title: "VM",
            groups: vec![
                vec![MenuEntry::Item(item("vm.new", "New VM"))],
                vec![MenuEntry::Submenu {
                    label: "Start/Shut Down",
                    items: vec![item("vm.start", "Start"), item("vm.shut-down", "Shut Down")],
                }],
            ],
        };
        let menu = app_menu(&view).owned();
        assert_eq!(menu.items.len(), 3);
        assert!(matches!(menu.items[1], OwnedMenuItem::Separator));
        let OwnedMenuItem::Submenu(submenu) = &menu.items[2] else {
            panic!("the third entry is the submenu");
        };
        assert_eq!(submenu.name.as_ref(), "Start/Shut Down");
        assert_eq!(names(submenu).len(), 2);
    }
}
