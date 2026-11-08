use super::*;
use xenadmin_core::{LocalDeviceDeclaration, LocalStorageKind};
pub(super) fn is_local(e: &Editor) -> bool {
    matches!(e.kind, 5 | 6)
}
pub(super) fn capture(e: &Editor) -> Result<Option<LocalDeviceDeclaration>, String> {
    if !is_local(e) || e.mode != Mode::Create {
        return Ok(None);
    }
    if !e.local_probe.as_ref().is_some_and(|(host, path, empty)| {
        Some(*host) == e.host && path == e.path.text.trim() && *empty
    }) {
        return Err("Probe the exact host and device for EXT and LVM repositories before reviewing a format. Known repositories must be introduced instead.".into());
    }
    if !e.local_acknowledged
        || e.declaration.text.trim().is_empty()
        || e.declaration.text.len() > 512
        || e.declaration.text.chars().any(char::is_control)
    {
        return Err("Enter your physical-device identity statement and acknowledge the loss of existing data.".into());
    }
    Ok(Some(LocalDeviceDeclaration {
        identity_statement: e.declaration.text.trim().into(),
        acknowledge_data_loss: true,
    }))
}
pub(super) fn fields(s: &State, k: &Kit) -> El {
    let e = s.storage_editor.as_ref().unwrap();
    let mut body=col().gap(8.).child(caption("Single persistent device path on the selected host",k).wrap())
        .child(TextInput::new(&e.path).label("Local /dev/disk/by-id device path").placeholder("/dev/disk/by-id/ata-...").on_edit(|s:&mut State| &mut s.storage_editor.as_mut().unwrap().path).on_change(|s:&mut State,_| {let e=s.storage_editor.as_mut().unwrap();e.local_probe=None;e.local_acknowledged=false;}).build(k).w_full().key("storage-local-device"))
        .child(caption("Enter one /dev/disk/by-id path verified by you. Device lists, mapper/RAID volumes and path traversal are not accepted. The application does not enumerate physical local disks.",k).wrap());
    if e.mode == Mode::Create {
        body=body.child(caption("Operator statement: physical disk/partition identity and intended use",k).wrap())
            .child(TextInput::new(&e.declaration).label("Operator device identity statement").on_edit(|s:&mut State|&mut s.storage_editor.as_mut().unwrap().declaration).build(k).w_full().key("storage-local-identity"))
            .child(caption("This is your declaration, not server-verified evidence. XAPI does not fully verify device identity, capacity, partitions, signatures or current use. An empty repository probe is not proof that a device is blank, unused or readable; aliases may refer to the same disk.",k).wrap())
            .child(Checkbox::new(e.local_acknowledged).label("I verified this device; erase its existing data").on_toggle(|s:&mut State| {let e=s.storage_editor.as_mut().unwrap();e.local_acknowledged = !e.local_acknowledged;}).build(k).key("storage-local-ack"));
    }
    if let Some((host, path, empty)) = &e.local_probe
        && Some(*host) == e.host
        && path == e.path.text.trim()
    {
        body=body.child(caption(if *empty {"The EXT/LVM probe returned no known repository. This does not establish device safety."} else {"The probe found existing storage. Select it to introduce it; formatting is refused."},k).wrap());
    }
    body
}
pub(super) fn scene(s: &mut State, review_page: bool) {
    super::scene(s, false);
    let e = s.storage_editor.as_mut().unwrap();
    e.kind = 5;
    e.name = TextEdit::new("Lab local EXT");
    e.server = TextEdit::new("");
    e.path = TextEdit::new("/dev/disk/by-id/ata-FIXTURE_LAB_DISK_001");
    e.declaration = TextEdit::new("Fixture lab disk 001, whole disk assigned to local VM storage");
    e.local_acknowledged = true;
    e.backends.extend(
        [LocalStorageKind::Ext, LocalStorageKind::Lvm].map(|kind| StorageBackend {
            kind: kind.kind().into(),
            name: kind.label().into(),
            features: vec!["SR_PROBE".into()],
            configuration: std::collections::BTreeMap::from([(
                "device".into(),
                "local device path".into(),
            )]),
        }),
    );
    e.local_probe = Some((e.host.unwrap(), e.path.text.clone(), true));
    if review_page {
        super::review(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
        s.window_width = size.0;
        s.window_height = size.1;
        let mut r = rune::Runtime::new(
            s,
            crate::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.,
            },
        );
        settle(&mut r);
        r
    }
    fn settle(r: &mut rune::Runtime<State>) {
        for _ in 0..30 {
            r.frame(1. / 60.);
        }
    }
    fn click(r: &mut rune::Runtime<State>, key: &str, size: (f32, f32)) {
        r.update(|_, tx| tx.scroll_into_view(key));
        settle(r);
        let (x, y) = r.find(key).unwrap();
        assert!(
            x > 0. && x < size.0 && y > 0. && y < size.1,
            "{key} outside viewport: {x},{y}"
        );
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    #[test]
    fn local_review_captures_operator_declaration_host_and_device_and_keeps_confirmation_reachable()
    {
        for size in [(513., 652.), (1440., 900.)] {
            let mut s = init();
            scene(&mut s, false);
            let original = s.storage_editor.as_ref().unwrap().host.unwrap();
            let mut r = runtime(s, size);
            click(&mut r, "storage-review", size);
            assert!(
                r.state.storage_editor.as_ref().unwrap().review.is_some(),
                "{:?}",
                r.state.storage_editor.as_ref().unwrap().error
            );
            let captured = r
                .state
                .storage_editor
                .as_ref()
                .unwrap()
                .review
                .clone()
                .unwrap();
            assert!(
                matches!(&captured.2,StorageOperation::Create{host,location:StorageLocation::Local{device,..},local_device:Some(d),..} if *host==original&&device.ends_with("001")&&d.identity_statement.contains("Fixture"))
            );
            r.update(|s, _| {
                let e = s.storage_editor.as_mut().unwrap();
                e.host = None;
                e.path = TextEdit::new("/dev/disk/by-id/ata-CHANGED");
                e.declaration = TextEdit::new("changed");
            });
            settle(&mut r);
            assert_eq!(
                r.state.storage_editor.as_ref().unwrap().review.as_ref(),
                Some(&captured)
            );
            click(&mut r, "storage-confirm-device", size);
            click(&mut r, "storage-confirm", size);
            assert!(r.state.resource_operations.is_empty());
            let lines = describe(&captured.2).join("\n");
            for text in [
                "PERMANENTLY ERASES",
                "Operator identity declaration",
                "does not fully verify",
                "automatically forgotten",
            ] {
                assert!(lines.contains(text), "{text}: {lines}");
            }
        }
    }
    #[test]
    fn local_review_requires_current_dual_probe_and_explicit_operator_acknowledgement() {
        for case in 0..6 {
            let mut s = init();
            scene(&mut s, false);
            let e = s.storage_editor.as_mut().unwrap();
            match case {
                0 => e.local_probe = None,
                1 => e.host = None,
                2 => e.path = TextEdit::new("/dev/disk/by-id/ata-CHANGED"),
                3 => e.local_acknowledged = false,
                4 => e.declaration = TextEdit::new(""),
                _ => {}
            };
            if case == 5 {
                query(&mut s, 6);
            }
            super::super::review(&mut s);
            assert!(
                s.storage_editor.as_ref().unwrap().review.is_none(),
                "case {case}"
            );
            assert!(s.resource_operations.is_empty());
        }
    }
    #[test]
    fn local_known_repository_selection_chooses_nonformatting_introduction() {
        let mut s = init();
        scene(&mut s, false);
        let e = s.storage_editor.as_mut().unwrap();
        e.query_inputs = Some(inputs(e));
        e.discovered = Some(StorageDiscoveryResult::LocalRepositories(vec![
            xenadmin_core::DiscoveredLocalRepository {
                kind: LocalStorageKind::Lvm,
                repository: xenadmin_core::DiscoveredRepository {
                    uuid: SrId::from(uuid::Uuid::from_u128(701)),
                    name: "Existing local".into(),
                    size_bytes: Some(4096),
                },
            },
        ]));
        select_result(&mut s, 0);
        let e = s.storage_editor.as_ref().unwrap();
        assert!(e.mode == Mode::Introduce);
        assert_eq!(e.kind, 6);
        assert!(e.review.is_none());
        super::super::review(&mut s);
        assert!(matches!(
            s.storage_editor
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .2,
            StorageOperation::Introduce {
                location: StorageLocation::Local {
                    kind: LocalStorageKind::Lvm,
                    ..
                },
                ..
            }
        ));
        assert!(s.resource_operations.is_empty());
    }
    #[test]
    fn local_form_exposes_declaration_and_erase_acknowledgement_at_both_widths() {
        for size in [(513., 652.), (1440., 900.)] {
            let mut s = init();
            scene(&mut s, false);
            let mut r = runtime(s, size);
            click(&mut r, "storage-local-ack", size);
            assert!(!r.state.storage_editor.as_ref().unwrap().local_acknowledged);
            click(&mut r, "storage-review", size);
            assert!(r.state.storage_editor.as_ref().unwrap().review.is_none());
            click(&mut r, "storage-local-ack", size);
            click(&mut r, "storage-review", size);
            assert!(r.state.storage_editor.as_ref().unwrap().review.is_some());
        }
    }
    #[test]
    fn local_path_focus_preserves_probe_but_text_edit_invalidates_it() {
        let mut s = init();
        scene(&mut s, false);
        let size = (513., 652.);
        let mut r = runtime(s, size);
        click(&mut r, "storage-local-device", size);
        assert!(
            r.state
                .storage_editor
                .as_ref()
                .unwrap()
                .local_probe
                .is_some()
        );
        r.key(rune::Key::Text("-changed".into()).into());
        settle(&mut r);
        assert!(
            r.state
                .storage_editor
                .as_ref()
                .unwrap()
                .local_probe
                .is_none()
        );
        assert!(!r.state.storage_editor.as_ref().unwrap().local_acknowledged);
        click(&mut r, "storage-review", size);
        assert!(r.state.storage_editor.as_ref().unwrap().review.is_none());
    }
}
