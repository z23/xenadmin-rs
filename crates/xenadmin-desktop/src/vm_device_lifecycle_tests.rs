use super::*;
use std::sync::Arc;
use xenadmin_core::{ClassAvailability, VbdId};

fn device_state() -> State {
    let mut state = crate::init();
    crate::demo(&mut state);
    state.table.select("30000000-0000-0000-0000-000000000000");
    crate::vm_devices::open(&mut state);
    let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
    let disk = inventory
        .details
        .vdis
        .values_mut()
        .find(|disk| disk.kind == "user")
        .unwrap();
    disk.allowed_operations
        .as_mut()
        .unwrap()
        .push("destroy".into());
    let mut unattached = disk.clone();
    unattached.id = VdiId::parse("71000000-0000-0000-0000-000000000099").unwrap();
    unattached.name = "Existing data".into();
    inventory.details.vdis.insert(unattached.id, unattached);
    state
}
fn runtime(mut state: State, size: (f32, f32)) -> rune::Runtime<State> {
    state.window_width = size.0;
    state.window_height = size.1;
    let mut rt = rune::Runtime::new(
        state,
        crate::view,
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
    rt.update(|_, tx| tx.scroll_into_view(key));
    settle(rt);
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("missing control: {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.0);
    settle(rt);
}
fn type_text(rt: &mut rune::Runtime<State>, key: &str, text: &str) {
    click(rt, key);
    rt.key(rune::Key::Text(text.into()).into());
    rt.frame(0.0);
}
fn form(state: &State) -> &Form {
    state.devices.as_ref().unwrap().lifecycle.as_ref().unwrap()
}
fn reviewed(state: &State) -> &DeviceOperation {
    &form(state).review.as_ref().unwrap().operation
}

#[test]
fn disk_creation_reviews_selected_storage_size_and_captured_vm_at_both_widths() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(device_state(), size);
        let original_vm = rt.state.devices.as_ref().unwrap().vm;
        let pool = rt.state.inventory.as_ref().unwrap().pool_id;
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
        click(&mut rt, "create-vm-disk");
        type_text(&mut rt, "lifecycle-name", "Application data");
        type_text(&mut rt, "lifecycle-size", "16 GiB");
        click(&mut rt, &format!("lifecycle-sr-{sr}"));
        click(&mut rt, "review-device-lifecycle");
        assert_eq!(
            reviewed(&rt.state),
            &DeviceOperation::CreateDisk {
                sr,
                name: "Application data".into(),
                size: 16 << 30,
                device: "1".into()
            }
        );
        rt.state
            .table
            .select("30000000-0000-0000-0000-000000000001");
        rt.frame(0.0);
        assert_eq!(form(&rt.state).vm, original_vm);
        assert_eq!(form(&rt.state).pool, pool);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Application data")
        );
        click(&mut rt, "apply-device-lifecycle");
        assert!(rt.state.operations.is_empty());
        assert!(
            form(&rt.state).review.is_some(),
            "synthetic reviews never submit"
        );
    }
}

#[test]
fn attach_and_add_cd_review_exact_targets_without_deleting_existing_data() {
    let mut rt = runtime(device_state(), (513.0, 652.0));
    let disk = VdiId::parse("71000000-0000-0000-0000-000000000099").unwrap();
    click(&mut rt, "attach-vm-disk");
    click(&mut rt, "lifecycle-read-only");
    click(&mut rt, &format!("lifecycle-disk-{disk}"));
    click(&mut rt, "review-device-lifecycle");
    assert_eq!(
        reviewed(&rt.state),
        &DeviceOperation::AttachDisk {
            vdi: disk,
            device: "1".into(),
            read_only: true
        }
    );
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("Existing disk contents are retained")
    );
    click(&mut rt, "cancel-device-lifecycle");
    click(&mut rt, "add-vm-cd");
    click(&mut rt, "review-device-lifecycle");
    assert_eq!(
        reviewed(&rt.state),
        &DeviceOperation::AddCdDrive { device: "1".into() }
    );
}

#[test]
fn detach_preserves_data_delete_reviews_consumers_and_ownership_changes_disable_it() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(device_state(), size);
        let vbd = VbdId::parse("72000000-0000-0000-0000-000000000001").unwrap();
        let disk = rt.state.inventory.as_ref().unwrap().details.vbds[&vbd]
            .vdi
            .unwrap();
        click(&mut rt, &format!("detach-disk-{vbd}"));
        assert_eq!(
            reviewed(&rt.state),
            &DeviceOperation::DetachDisk { vbd, vdi: disk }
        );
        assert!(!form(&rt.state).review.as_ref().unwrap().destructive);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("disk and its data remain")
        );
        click(&mut rt, "cancel-device-lifecycle");
        click(&mut rt, &format!("delete-disk-{vbd}"));
        assert_eq!(
            reviewed(&rt.state),
            &DeviceOperation::DeleteDisk { vbd, vdi: disk }
        );
        let text = rt.accessibility_json().to_string();
        assert!(text.contains("All data on this disk will be lost"));
        assert!(text.contains("Consumer:"));
        assert!(text.contains(&form(&rt.state).vm.to_string()));
        rt.update(|state, _| {
            Arc::make_mut(state.inventory.as_mut().unwrap())
                .details
                .vbds
                .get_mut(&vbd)
                .unwrap()
                .owned = false;
        });
        rt.frame(0.0);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("no longer available")
        );
        click(&mut rt, "apply-device-lifecycle");
        assert!(rt.state.operations.is_empty());
        assert!(form(&rt.state).review.is_some());
    }
}

#[test]
fn nic_editor_reviews_generation_and_replacement_and_captures_original_identity() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut rt = runtime(device_state(), size);
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
        click(&mut rt, "add-vm-nic");
        click(&mut rt, &format!("lifecycle-network-{network}"));
        click(&mut rt, "review-device-lifecycle");
        assert_eq!(
            reviewed(&rt.state),
            &DeviceOperation::AddNic {
                network,
                device: "1".into(),
                mac: None
            }
        );
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("generated by the host")
        );
        click(&mut rt, "cancel-device-lifecycle");
        let nic = rt
            .state
            .inventory
            .as_ref()
            .unwrap()
            .details
            .vifs
            .values()
            .next()
            .unwrap()
            .clone();
        click(&mut rt, &format!("edit-nic-{}", nic.id));
        assert!(
            build_operation(form(&rt.state))
                .unwrap_err()
                .contains("different")
        );
        // Replace the editor value without depending on platform text selection shortcuts.
        form_mut(&mut rt.state).unwrap().mac = TextEdit::new("02:00:00:00:00:42");
        rt.frame(0.0);
        click(&mut rt, "review-device-lifecycle");
        assert_eq!(
            reviewed(&rt.state),
            &DeviceOperation::EditNic {
                vif: nic.id,
                expected_network: nic.network.unwrap(),
                expected_mac: nic.mac,
                network: nic.network.unwrap(),
                mac: "02:00:00:00:00:42".into()
            }
        );
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("VM may be left without this NIC")
        );
        click(&mut rt, "cancel-device-lifecycle");
        click(&mut rt, &format!("delete-nic-{}", nic.id));
        assert_eq!(
            reviewed(&rt.state),
            &DeviceOperation::DeleteNic { vif: nic.id }
        );
    }
}

#[test]
fn invalid_resource_choices_and_user_values_cannot_reach_review() {
    let mut state = device_state();
    open(&mut state, Kind::CreateDisk);
    assert!(build_operation(form(&state)).is_err());
    let sr = *state
        .inventory
        .as_ref()
        .unwrap()
        .details
        .storage
        .keys()
        .next()
        .unwrap();
    let f = form_mut(&mut state).unwrap();
    f.name = TextEdit::new("Data");
    f.size = TextEdit::new("16 GiB");
    assert!(build_operation(f).unwrap_err().contains("storage"));
    f.sr = Some(sr);
    for invalid in ["0", "513", "1.5 GiB", "18446744073709551615 GiB", "-1"] {
        f.size = TextEdit::new(invalid);
        assert!(build_operation(f).is_err(), "accepted size {invalid}");
    }
    f.size = TextEdit::new("512");
    for invalid in ["", "256", "-1", "01", "one"] {
        f.device = TextEdit::new(invalid);
        assert!(build_operation(f).is_err(), "accepted slot {invalid}");
    }
    f.device = TextEdit::new("1");
    for invalid in ["", "  ", "Data\n", "\tData"] {
        f.name = TextEdit::new(invalid);
        assert!(build_operation(f).is_err(), "accepted name {invalid:?}");
    }
    open(&mut state, Kind::AddNic);
    assert!(
        build_operation(form(&state))
            .unwrap_err()
            .contains("network")
    );
    for invalid in [
        "00:00:00:00:00:00",
        "01:00:00:00:00:01",
        "02:00:00:00:00",
        "02:00:00:00:00:gg",
    ] {
        assert!(parse_mac(invalid).is_err(), "accepted MAC {invalid}");
    }
    assert_eq!(parse_mac("02:AB:CD:EF:01:23").unwrap(), "02:ab:cd:ef:01:23");
}

#[test]
fn device_classes_are_read_using_canonical_inventory_keys() {
    let mut state = device_state();
    let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
    assert!(super::super::unavailable(inventory, "VBD", "Disks").is_none());
    inventory
        .details
        .classes
        .insert("vbd".into(), ClassAvailability::PermissionDenied);
    assert!(
        super::super::unavailable(inventory, "VBD", "Disks")
            .unwrap()
            .contains("permission")
    );
}

#[test]
fn reviewed_changes_cannot_follow_a_pool_switch_or_missing_live_observation() {
    let mut rt = runtime(device_state(), (513.0, 652.0));
    click(&mut rt, "add-vm-cd");
    click(&mut rt, "review-device-lifecycle");
    let source_pool = form(&rt.state).pool;
    let operation = reviewed(&rt.state).clone();
    rt.update(|state, _| {
        Arc::make_mut(state.inventory.as_mut().unwrap()).pool_id =
            PoolId::parse("10000000-0000-0000-0000-000000000099").unwrap();
    });
    // The device page itself is hidden when its connection is no longer selected.
    assert_ne!(rt.state.inventory.as_ref().unwrap().pool_id, source_pool);
    apply(&mut rt.state);
    assert!(rt.state.operations.is_empty());
    assert_eq!(reviewed(&rt.state), &operation);
    rt.update(|state, _| {
        Arc::make_mut(state.inventory.as_mut().unwrap()).pool_id = source_pool;
        state.demo = false;
    });
    rt.frame(0.0);
    assert!(
        rt.accessibility_json()
            .to_string()
            .contains("Cancel and reopen")
    );
    click(&mut rt, "apply-device-lifecycle");
    assert!(rt.state.operations.is_empty());
    assert_eq!(reviewed(&rt.state), &operation);
}

#[test]
fn shared_disk_consumers_are_described_and_deletion_stays_unavailable() {
    let mut state = device_state();
    let vm = state.devices.as_ref().unwrap().vm;
    let inventory = Arc::make_mut(state.inventory.as_mut().unwrap());
    let vbd = VbdId::parse("72000000-0000-0000-0000-000000000001").unwrap();
    let mut second = inventory.details.vbds[&vbd].clone();
    let disk = second.vdi.unwrap();
    second.id = VbdId::parse("72000000-0000-0000-0000-000000000099").unwrap();
    second.vm = Some(*inventory.vms.keys().find(|id| **id != vm).unwrap());
    inventory
        .details
        .vdi_attachments
        .get_mut(&disk)
        .unwrap()
        .push(second.id);
    inventory.details.vbds.insert(second.id, second);
    let operation = DeviceOperation::DeleteDisk { vbd, vdi: disk };
    assert!(!operation.availability(inventory, vm).is_available());
    let review = review_for(operation, inventory);
    assert_eq!(
        review
            .lines
            .iter()
            .filter(|line| line.starts_with("Consumer:"))
            .count(),
        2
    );
}
