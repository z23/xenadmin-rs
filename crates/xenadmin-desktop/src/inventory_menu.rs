use super::*;
#[cfg(test)]
#[path = "inventory_menu_tests.rs"]
mod tests;
#[derive(Clone, PartialEq, Eq)]
struct Scope {
    pool: PoolId,
    page: Page,
    observation: Option<xenadmin_core::ObservedResource>,
}
impl Scope {
    fn capture(s: &State) -> Option<Self> {
        let pool = s.inventory.as_ref()?.pool_id;
        Some(Self {
            pool,
            page: s.page,
            observation: s.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .observe_resource(xenadmin_core::ResourceTarget::Pool(pool))
                    .ok()
            }),
        })
    }
}
pub(super) struct Context {
    scope: Scope,
    id: String,
    selection: Vec<String>,
    point: rune::PointerPress,
}
fn selected(s: &State) -> Vec<String> {
    s.rows
        .iter()
        .filter(|r| s.table.is_selected(&r.id))
        .map(|r| r.id.clone())
        .collect()
}
fn valid(s: &State, context: &Context) -> bool {
    Scope::capture(s).as_ref() == Some(&context.scope) && s.rows.iter().any(|r| r.id == context.id)
}
fn open(s: &mut State, scope: Scope, id: String, point: rune::PointerPress) {
    if Scope::capture(s).as_ref() != Some(&scope) || !s.rows.iter().any(|r| r.id == id) {
        return;
    }
    if !s.table.is_selected(&id) {
        s.table.select(id.clone());
    }
    s.row_menu = Some(Context {
        scope,
        id,
        selection: selected(s),
        point,
    });
}
pub(super) fn cell(_s: &State, r: &DisplayRow, font: rune::TextStyle, ink: rune::Color) -> Cell {
    Cell::el(
        text(r.name.clone())
            .font(font)
            .color(ink)
            .help_when_truncated(r.name.clone())
            .key(format!("inventory-name-{}", r.id))
            .truncate()
            .w_full(),
    )
}
pub(super) fn attach(s: &State, table: El) -> El {
    let Some(scope) = Scope::capture(s) else {
        return table;
    };
    let pointer_scope = scope.clone();
    let table = table.on_secondary_press(move |_s: &mut State, point, tx| {
        let scope = pointer_scope.clone();
        tx.element_at_in(
            "inventory-table",
            point.x,
            point.y,
            move |s: &mut State, picked, _| {
                let Some(picked) = picked else {
                    return;
                };
                let Some(id) = s
                    .rows
                    .iter()
                    .find(|r| picked.path().split('/').any(|part| part == r.id))
                    .map(|r| r.id.clone())
                else {
                    return;
                };
                open(s, scope, id, point);
            },
        );
    });
    let cursor = s.table.cursor().map(str::to_owned);
    let point = cursor
        .as_ref()
        .and_then(|id| rune::memo::window_layout(&format!("inventory-name-{id}")))
        .map(|[x, y, w, h]| rune::PointerPress {
            x: x + w.min(32.0) * 0.5,
            y: y + h,
        });
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .key("inventory-menu-keyboard")
        .on_key(move |s: &mut State, event: &rune::KeyEvent, _| {
            let shortcut = event.key == rune::Key::ContextMenu
                || event.key == rune::Key::Function(10) && event.mods.shift;
            if !shortcut || event.mods.alt || event.mods.ctrl || event.mods.cmd || event.mods.logo {
                return false;
            }
            if let (Some(id), Some(point)) = (&cursor, point)
                && s.table.cursor() == Some(id.as_str())
            {
                open(s, scope.clone(), id.clone(), point);
            }
            true
        })
        .child(table)
}
fn target(s: &mut State) -> bool {
    let Some(context) = s.row_menu.take() else {
        return false;
    };
    if !valid(s, &context) {
        return false;
    }
    console_view::release(s);
    s.console = None;
    s.devices = None;
    s.snapshot_view = None;
    s.metrics = None;
    s.table.select(context.id);
    true
}
pub(super) fn overlay(s: &State, k: &Kit, base: El) -> El {
    let Some(context) = &s.row_menu else {
        return base;
    };
    let current = valid(s, context);
    let vm = current && context.scope.page == Page::Guests;
    let mut layers = Vec::new();
    let mut menu = rune_kit::context_menu("inventory-context", base, Some(context.point))
        .min_width(280.0)
        .on_close(|s: &mut State| s.row_menu = None)
        .item(
            rune_kit::MenuItem::new("Details")
                .disabled(!current)
                .on_select(|s: &mut State| {
                    if target(s) {
                        s.show_details = true;
                    }
                }),
        )
        .item(
            rune_kit::MenuItem::new("Copy UUID")
                .disabled(!current)
                .on_select_tx(|s: &mut State, tx| {
                    if let Some(context) = s.row_menu.take()
                        && valid(s, &context)
                    {
                        tx.copy(context.id);
                    }
                }),
        );
    if vm {
        menu = menu
            .item(
                rune_kit::MenuItem::new("Properties…").on_select(|s: &mut State| {
                    if target(s) {
                        vm_properties::open(s);
                    }
                }),
            )
            .item(
                rune_kit::MenuItem::new("Disks and network interfaces…").on_select(
                    |s: &mut State| {
                        if target(s) {
                            vm_devices::open(s);
                        }
                    },
                ),
            )
            .item(
                rune_kit::MenuItem::new("Snapshots…").on_select(|s: &mut State| {
                    if target(s) {
                        vm_snapshots::open(s);
                    }
                }),
            )
            .item(
                rune_kit::MenuItem::new("Power actions for selection…").on_select(
                    |s: &mut State| {
                        if let Some(context) = s.row_menu.take()
                            && valid(s, &context)
                            && selected(s) == context.selection
                        {
                            bulk_power::open(s);
                        }
                    },
                ),
            );
    }
    if current && matches!(context.scope.page, Page::Guests | Page::Templates) {
        menu = menu.item(
            rune_kit::MenuItem::new("Edit properties for selection…").on_select(|s: &mut State| {
                if let Some(context) = s.row_menu.take()
                    && valid(s, &context)
                    && selected(s) == context.selection
                {
                    bulk_properties::open(s);
                }
            }),
        );
    }
    if context.scope.page == Page::Guests {
        menu = menu.item(
            rune_kit::MenuItem::new("Move selection…")
                .disabled(!current)
                .on_select(|s: &mut State| {
                    if let Some(context) = s.row_menu.take()
                        && valid(s, &context)
                        && selected(s) == context.selection
                    {
                        bulk_move::open(s);
                    }
                }),
        );
        menu = menu.item(
            rune_kit::MenuItem::new("Move selection to another pool…")
                .disabled(!current)
                .on_select(|s: &mut State| {
                    if let Some(context) = s.row_menu.take()
                        && valid(s, &context)
                        && selected(s) == context.selection
                    {
                        bulk_cross_pool::open(s);
                    }
                }),
        );
    }
    if matches!(context.scope.page, Page::Guests | Page::Templates) {
        menu = menu.item(
            rune_kit::MenuItem::new("Export selection…")
                .disabled(!current)
                .on_select(|s: &mut State| {
                    if let Some(context) = s.row_menu.take()
                        && valid(s, &context)
                        && selected(s) == context.selection
                    {
                        bulk_export::open(s);
                    }
                }),
        );
        menu = menu.item(
            rune_kit::MenuItem::new("Delete selection…")
                .disabled(!current)
                .on_select(|s: &mut State| {
                    if let Some(context) = s.row_menu.take()
                        && valid(s, &context)
                        && selected(s) == context.selection
                    {
                        bulk_delete::open(s);
                    }
                }),
        );
    }
    let root = menu.build(k, &mut layers);
    rune_kit::root(
        root,
        layers
            .into_iter()
            .map(|layer| layer.focus_return("inventory-table")),
    )
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    if let (Some(scope), Some(id)) = (Scope::capture(s), s.rows.first().map(|r| r.id.clone())) {
        open(s, scope, id, rune::PointerPress { x: 360.0, y: 300.0 });
    }
}
