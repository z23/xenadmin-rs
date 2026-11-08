use super::*;
pub(super) struct Context {
    pool: PoolId,
    id: String,
    page: Page,
    point: rune::PointerPress,
}
pub(super) fn cell(s: &State, r: &DisplayRow, font: rune::TextStyle, ink: rune::Color) -> Cell {
    let Some(pool) = s.inventory.as_ref().map(|i| i.pool_id) else {
        return Cell::text(&r.name);
    };
    let id = r.id.clone();
    let page = s.page;
    Cell::el(
        text(r.name.clone())
            .font(font)
            .color(ink)
            .help_when_truncated(r.name.clone())
            .key(format!("inventory-name-{}", r.id))
            .truncate()
            .w_full()
            .on_secondary_press(move |s: &mut State, point, _| {
                if s.inventory.as_ref().is_none_or(|i| i.pool_id != pool)
                    || s.page != page
                    || !s.rows.iter().any(|r| r.id == id)
                {
                    return;
                }
                if !s.table.is_selected(&id) {
                    s.table.select(id.clone());
                }
                s.row_menu = Some(Context {
                    pool,
                    id: id.clone(),
                    page,
                    point,
                });
            }),
    )
}
fn target(s: &mut State) -> bool {
    let Some(context) = s.row_menu.take() else {
        return false;
    };
    if s.inventory
        .as_ref()
        .is_none_or(|i| i.pool_id != context.pool)
        || s.page != context.page
        || !s.rows.iter().any(|r| r.id == context.id)
    {
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
    let current = s
        .inventory
        .as_ref()
        .is_some_and(|i| i.pool_id == context.pool)
        && s.page == context.page;
    let vm = current && context.page == Page::Guests;
    let mut layers = Vec::new();
    let mut menu = rune_kit::context_menu("inventory-context", base, Some(context.point))
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
                    if let Some(context) = s.row_menu.take() {
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
                        s.row_menu = None;
                        bulk_power::open(s);
                    },
                ),
            );
    }
    let root = menu.build(k, &mut layers);
    rune_kit::root(root, layers)
}
