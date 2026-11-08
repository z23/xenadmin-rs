use super::*;
use xenadmin_core::{ClassAvailability, VmSettingEdit};

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
    rt.update(|_, tx| tx.scroll_into_view(key));
    settle(rt);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}
fn select_two(rt: &mut rune::Runtime<State>) {
    let first = rt.state.rows[0].id.clone();
    click(rt, &format!("inventory-name-{first}"));
    rt.key(rune::KeyEvent {
        key: rune::Key::Down,
        mods: rune::Mods {
            shift: true,
            ..Default::default()
        },
    });
    settle(rt);
    assert_eq!(selected(&rt.state).unwrap().len(), 2);
}
fn target(s: &mut State, template: bool) -> VmId {
    demo(s);
    let id = s
        .inventory
        .as_ref()
        .unwrap()
        .vms
        .values()
        .find(|v| !v.is_template)
        .unwrap()
        .id;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let vm = i.vms.get_mut(&id).unwrap();
    vm.is_template = template;
    vm.power_state = PowerState::Halted;
    let d = i.details.vms.get_mut(&id).unwrap();
    d.is_default_template = Some(false);
    d.current_operations.clear();
    d.allowed_operations = Some(
        [
            "changing_VCPUs_live",
            "changing_dynamic_range",
            "changing_shadow_memory",
            "changing_shadow_memory_live",
        ]
        .map(str::to_owned)
        .into(),
    );
    d.blocked_operations.clear();
    d.vcpus_max = Some(8);
    d.vcpus_at_startup = Some(2);
    d.memory_static_min = Some(512 << 20);
    d.memory_dynamic_min = Some(1 << 30);
    d.memory_dynamic_max = Some(2 << 30);
    d.memory_static_max = Some(4 << 30);
    d.affinity_known = true;
    d.affinity = None;
    d.domain_type = Some("hvm".into());
    d.boot_policy = Some("BIOS order".into());
    d.boot_order = Some("cd".into());
    d.topology_known = true;
    d.cores_per_socket = Some("1".into());
    d.cpu_weight_known = true;
    d.cpu_weight = None;
    d.shadow_multiplier = Some("1".into());
    d.auto_power_on = Some(false);
    d.firmware = Some("uefi".into());
    d.secure_boot = Some(false);
    i.details
        .classes
        .insert("vbd".into(), ClassAvailability::Available);
    i.details.vbds.retain(|_, v| v.vm != Some(id));
    s.page = if template {
        Page::Templates
    } else {
        Page::Guests
    };
    s.rebuild();
    s.table.select(id.to_string());
    open(s);
    id
}
fn property(s: &mut State, p: Property, values: &[&str]) {
    choose(s, p);
    for (field, value) in edit(s).values.iter_mut().zip(values) {
        *field = TextEdit::new(*value);
    }
}

#[test]
fn property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut rt = runtime(size);
        select_two(&mut rt);
        click(&mut rt, "batch-properties-open");
        rt.update(|s, _| edit(s).values[0] = TextEdit::new("Common reviewed description"));
        settle(&mut rt);
        click(&mut rt, "batch-properties-review");
        let reviewed = rt
            .state
            .bulk_properties
            .as_ref()
            .unwrap()
            .review
            .clone()
            .unwrap();
        assert_eq!(reviewed.batch.vms.len(), 2);
        assert!(rt.find("batch-properties-submit").is_some());
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Session permissions are unavailable")
        );
        rt.update(|s, _| {
            let other = s.rows[5].id.clone();
            s.table.select(other);
            choose(s, Property::Name);
        });
        assert_eq!(
            rt.state.bulk_properties.as_ref().unwrap().property,
            Property::Description
        );
        assert!(rt.state.bulk_properties.as_ref().unwrap().review.as_ref() == Some(&reviewed));
        click(&mut rt, "batch-properties-submit");
        assert!(rt.state.resource_operations.is_empty());
        rt.update(|s, _| {
            Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into()
        });
        settle(&mut rt);
        assert!(proposed(&rt.state, rt.state.bulk_properties.as_ref().unwrap()).is_err());
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Connection changed")
        );
    }
}

#[test]
fn property_batch_names_every_unchanged_target_and_refuses_all_noop() {
    let mut rt = runtime((513.0, 652.0));
    select_two(&mut rt);
    click(&mut rt, "batch-properties-open");
    let ids = rt
        .state
        .bulk_properties
        .as_ref()
        .unwrap()
        .targets
        .iter()
        .map(|v| v.id)
        .collect::<Vec<_>>();
    rt.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .get_mut(&ids[0])
            .unwrap()
            .description = "desired".into();
        edit(s).values[0] = TextEdit::new("desired");
    });
    click(&mut rt, "batch-properties-review");
    let r = rt
        .state
        .bulk_properties
        .as_ref()
        .unwrap()
        .review
        .as_ref()
        .unwrap();
    assert_eq!(r.batch.vms.len(), 1);
    assert_eq!(r.batch.vms[0].vm, ids[1]);
    assert_eq!(r.rows.len(), 2);
    let a = rt.accessibility_json().to_string();
    assert!(a.contains("Already desired"));
    assert!(a.contains(&ids[0].to_string()));
    click(&mut rt, "batch-properties-back");
    rt.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .get_mut(&ids[1])
            .unwrap()
            .description = "desired".into();
        review(s);
    });
    assert!(rt.state.bulk_properties.as_ref().unwrap().review.is_none());
    assert!(
        rt.state
            .bulk_properties
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("All selected")
    );
}

#[test]
fn property_batch_missing_unsupported_and_changed_targets_block_whole_review() {
    let mut rt = runtime((1440.0, 900.0));
    select_two(&mut rt);
    click(&mut rt, "batch-properties-open");
    let ids = rt
        .state
        .bulk_properties
        .as_ref()
        .unwrap()
        .targets
        .iter()
        .map(|v| v.id)
        .collect::<Vec<_>>();
    rt.update(|s, _| {
        property(s, Property::Weight, &["512"]);
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .vms
            .get_mut(&ids[1])
            .unwrap()
            .cpu_weight_known = false;
        review(s);
    });
    let e = rt.state.bulk_properties.as_ref().unwrap();
    assert!(e.review.is_none());
    assert!(e.error.as_ref().unwrap().contains(&ids[1].to_string()));
    rt.update(|s, _| {
        property(s, Property::Description, &["desired"]);
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .vms
            .remove(&ids[0]);
        review(s);
    });
    assert!(
        rt.state
            .bulk_properties
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("unknown")
    );
    rt.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .get_mut(&ids[1])
            .unwrap()
            .power_state = PowerState::Unknown("fixture".into());
        review(s);
    });
    let e = rt.state.bulk_properties.as_ref().unwrap();
    assert!(e.error.as_ref().unwrap().contains("power state changed"));
    assert!(e.review.is_none());
}

#[test]
fn property_batch_all_existing_property_kinds_bind_per_target_previous_values() {
    let mut s = init();
    let id = target(&mut s, false);
    let cases = [
        (Property::Name, vec!["New name"]),
        (Property::Description, vec!["New description"]),
        (Property::Cpus, vec!["6", "3"]),
        (Property::Memory, vec!["512 MiB", "2 GiB", "3 GiB", "4 GiB"]),
        (Property::Topology, vec!["2"]),
        (Property::Weight, vec!["512"]),
        (Property::Shadow, vec!["2"]),
        (Property::BootOrder, vec!["dc"]),
        (Property::Autostart, vec![]),
        (Property::SecureBoot, vec![]),
        (Property::HomeHost, vec![]),
    ];
    for (p, values) in cases {
        property(&mut s, p, &values);
        edit(&mut s).toggle = true;
        if p == Property::HomeHost {
            let host = s.inventory.as_ref().unwrap().hosts.keys().next().copied();
            edit(&mut s).host = host;
        }
        let r = proposed(&s, s.bulk_properties.as_ref().unwrap())
            .unwrap_or_else(|error| panic!("{p:?}: {error}"));
        assert_eq!(r.batch.vms.len(), 1, "{p:?}");
        assert_eq!(r.batch.vms[0].vm, id);
        assert_eq!(r.batch.vms[0].patch.edits().len(), 1);
        assert!(r.rows[0].summary.contains('→'));
    }
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .vms
        .get_mut(&id)
        .unwrap()
        .power_state = PowerState::Running;
    edit(&mut s).targets[0].power_state = PowerState::Running;
    property(&mut s, Property::LiveCpus, &["4"]);
    let r = proposed(&s, s.bulk_properties.as_ref().unwrap()).unwrap();
    assert!(matches!(
        r.batch.vms[0].patch.edits(),
        [VmSettingEdit::LiveCpus {
            previous: 2,
            max: 8,
            value: 4
        }]
    ));
    property(&mut s, Property::DynamicMemory, &["2 GiB", "3 GiB"]);
    assert_eq!(
        proposed(&s, s.bulk_properties.as_ref().unwrap())
            .unwrap()
            .batch
            .vms
            .len(),
        1
    );
    let id = target(&mut s, true);
    property(&mut s, Property::Firmware, &[]);
    edit(&mut s).toggle = false;
    let r = proposed(&s, s.bulk_properties.as_ref().unwrap()).unwrap();
    assert!(r.batch.vms[0].patch.is_template());
    assert!(
        matches!(r.batch.vms[0].patch.edits(),[VmSettingEdit::Firmware{previous,value}]if previous=="uefi"&&value=="bios")
    );
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .vms
        .get_mut(&id)
        .unwrap()
        .is_default_template = Some(true);
    assert!(
        proposed(&s, s.bulk_properties.as_ref().unwrap())
            .unwrap_err()
            .contains("built-in")
    );
}

#[test]
fn property_batch_live_limits_capabilities_and_stale_review_refuse_submission() {
    let mut s = init();
    let id = target(&mut s, false);
    property(&mut s, Property::LiveCpus, &["4"]);
    assert!(proposed(&s, s.bulk_properties.as_ref().unwrap()).is_err());
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .vms
        .get_mut(&id)
        .unwrap()
        .power_state = PowerState::Running;
    edit(&mut s).targets[0].power_state = PowerState::Running;
    property(&mut s, Property::LiveCpus, &["1"]);
    assert!(proposed(&s, s.bulk_properties.as_ref().unwrap()).is_err());
    property(&mut s, Property::LiveCpus, &["9"]);
    assert!(proposed(&s, s.bulk_properties.as_ref().unwrap()).is_err());
    property(&mut s, Property::DynamicMemory, &["1 GiB", "8 GiB"]);
    assert!(proposed(&s, s.bulk_properties.as_ref().unwrap()).is_err());
    property(&mut s, Property::LiveCpus, &["4"]);
    review(&mut s);
    assert!(s.bulk_properties.as_ref().unwrap().review.is_some());
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .vms
        .get_mut(&id)
        .unwrap()
        .vcpus_at_startup = Some(3);
    assert!(
        proposed(&s, s.bulk_properties.as_ref().unwrap()).as_ref()
            != s.bulk_properties
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .ok_or(&String::new())
    );
    submit(&mut s);
    assert!(s.resource_operations.is_empty());
    property(&mut s, Property::LiveCpus, &["4"]);
    Arc::make_mut(s.inventory.as_mut().unwrap())
        .details
        .vms
        .get_mut(&id)
        .unwrap()
        .blocked_operations
        .insert(
            "changing_VCPUs_live".into(),
            "administrative restriction".into(),
        );
    assert!(proposed(&s, s.bulk_properties.as_ref().unwrap()).is_err());
}

#[test]
fn property_batch_picker_and_template_entry_work_with_keyboard_menu_capture() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut rt = runtime(size);
        rt.update(|s, _| {
            target(s, true);
            s.bulk_properties = None;
        });
        settle(&mut rt);
        click(&mut rt, "batch-properties-open");
        click(&mut rt, "batch-property-picker");
        click(&mut rt, "batch-property-Firmware");
        assert_eq!(
            rt.state.bulk_properties.as_ref().unwrap().property,
            Property::Firmware
        );
        click(&mut rt, "batch-properties-close");
        let id = rt.state.table.cursor().unwrap().to_string();
        click(&mut rt, &format!("inventory-name-{id}"));
        rt.key(rune::Key::ContextMenu.into());
        settle(&mut rt);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Edit properties for selection")
        );
        click(&mut rt, "inventory-context-item2");
        assert!(rt.state.bulk_properties.is_some());
    }
    let mut rt = runtime((1440.0, 900.0));
    select_two(&mut rt);
    rt.key(rune::Key::ContextMenu.into());
    settle(&mut rt);
    rt.update(|s, _| s.table.select(s.rows[4].id.clone()));
    click(&mut rt, "inventory-context-item6");
    assert!(rt.state.bulk_properties.is_none());
}
