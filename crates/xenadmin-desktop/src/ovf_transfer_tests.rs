use super::tests::{click, disk_scene, runtime};
use super::*;
fn package(dir: &std::path::Path) -> std::path::PathBuf {
    let xml = r#"<Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1" xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1" xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData"><References><File ovf:id="data" ovf:href="data.raw"/></References><DiskSection><Info>Disks</Info><Disk ovf:diskId="data" ovf:fileRef="data" ovf:capacity="4096" ovf:format="http://raw"/></DiskSection><NetworkSection><Info>Networks</Info><Network ovf:name="Application network"/></NetworkSection><VirtualSystem ovf:id="guest"><Name>Portable application</Name><VirtualHardwareSection><Info>Hardware</Info><Item><rasd:InstanceID>1</rasd:InstanceID><rasd:ResourceType>3</rasd:ResourceType><rasd:VirtualQuantity>2</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>2</rasd:InstanceID><rasd:ResourceType>4</rasd:ResourceType><rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits><rasd:VirtualQuantity>512</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>3</rasd:InstanceID><rasd:ResourceType>17</rasd:ResourceType><rasd:HostResource>ovf:/disk/data</rasd:HostResource></Item><Item><rasd:InstanceID>4</rasd:InstanceID><rasd:ResourceType>10</rasd:ResourceType><rasd:Connection>Application network</rasd:Connection></Item></VirtualHardwareSection></VirtualSystem></Envelope>"#;
    std::fs::write(dir.join("data.raw"), [7u8; 4096]).unwrap();
    let path = dir.join("application.ovf");
    std::fs::write(&path, xml).unwrap();
    path
}
fn picked(rt: &mut rune::Runtime<State>, path: std::path::PathBuf) {
    rt.resolve_file_dialog(Some(path)).unwrap();
    rt.frame(0.0);
    for _ in 0..500 {
        if poll(&mut rt.state) {
            rt.update(|_, _| {});
            rt.frame(0.0);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("inspection did not complete");
}
#[test]
fn ovf_import_requires_explicit_network_mapping_and_reviews_captured_storage() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut rt = runtime(size);
        disk_scene(&mut rt);
        click(&mut rt, "transfer-format-ovf");
        let sr = SrId::from(rt.state.inventory.as_ref().unwrap().storage[0].uuid);
        click(&mut rt, &format!("transfer-sr-{sr}"));
        let dir = tempfile::tempdir().unwrap();
        let file = package(dir.path());
        click(&mut rt, "transfer-choose");
        assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Open);
        picked(&mut rt, file.clone());
        let summary = rt.state.transfers.file.as_ref().unwrap().ovf().unwrap();
        assert_eq!(summary.guests[0].name, "Portable application");
        assert_eq!(summary.guests[0].memory_bytes, 512 << 20);
        assert!(action(&rt.state.transfers).is_none());
        click(&mut rt, "transfer-review");
        assert!(!rt.state.transfers.review);
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
        click(&mut rt, &format!("ovf-network-0-{network}"));
        click(&mut rt, "transfer-review");
        assert!(rt.state.transfers.review);
        assert_eq!(
            action(&rt.state.transfers),
            Some(TransferAction::ImportOvf {
                sr,
                networks: std::collections::BTreeMap::from([(
                    "Application network".into(),
                    network
                )])
            })
        );
        let text = rt.accessibility_json().to_string();
        assert!(text.contains("MAC addresses are generated"));
        assert!(text.contains("No package manifest"));
        assert!(text.contains(&network.to_string()));
        click(&mut rt, "transfer-apply");
        assert!(rt.state.resource_operations.is_empty());
        assert!(file.exists());
        rt.update(|s, _| {
            Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into()
        });
        assert!(!current(&rt.state, &rt.state.transfers));
        apply(&mut rt.state);
        assert!(rt.state.resource_operations.is_empty());
    }
}
#[test]
fn ova_and_ovf_export_use_new_destinations_and_keep_captured_source() {
    for (format, key, name) in [
        (OvfPackageFormat::Ova, "ova", "application.ova"),
        (OvfPackageFormat::Ovf, "ovf", "application-package"),
    ] {
        let mut rt = runtime((513.0, 652.0));
        let source = rt.state.transfers.source.clone().unwrap();
        click(&mut rt, &format!("transfer-package-{key}"));
        assert_eq!(
            action(&rt.state.transfers),
            Some(TransferAction::ExportOvf {
                source: source.clone(),
                format
            })
        );
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join(name);
        click(&mut rt, "transfer-choose");
        assert_eq!(rt.file_dialog().unwrap().0, rune::FileDialogKind::Save);
        picked(&mut rt, dest.clone());
        assert!(rt.state.transfers.file.is_some());
        click(&mut rt, "transfer-review");
        assert!(rt.state.transfers.review);
        click(&mut rt, "transfer-apply");
        assert!(rt.state.resource_operations.is_empty());
        assert!(!dest.exists());
        assert_eq!(rt.state.transfers.source.as_ref(), Some(&source));
    }
}
#[test]
fn changing_package_format_discards_inspection_and_stale_picker_results() {
    let mut rt = runtime((640.0, 800.0));
    let dir = tempfile::tempdir().unwrap();
    click(&mut rt, "transfer-package-ova");
    click(&mut rt, "transfer-choose");
    let serial = rt.state.transfers.serial;
    set_format(&mut rt.state, None);
    rt.state.transfers.package_export = Some(OvfPackageFormat::Ovf);
    assert!(rt.state.transfers.serial > serial);
    rt.resolve_file_dialog(Some(dir.path().join("old.ova")))
        .unwrap();
    rt.frame(0.0);
    assert!(rt.state.transfers.file.is_none());
    assert!(rt.state.transfers.result.is_none());
    assert!(!rt.state.transfers.pending);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
