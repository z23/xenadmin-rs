use super::*;
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    rt.update(|_, tx| tx.scroll_into_view(key));
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}
fn runtime(size: (f32, f32)) -> rune::Runtime<State> {
    let mut s = init();
    demo(&mut s);
    s.window_width = size.0;
    s.window_height = size.1;
    rune::Runtime::new(
        s,
        super::super::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    )
}
#[test]
fn multi_selection_survives_inventory_refresh_and_reviews_captured_vms_at_both_sizes() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(size);
        rt.update(|s, _| s.rows.retain(|r| r.state == "Running"));
        let first = rt.state.rows[0].id.clone();
        click(&mut rt, &format!("inventory-name-{first}"));
        rt.key(rune::KeyEvent {
            key: rune::Key::Down,
            mods: rune::Mods {
                shift: true,
                ..Default::default()
            },
        });
        rt.frame(0.0);
        let selected = selection(&rt.state);
        assert_eq!(selected.len(), 2, "multi selection at {size:?}");
        rt.update(|s, _| s.rebuild());
        assert_eq!(selection(&rt.state).len(), 2);
        click(&mut rt, "batch-power-open");
        click(&mut rt, "batch-power-review");
        let captured = rt
            .state
            .bulk_power
            .as_ref()
            .unwrap()
            .review
            .clone()
            .expect("review");
        assert_eq!(
            captured.vms.iter().map(|v| v.vm).collect::<Vec<_>>(),
            selected.iter().map(|v| v.id).collect::<Vec<_>>()
        );
        rt.update(|s, _| {
            let other = s.rows[3].id.clone();
            s.table.select(other);
        });
        assert_eq!(
            rt.state
                .bulk_power
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap(),
            &captured
        );
        click(&mut rt, "batch-power-submit");
        assert!(rt.state.resource_operations.is_empty());
        rt.update(|s, _| {
            let id = captured.vms[0].vm;
            let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
            inventory.vms.get_mut(&id).unwrap().power_state = PowerState::Unknown("changed".into());
        });
        let e = rt.state.bulk_power.as_ref().unwrap();
        assert!(!unavailable(&rt.state, e, e.action).is_empty());
    }
}
#[test]
fn context_menu_targets_clicked_row_and_keeps_non_vm_menus_relevant() {
    let mut rt = runtime((1440.0, 900.0));
    let other = rt.state.rows[2].id.clone();
    rt.update(|_, tx| tx.scroll_into_view(format!("inventory-name-{other}")));
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let (x, y) = rt.find(&format!("inventory-name-{other}")).unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(0.0);
    assert!(rt.state.row_menu.is_some());
    click(&mut rt, "inventory-context-item0");
    assert_eq!(rt.state.table.cursor(), Some(other.as_str()));
    assert!(rt.state.show_details);
    assert!(rt.state.row_menu.is_none());
    rt.update(|s, _| {
        s.page = Page::Networks;
        s.filter_rows();
    });
    let network = rt.state.rows[0].id.clone();
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    let (x, y) = rt.find(&format!("inventory-name-{network}")).unwrap();
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(0.0);
    assert!(rt.state.row_menu.is_some());
    assert!(rt.find("inventory-context-item2").is_none());
}
#[test]
fn batch_uses_only_visible_selection_and_rejects_over_limit_without_silent_subset() {
    let mut rt = runtime((1440.0, 900.0));
    let first = rt.state.rows[0].id.clone();
    click(&mut rt, &format!("inventory-name-{first}"));
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('a'),
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    });
    rt.frame(0.0);
    assert!(selection(&rt.state).len() > 32);
    rt.update(|s, _| open(s));
    assert!(rt.state.bulk_power.is_none());
    assert!(rt.state.error.as_deref().unwrap().contains("32"));
    rt.update(|s, _| {
        s.search = TextEdit::new(first);
        s.filter_rows();
    });
    assert_eq!(selection(&rt.state).len(), 1);
}
