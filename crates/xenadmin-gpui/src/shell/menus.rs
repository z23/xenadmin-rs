//! The menu bar: the application menus built from the resolved command
//! table.
//!
//! `AppStore::menu_bar` gives the ten menus for the model's target, hidden
//! commands omitted and disabled ones with their reason. They become GPUI
//! application menus whose items carry [`RunCommand`], and the Kit's
//! `AppMenuBar` draws them in the window.
//!
//! Three steps make a change visible (kit-platform.md, 3.3): `cx.set_menus`,
//! the copy in the Kit's `GlobalState`, and `AppMenuBar::reload`. The reload
//! closes an open menu, so the shell does all three only when the resolved
//! menus differ from the ones it installed last.

use xenadmin_ui::command::{MenuEntry, MenuItem, MenuView};

use super::actions::RunCommand;
use crate::kit::*;

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
