use super::*;
use xenadmin_core::{OperationRecord, OperationState, SnapshotAction, VbdId};

fn source_id() -> VmId {
    VmId::parse("30000000-0000-0000-0000-000000000000").unwrap()
}
fn snapshot_id() -> SnapshotId {
    SnapshotId::parse("60000000-0000-0000-0000-000000000000").unwrap()
}
fn state(template: bool) -> State {
    let mut s = init();
    demo(&mut s);
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    inventory.vms.get_mut(&source_id()).unwrap().is_template = template;
    let details = inventory.details.vms.get_mut(&source_id()).unwrap();
    details.is_default_template = Some(false);
    details.allowed_operations = Some(vec![
        "clone".into(),
        "copy".into(),
        "destroy".into(),
        "make_into_template".into(),
    ]);
    let mut snapshot = inventory.snapshots.get(&snapshot_id()).unwrap().clone();
    snapshot.allowed_actions.push(SnapshotAction::Delete);
    inventory.snapshots.insert(snapshot);
    let mut disk = inventory
        .details
        .vdis
        .values()
        .find(|disk| disk.kind == "user")
        .unwrap()
        .clone();
    disk.id = VdiId::parse("71000000-0000-0000-0000-000000000099").unwrap();
    disk.name = "Snapshot data".into();
    disk.is_snapshot = Some(true);
    let mut vbd = inventory
        .details
        .vbds
        .values()
        .find(|vbd| vbd.kind == "Disk")
        .unwrap()
        .clone();
    vbd.id = VbdId::parse("72000000-0000-0000-0000-000000000099").unwrap();
    vbd.vm = None;
    vbd.snapshot = Some(snapshot_id());
    vbd.vdi = Some(disk.id);
    inventory
        .details
        .vdi_attachments
        .insert(disk.id, vec![vbd.id]);
    inventory.details.vbds.insert(vbd.id, vbd);
    inventory.details.vdis.insert(disk.id, disk);
    s.page = if template {
        Page::Templates
    } else {
        Page::Guests
    };
    s.filter_rows();
    s.table.select(source_id().to_string());
    s
}
fn runtime(mut state: State, size: (f32, f32)) -> rune::Runtime<State> {
    state.window_width = size.0;
    state.window_height = size.1;
    let mut rt = rune::Runtime::new(
        state,
        view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut rt);
    rt
}
fn settle(rt: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}
fn click(rt: &mut rune::Runtime<State>, key: &str) {
    for prefix in [
        "vm-lifecycle-sr",
        "vm-lifecycle-network",
        "vm-lifecycle-iso",
        "vm-lifecycle-bios-host",
    ] {
        if key.starts_with(&format!("{prefix}-")) {
            rt.update(|_, tx| tx.scroll_into_view(format!("{prefix}-choices")));
            settle(rt);
        }
    }
    rt.update(|_, tx| tx.scroll_into_view(key));
    settle(rt);
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    settle(rt);
}
fn details(rt: &mut rune::Runtime<State>) {
    if rt.state.window_width < 1200.0 {
        click(rt, "show-details");
    }
}
fn reviewed(s: &State) -> &VmLifecycleOperation {
    &editor(s).unwrap().review.as_ref().unwrap().operation
}

#[test]
fn creation_wizard_reviews_typed_resources_and_keeps_its_source_at_both_widths() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(state(true), size);
        details(&mut rt);
        let sr = *rt
            .state
            .inventory
            .as_ref()
            .unwrap()
            .details
            .storage
            .keys()
            .next()
            .unwrap();
        let network = *rt
            .state
            .inventory
            .as_ref()
            .unwrap()
            .details
            .networks
            .keys()
            .next()
            .unwrap();
        click(&mut rt, "create-from-template");
        rt.update(|s, _| editor_mut(s).unwrap().name = TextEdit::new(""));
        click(&mut rt, "vm-lifecycle-name");
        rt.key(rune::Key::Text("New application VM".into()).into());
        rt.frame(0.0);
        click(&mut rt, "review-vm-lifecycle");
        assert_eq!(editor(&rt.state).unwrap().step, 1);
        click(&mut rt, "vm-lifecycle-firmware-uefi");
        click(&mut rt, "review-vm-lifecycle");
        assert_eq!(editor(&rt.state).unwrap().step, 2);
        click(&mut rt, &format!("vm-lifecycle-sr-{sr}"));
        click(&mut rt, &format!("vm-lifecycle-network-{network}"));
        click(&mut rt, "vm-lifecycle-disk-size");
        rt.key(rune::Key::Text("8 GiB".into()).into());
        rt.frame(0.0);
        click(&mut rt, "review-vm-lifecycle");
        assert!(
            matches!(reviewed(&rt.state),VmLifecycleOperation::Create{name,sr:selected_sr,network:selected_network,vcpus:4,memory,disk_size:Some(size),firmware:GuestFirmware::Uefi,iso:None,..} if name=="New application VM"&&*selected_sr==sr&&*selected_network==network&&*memory==2<<30&&*size==8<<30)
        );
        rt.update(|s, _| s.table.select("30000000-0000-0000-0000-000000000001"));
        rt.frame(0.0);
        assert_eq!(editor(&rt.state).unwrap().source.id, source_id());
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Retain inherited installation media")
        );
        click(&mut rt, "apply-vm-lifecycle");
        assert!(rt.state.operations.is_empty());
        assert!(editor(&rt.state).unwrap().review.is_some());
    }
}
#[test]
fn cloning_distinguishes_fast_and_full_copy_and_exact_destination() {
    for source_template in [false, true] {
        let mut rt = runtime(state(source_template), (513.0, 652.0));
        details(&mut rt);
        let sr = *rt
            .state
            .inventory
            .as_ref()
            .unwrap()
            .details
            .storage
            .keys()
            .next()
            .unwrap();
        click(&mut rt, "clone-vm");
        click(&mut rt, "review-vm-lifecycle");
        assert!(
            matches!(reviewed(&rt.state),VmLifecycleOperation::Clone{expected_template,mode:VmCopyMode::Fast,as_template,..}if *expected_template==source_template&&*as_template==source_template)
        );
        click(&mut rt, "back-vm-lifecycle");
        click(&mut rt, "vm-lifecycle-full-copy");
        click(&mut rt, "vm-lifecycle-as-template");
        click(&mut rt, &format!("vm-lifecycle-sr-{sr}"));
        click(&mut rt, "review-vm-lifecycle");
        assert!(
            matches!(reviewed(&rt.state),VmLifecycleOperation::Clone{mode:VmCopyMode::Full{sr:target},as_template,..}if *target==sr&&*as_template!=source_template)
        );
    }
}
#[test]
fn conversion_and_template_metadata_have_reviewed_captured_sources() {
    let mut rt = runtime(state(false), (1440.0, 900.0));
    click(&mut rt, "convert-vm-template");
    assert_eq!(
        reviewed(&rt.state),
        &VmLifecycleOperation::ConvertToTemplate
    );
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("no longer appear as a runnable VM")
    );
    let mut rt = runtime(state(true), (513.0, 652.0));
    details(&mut rt);
    click(&mut rt, "edit-template");
    assert!(operation(editor(&rt.state).unwrap()).is_err());
    rt.update(|s, _| editor_mut(s).unwrap().name = TextEdit::new("Renamed template"));
    click(&mut rt, "review-vm-lifecycle");
    assert!(
        matches!(reviewed(&rt.state),VmLifecycleOperation::EditTemplate{name,..}if name=="Renamed template")
    );
}
#[test]
fn deletion_defaults_record_only_and_captures_exact_selected_data_and_snapshots() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(state(false), size);
        details(&mut rt);
        click(&mut rt, "delete-vm");
        click(&mut rt, "review-vm-lifecycle");
        assert!(
            matches!(reviewed(&rt.state),VmLifecycleOperation::Delete{disks,snapshots,..}if disks.is_empty()&&snapshots.is_empty())
        );
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Record-only deletion")
        );
        click(&mut rt, "back-vm-lifecycle");
        let disk = rt
            .state
            .inventory
            .as_ref()
            .unwrap()
            .vm_deletion_disks(source_id())[0];
        let snapshot_disk = rt
            .state
            .inventory
            .as_ref()
            .unwrap()
            .snapshot_deletion_disks(snapshot_id())[0];
        click(&mut rt, &format!("vm-delete-disk-{disk}"));
        click(&mut rt, &format!("vm-delete-snapshot-{}", snapshot_id()));
        click(&mut rt, &format!("vm-delete-snapshot-disk-{snapshot_disk}"));
        click(&mut rt, "review-vm-lifecycle");
        assert_eq!(
            reviewed(&rt.state),
            &VmLifecycleOperation::Delete {
                expected_template: false,
                disks: vec![disk],
                snapshots: vec![SnapshotDeletionSelection {
                    snapshot: snapshot_id(),
                    disks: vec![snapshot_disk]
                }]
            }
        );
        assert!(rt.accessibility_json().to_string().contains("Consumer:"));
        rt.update(|s, _| {
            let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
            inventory
                .details
                .vbds
                .values_mut()
                .find(|vbd| vbd.vdi == Some(disk))
                .unwrap()
                .owned = false;
        });
        rt.frame(0.0);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("no longer available")
        );
        click(&mut rt, "apply-vm-lifecycle");
        assert!(rt.state.operations.is_empty());
    }
}
#[test]
fn invalid_resources_and_stale_or_changed_sources_cannot_submit() {
    let mut s = state(true);
    open(&mut s, source_id(), Kind::Create);
    for invalid in ["0", "257", "-1", "1.5"] {
        editor_mut(&mut s).unwrap().vcpus = TextEdit::new(invalid);
        assert!(operation(editor(&s).unwrap()).is_err());
    }
    editor_mut(&mut s).unwrap().vcpus = TextEdit::new("2");
    for invalid in ["0", "1 KiB", "1025", "-1", "18446744073709551615 GiB"] {
        editor_mut(&mut s).unwrap().memory = TextEdit::new(invalid);
        assert!(operation(editor(&s).unwrap()).is_err());
    }
    let mut rt = runtime(state(false), (513.0, 652.0));
    details(&mut rt);
    click(&mut rt, "convert-vm-template");
    rt.update(|s, _| {
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id =
            PoolId::parse("10000000-0000-0000-0000-000000000099").unwrap()
    });
    rt.frame(0.0);
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("Cancel and reopen")
    );
    apply(&mut rt.state);
    assert!(rt.state.operations.is_empty());
}
fn record(index: u128, state: OperationState) -> OperationRecord {
    serde_json::from_value(serde_json::json!({"id":uuid::Uuid::from_u128(0x81000000000000000000000000000000+index),"pool":"10000000-0000-0000-0000-000000000001","vm":source_id(),"action":VmLifecycleOperation::ConvertToTemplate,"state":state,"progress":null,"detail":"Synthetic lifecycle outcome","created_at_ms":index as u64,"created_vm":"30000000-0000-0000-0000-000000000099","created_devices":[xenadmin_core::CreatedDevice::Disk(VdiId::parse("71000000-0000-0000-0000-000000000088").unwrap())]})).unwrap()
}
#[test]
fn global_history_survives_deleted_source_and_is_reachable_offline_at_compact_width() {
    let mut s = state(false);
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    let mut created = inventory.vms.remove(&source_id()).unwrap();
    created.id = VmId::parse("30000000-0000-0000-0000-000000000099").unwrap();
    let created_id = created.id;
    created.name = "Partially created VM".into();
    inventory.vms.insert(created.id, created);
    s.filter_rows();
    s.operation_rows = vec![record(1, OperationState::OutcomeUnknown)];
    let mut rt = runtime(s, (480.0, 652.0));
    let (x, y) = rt.find("vm-lifecycle-history").unwrap();
    assert!((0.0..480.0).contains(&x) && (0.0..652.0).contains(&y));
    click(&mut rt, "vm-lifecycle-history");
    let text = rt.accessibility_json().to_string();
    assert!(text.contains("Created VM/template UUID"));
    assert!(text.contains("71000000-0000-0000-0000-000000000088"));
    assert!(text.contains("OutcomeUnknown"));
    click(&mut rt, "close-vm-lifecycle-history");
    rt.update(|s, _| {
        s.table.select(created_id.to_string());
        s.show_details = true;
    });
    rt.frame(0.0);
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("Synthetic lifecycle outcome")
    );
    click(&mut rt, "vm-lifecycle-history");
    rt.update(|s, _| {
        s.connection = None;
        s.inventory = None;
        s.snapshot = None;
        s.demo = false;
    });
    rt.frame(0.0);
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("Created VM/template UUID")
    );
}
#[test]
fn shared_history_retains_unresolved_records_and_bounds_terminal_records() {
    let mut rows = vec![record(1, OperationState::OutcomeUnknown)];
    rows.extend((2..112).map(|i| record(i, OperationState::Succeeded)));
    let retained = merge_operation_rows(&rows, Vec::new());
    assert_eq!(retained.len(), 101);
    assert_eq!(retained[0].id, rows[0].id);
    assert_eq!(merge_operation_rows(&retained, Vec::new()), retained);
    let rejected = record(111, OperationState::Rejected);
    let merged = merge_operation_rows(&retained, vec![rejected.clone()]);
    assert_eq!(
        merged
            .iter()
            .find(|row| row.id == rejected.id)
            .unwrap()
            .state,
        OperationState::Rejected
    );
}

#[test]
fn installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_edited_or_deleted() {
    let mut s = state(true);
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    let mut iso = inventory.details.vdis.values().next().unwrap().clone();
    iso.id = VdiId::parse("71000000-0000-0000-0000-000000000077").unwrap();
    iso.name = "Installer ISO".into();
    iso.kind = "iso".into();
    iso.read_only = Some(true);
    iso.is_snapshot = Some(false);
    let iso_id = iso.id;
    inventory.details.vdis.insert(iso.id, iso);
    let sr = *inventory.details.storage.keys().next().unwrap();
    let network = *inventory.details.networks.keys().next().unwrap();
    inventory
        .details
        .vms
        .get_mut(&source_id())
        .unwrap()
        .is_default_template = Some(true);
    let mut rt = runtime(s, (513.0, 652.0));
    details(&mut rt);
    click(&mut rt, "edit-template");
    assert!(rt.state.vm_lifecycle.is_none());
    click(&mut rt, "delete-vm");
    assert!(rt.state.vm_lifecycle.is_none());
    click(&mut rt, "create-from-template");
    click(&mut rt, "review-vm-lifecycle");
    click(&mut rt, "review-vm-lifecycle");
    click(&mut rt, &format!("vm-lifecycle-sr-{sr}"));
    click(&mut rt, &format!("vm-lifecycle-network-{network}"));
    click(&mut rt, &format!("vm-lifecycle-iso-{iso_id}"));
    assert_eq!(editor(&rt.state).unwrap().iso, Some(iso_id));
    click(&mut rt, "review-vm-lifecycle");
    assert!(
        matches!(reviewed(&rt.state), VmLifecycleOperation::Create {iso:Some(id),..} if *id==iso_id)
    );
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("Replace the first CD drive")
    );
}

#[test]
fn copied_bios_choice_captures_host_and_affinity_in_the_creation_review() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut s = init();
        firmware_scene(&mut s);
        let host = editor(&s)
            .unwrap()
            .initial
            .hosts
            .keys()
            .next()
            .copied()
            .unwrap();
        let mut rt = runtime(s, size);
        click(&mut rt, &format!("vm-lifecycle-bios-host-{host}"));
        assert_eq!(editor(&rt.state).unwrap().bios_host, Some(host));
        let lines = creation::review(editor(&rt.state).unwrap());
        assert!(
            lines
                .iter()
                .any(|s| s.contains("home-host preference") && s.contains(&host.to_string()))
        );
        assert!(
            matches!(creation::operation(editor(&rt.state).unwrap()).unwrap(),VmLifecycleOperation::Create{bios_host:Some(h),..}if h==host)
        );
        click(&mut rt, "vm-lifecycle-firmware-uefi");
        assert_eq!(
            editor(&rt.state).unwrap().firmware,
            GuestFirmware::Inherited
        ); // installed BIOS disk blocks the flip
        assert_eq!(editor(&rt.state).unwrap().bios_host, Some(host));
    }
}
