use super::*;

fn settle(rt: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    demo(&mut s);
    s.window_width = size.0;
    s.window_height = size.1;
    let mut rt = rune::Runtime::new(
        s,
        super::super::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut rt);
    rt
}
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}
fn right(rt: &mut rune::Runtime<State>, x: f32, y: f32) {
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    settle(rt);
}
fn row(rt: &rune::Runtime<State>, id: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| {
            e.role == "row" && e.key.as_deref() == Some(id) && e.path.contains("inventory-table")
        })
        .unwrap()
        .visible
}

#[test]
fn right_click_any_column_or_padding_targets_the_visible_row_at_both_widths() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(size);
        let id = rt.state.rows[2].id.clone();
        let [x, y, w, h] = row(&rt, &id);
        for offset in [1.0, w * 0.45, w - 1.0] {
            right(&mut rt, x + offset, y + h / 2.0);
            assert_eq!(
                rt.state.row_menu.as_ref().map(|c| c.id.as_str()),
                Some(id.as_str()),
                "at {size:?} offset {offset}"
            );
            click(&mut rt, "inventory-context-item0");
            assert_eq!(rt.state.table.cursor(), Some(id.as_str()));
            assert!(rt.state.show_details);
            rt.update(|s, _| s.show_details = false);
            settle(&mut rt);
        }
    }
}

#[test]
fn menu_key_and_shift_f10_preserve_keyboard_selection_and_open_current_row() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(size);
        let id = rt.state.rows[0].id.clone();
        click(&mut rt, &format!("inventory-name-{id}"));
        rt.key(rune::KeyEvent {
            key: rune::Key::Down,
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        });
        settle(&mut rt);
        let cursor = rt.state.table.cursor().unwrap().to_owned();
        let selection = selected(&rt.state);
        assert_eq!(selection.len(), 2);
        for event in [
            rune::Key::ContextMenu.into(),
            rune::KeyEvent {
                key: rune::Key::Function(10),
                mods: rune::Mods {
                    shift: true,
                    ..Default::default()
                },
            },
        ] {
            rt.key(event);
            settle(&mut rt);
            let menu = rt.state.row_menu.as_ref().expect("keyboard menu");
            assert_eq!(menu.id, cursor);
            assert_eq!(menu.selection, selection);
            rt.key(rune::Key::Escape.into());
            settle(&mut rt);
            assert!(rt.state.row_menu.is_none());
        }
        rt.key(rune::Key::Down.into());
        settle(&mut rt);
        assert_ne!(rt.state.table.cursor(), Some(cursor.as_str()));
    }
}

#[test]
fn header_and_blank_space_do_not_invent_a_row_target() {
    let mut rt = runtime((1440.0, 900.0));
    rt.update(|s, _| s.rows.truncate(2));
    settle(&mut rt);
    let header = rt
        .elements()
        .into_iter()
        .find(|e| {
            e.role == "columnheader"
                && e.path.contains("inventory-table")
                && e.key.as_deref() == Some("name")
        })
        .unwrap();
    right(&mut rt, header.rect[0] + 5.0, header.rect[1] + 10.0);
    assert!(rt.state.row_menu.is_none());
    let viewport = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("viewport") && e.path.contains("inventory-table"))
        .unwrap();
    right(
        &mut rt,
        viewport.visible[0] + 10.0,
        viewport.visible[1] + viewport.visible[3] - 5.0,
    );
    assert!(rt.state.row_menu.is_none());
}

#[test]
fn stale_menu_cannot_apply_to_a_removed_row_or_changed_pool() {
    let mut rt = runtime((1440.0, 900.0));
    let first = rt.state.rows[0].id.clone();
    let second = rt.state.rows[1].id.clone();
    let [x, y, _, h] = row(&rt, &first);
    right(&mut rt, x + 5.0, y + h / 2.0);
    rt.update(|s, _| s.rows.retain(|r| r.id != first));
    settle(&mut rt);
    assert!(!valid(&rt.state, rt.state.row_menu.as_ref().unwrap()));
    rt.update(|s, _| assert!(!target(s)));
    settle(&mut rt);
    let [x, y, _, h] = row(&rt, &second);
    right(&mut rt, x + 5.0, y + h / 2.0);
    rt.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::from_u128(999).into();
    });
    assert!(!valid(&rt.state, rt.state.row_menu.as_ref().unwrap()));
    rt.update(|s, _| assert!(!target(s)));
}

#[test]
fn keyboard_menu_follows_virtual_scrolling_and_changed_selection_cannot_start_a_batch() {
    let mut rt = runtime((1440.0, 900.0));
    let first = rt.state.rows[0].id.clone();
    click(&mut rt, &format!("inventory-name-{first}"));
    rt.key(rune::Key::End.into());
    settle(&mut rt);
    let last = rt.state.table.cursor().unwrap().to_owned();
    assert_ne!(last, first);
    rt.key(rune::Key::ContextMenu.into());
    settle(&mut rt);
    assert_eq!(rt.state.row_menu.as_ref().unwrap().id, last);
    rt.update(|s, _| s.table.select(first.clone()));
    settle(&mut rt);
    click(&mut rt, "inventory-context-item5");
    assert!(rt.state.bulk_power.is_none());
    assert!(rt.state.row_menu.is_none());
    assert_eq!(rt.state.table.cursor(), Some(first.as_str()));
}
