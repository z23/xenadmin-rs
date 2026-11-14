//! Spike S4 on the Kit's headless test platform.
//!
//! A native Wayland session was not available in this run. These tests do
//! not establish Wayland input, the system clipboard, portal dialogs, or
//! Orca.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::tree::{TreeItem, TreeState, tree as navigation_tree};
use gpui_kit::component::{GlobalState, Theme, ThemeMode, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    App, Bounds, ClickEvent, Entity, Focusable as _, Menu, MenuItem, OwnedMenuItem, SharedString,
    TestAppContext, TestSupportExt as _, Window, WindowBounds, WindowOptions, div, point, px, size,
};

gpui_kit::actions!(
    shell_spike,
    [ImportAction, ExportAction, ToolbarAction, SaveName]
);

struct Names;

impl TableDelegate for Names {
    fn columns_count(&self, _: &App) -> usize {
        1
    }

    fn rows_count(&self, _: &App) -> usize {
        3
    }

    fn column(&self, _: usize, _: &App) -> Column {
        Column::new("name", "Name").width(px(140.))
    }

    fn render_td(
        &mut self,
        row: usize,
        _: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div().child(["Alpha", "Beta", "Gamma"][row])
    }
}

struct Shell {
    menus: Entity<AppMenuBar>,
    tree: Entity<TreeState>,
    table: Entity<TableState<Names>>,
    name: Entity<InputState>,
    tab: usize,
    step: usize,
    vm_selected: bool,
    saved: bool,
    confirmed: Rc<Cell<bool>>,
    error: bool,
}

impl Shell {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let menus = AppMenuBar::new(cx);
        let tree = cx.new(|cx| {
            TreeState::new(cx).items(vec![
                TreeItem::new("pool", "Demonstration pool")
                    .expanded(true)
                    .children([
                        TreeItem::new("host", "xcp-lab-01"),
                        TreeItem::new("vm", "Xen Orchestra"),
                    ]),
            ])
        });
        let table = cx.new(|cx| TableState::new(Names, window, cx));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        Self {
            menus,
            tree,
            table,
            name,
            tab: 0,
            step: 0,
            vm_selected: false,
            saved: false,
            confirmed: Rc::new(Cell::new(false)),
            error: false,
        }
    }
}

fn install_stub_menus(cx: &mut App, vm_selected: bool) {
    let items: Vec<MenuItem> = xenadmin_gpui::stub_menus(vm_selected)
        .into_iter()
        .map(|item| {
            let action = match item.label {
                "Import" => MenuItem::action(item.label, ImportAction),
                "Export" => MenuItem::action(item.label, ExportAction),
                "Toolbar" => MenuItem::action(item.label, ToolbarAction),
                _ => unreachable!("stub label"),
            };
            action.disabled(item.disabled).checked(item.checked)
        })
        .collect();
    cx.set_menus([Menu::new("File").items(items)]);
    let owned = cx.get_menus().expect("platform menus");
    GlobalState::global_mut(cx).set_app_menus(owned);
}

fn menu_flags(cx: &App) -> Vec<(String, bool, bool)> {
    cx.get_menus()
        .expect("menus")
        .into_iter()
        .flat_map(|menu| menu.items)
        .filter_map(|item| match item {
            OwnedMenuItem::Action {
                name,
                disabled,
                checked,
                ..
            } => Some((name, disabled, checked)),
            _ => None,
        })
        .collect()
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let confirmed = Rc::clone(&self.confirmed);
        v_flex()
            .id("shell")
            .test_support()
            .size_full()
            .gap_2()
            .p_2()
            .child(self.menus.clone())
            .child(
                TabBar::new("tabs")
                    .child(Tab::new().label("General"))
                    .child(Tab::new().label("Search"))
                    .selected_index(self.tab)
                    .on_click(move |index, _, cx| {
                        this.update(cx, |shell, cx| {
                            shell.tab = *index;
                            cx.notify();
                        });
                    }),
            )
            .child(
                h_flex().gap_2().child(
                    div()
                        .id("navigation")
                        .test_support()
                        .w(px(220.))
                        .h(px(180.))
                        .child(navigation_tree(&self.tree, |_, entry, _, _, _| {
                            gpui_kit::component::list::ListItem::new(entry.item().id.clone())
                                .child(entry.item().label.clone())
                        })),
                ),
            )
            .child(div().h(px(120.)).child(DataTable::new(&self.table)))
            .child(
                Button::new("select-vm")
                    .label("Select VM")
                    .on_click(cx.listener(|shell, _: &ClickEvent, _, cx| {
                        shell.vm_selected = true;
                        install_stub_menus(cx, true);
                        shell.menus.update(cx, |menus, cx| menus.reload(cx));
                        cx.notify();
                    })),
            )
            .child(
                Button::new("open-rename")
                    .label("Rename")
                    .on_click(cx.listener(|shell, _: &ClickEvent, window, cx| {
                        let name = shell.name.clone();
                        let shell_entity = cx.entity();
                        window.open_dialog(cx, move |dialog, _, _| {
                            let shell_entity = shell_entity.clone();
                            let name = name.clone();
                            dialog.title("Rename").child(
                                v_flex()
                                    .gap_2()
                                    .child(Input::new(&name).id("server-name"))
                                    .child(Button::new("save-name").label("Save").on_click(
                                        move |_, _, cx| {
                                            let value = name.read(cx).value();
                                            shell_entity.update(cx, |shell, cx| {
                                                shell.error = value.is_empty();
                                                shell.saved = !value.is_empty();
                                                cx.notify();
                                            });
                                        },
                                    )),
                            )
                        });
                    })),
            )
            .child(
                Button::new("open-wizard")
                    .label("New VM")
                    .on_click(cx.listener(|_shell, _: &ClickEvent, window, cx| {
                        let shell_entity = cx.entity();
                        window.open_dialog(cx, move |dialog, _, cx| {
                            let step = shell_entity.read(cx).step;
                            let shell_entity = shell_entity.clone();
                            dialog.title("New VM").child(
                                v_flex()
                                    .id("wizard")
                                    .test_support()
                                    .gap_2()
                                    .child(
                                        Stepper::new("new-vm")
                                            .selected_index(step)
                                            .item(StepperItem::new().child("Template"))
                                            .item(StepperItem::new().child("Name"))
                                            .item(StepperItem::new().child("Review")),
                                    )
                                    .child(if step == 2 {
                                        div().id("review").test_support().child("Review the VM")
                                    } else {
                                        div().id("wizard-step").test_support().child("Step")
                                    })
                                    .child(Button::new("wizard-next").label("Next").on_click(
                                        move |_, _, cx| {
                                            shell_entity.update(cx, |shell, cx| {
                                                shell.step = (shell.step + 1).min(2);
                                                cx.notify();
                                            });
                                        },
                                    )),
                            )
                        });
                    })),
            )
            .child({
                let confirmed = Rc::clone(&confirmed);
                Button::new("open-alert")
                    .label("Delete")
                    .on_click(move |_, window, cx| {
                        let confirmed = Rc::clone(&confirmed);
                        window.open_alert_dialog(cx, move |alert, _, _| {
                            let confirmed = Rc::clone(&confirmed);
                            alert
                                .confirm()
                                .title("Delete VM")
                                .description("Delete the selected VM?")
                                .on_ok(move |_, _, _| {
                                    confirmed.set(true);
                                    true
                                })
                        });
                    })
            })
            .when(self.error, |row| {
                row.child(
                    div()
                        .id("name-error")
                        .test_support()
                        .child("Name is required"),
                )
            })
            .child(
                div()
                    .id(SharedString::from(format!("tab-{}", self.tab)))
                    .test_support(),
            )
    }
}

fn open_shell(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Shell>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        install_stub_menus(cx, false);
        let (window, shell) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(960.), px(640.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Shell::new(window, cx)),
        )
        .expect("shell window");
        shell.update(cx, |shell, cx| {
            shell.menus.update(cx, |menus, cx| menus.reload(cx));
        });
        (window, shell)
    })
}

#[gpui_kit::test]
fn menus_follow_selection(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    let idle = cx.update(|cx| menu_flags(cx));
    assert_eq!(idle[0], ("Import".into(), true, false));
    assert_eq!(idle[2], ("Toolbar".into(), false, false));
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("select-vm", cx);
    })
    .expect("click");
    let selected = cx.update(|cx| menu_flags(cx));
    assert_eq!(selected[0], ("Import".into(), false, false));
    assert_eq!(selected[2], ("Toolbar".into(), false, true));
    let _ = shell;
}

#[gpui_kit::test]
fn tabs_dialog_wizard_and_alert(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tab-0").is_some());
        window.click("open-rename", cx);
        window.input("", cx);
        window.click("save-name", cx);
    })
    .expect("empty save");
    assert!(shell.read_with(cx, |shell, _| shell.error));
    cx.update_window(window, |_, window, cx| {
        window.click("server-name", cx);
        window.input("lab", cx);
        window.click("save-name", cx);
    })
    .expect("save");
    assert!(shell.read_with(cx, |shell, _| shell.saved && !shell.error));
    cx.update_window(window, |_, window, cx| {
        window.press("escape", cx);
    })
    .expect("escape");
    cx.update_window(window, |_, window, cx| {
        window.click("open-wizard", cx);
        window.click("wizard-next", cx);
        window.click("wizard-next", cx);
    })
    .expect("wizard");
    assert_eq!(shell.read_with(cx, |shell, _| shell.step), 2);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("review").is_some());
        window.press("escape", cx);
        window.click("open-alert", cx);
    })
    .expect("alert");
    cx.update_window(window, |_, window, cx| {
        let ok = window.try_find_by_label("OK");
        assert!(ok.is_some(), "the alert should expose an OK label");
        if let Some(ok) = ok {
            let center = ok.bounds().center();
            window.pointer_down(center, gpui_kit::MouseButton::Left, cx);
            window.pointer_up(center, gpui_kit::MouseButton::Left, cx);
        }
    })
    .expect("confirm");
    assert!(shell.read_with(cx, |shell, _| shell.confirmed.get()));
}

#[gpui_kit::test]
fn keyboard_reaches_tree_table_and_a_button(cx: &mut TestAppContext) {
    let (window, shell) = open_shell(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        shell.update(cx, |shell, cx| {
            shell.tree.update(cx, |tree, cx| tree.focus(window, cx));
        });
        let before = shell.read_with(cx, |shell, cx| shell.tree.read(cx).selected_index());
        window.press("down", cx);
        let after = shell.read_with(cx, |shell, cx| shell.tree.read(cx).selected_index());
        assert_ne!(before, after, "down should move the tree selection");
        shell.update(cx, |shell, cx| {
            shell.table.update(cx, |table, cx| {
                table.focus_handle(cx).focus(window, cx);
            });
        });
        let before = shell.read_with(cx, |shell, cx| shell.table.read(cx).selected_row());
        window.press("down", cx);
        let after = shell.read_with(cx, |shell, cx| shell.table.read(cx).selected_row());
        assert_ne!(before, after, "down should move the table selection");
        window.focus_next(cx);
        window.press("tab", cx);
        window.press("enter", cx);
    })
    .expect("keys");
}

#[gpui_kit::test]
fn file_prompts_use_the_test_platform(cx: &mut TestAppContext) {
    let mut receiver =
        cx.update(|cx| cx.prompt_for_new_path(std::path::Path::new("/tmp"), Some("guest.xva")));
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(PathBuf::from("/tmp/guest.xva")));
    let chosen = receiver
        .try_recv()
        .expect("prompt completed")
        .expect("a value")
        .expect("prompt ok")
        .expect("a path");
    assert_eq!(chosen, PathBuf::from("/tmp/guest.xva"));

    let mut opened = cx.update(|cx| {
        cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        })
    });
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![PathBuf::from("/tmp/pool.xml")]));
    let paths = opened
        .try_recv()
        .expect("open prompt completed")
        .expect("a value")
        .expect("open prompt ok")
        .expect("paths");
    assert_eq!(paths, vec![PathBuf::from("/tmp/pool.xml")]);
}

#[gpui_kit::test]
fn light_and_dark_themes_and_rem_size(cx: &mut TestAppContext) {
    let (window, _) = open_shell(cx);
    cx.update(|cx| {
        Theme::change(ThemeMode::Light, None, cx);
        assert!(!Theme::global(cx).is_dark());
        Theme::change(ThemeMode::Dark, None, cx);
        assert!(Theme::global(cx).is_dark());
    });
    let before = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.set_rem_size(px(16.));
            window.render_frame(cx);
            window.find("shell").bounds().size.height
        })
        .expect("base rem");
    cx.simulate_window_scale_factor_change(window, 1.25);
    let after = cx
        .update_window(window, |_, window, cx| {
            window.set_rem_size(px(20.));
            window.render_frame(cx);
            window.find("shell").bounds().size.height
        })
        .expect("scaled rem");
    let _ = (before, after);
}
