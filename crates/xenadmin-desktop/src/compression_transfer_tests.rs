use super::{
    tests::{click, runtime},
    *,
};
use xenadmin_core::XvaCompression;

#[test]
fn compression_choice_invalidates_destination_and_review_keeps_exact_source_and_format() {
    for size in [(513.0, 652.0), (1440.0, 900.0)] {
        let mut rt = runtime(size);
        let source = rt.state.transfers.source.clone();
        let d = tempfile::tempdir().unwrap();
        let plain = d.path().join("old.xva");
        rt.update(|s, _| s.transfers.file = Some(TransferFile::destination(&plain).unwrap()));
        click(&mut rt, "transfer-compression-zstd");
        assert!(rt.state.transfers.file.is_none());
        assert_eq!(rt.state.transfers.compression, XvaCompression::Zstd);
        click(&mut rt, "transfer-choose");
        let (_, options) = rt.file_dialog().unwrap();
        assert_eq!(options.file_name.as_deref(), Some("guest.xva.zst"));
        rt.resolve_file_dialog(None).unwrap();
        rt.frame(0.0);
        let output = d.path().join("chosen.xva.zst");
        rt.update(|s, _| s.transfers.file = Some(TransferFile::destination(&output).unwrap()));
        click(&mut rt, "transfer-review");
        assert!(rt.state.transfers.review);
        assert!(
            rt.accessibility_json()
                .to_string()
                .contains("Output format: zstd")
        );
        click(&mut rt, "transfer-compression-gzip");
        assert!(matches!(
            action(&rt.state.transfers),
            Some(TransferAction::ExportXva {
                compression: XvaCompression::Zstd,
                ..
            })
        ));
        assert_eq!(rt.state.transfers.source, source);
        click(&mut rt, "transfer-apply");
        assert!(!output.exists());
        assert!(rt.state.resource_operations.is_empty());
    }
}

#[test]
fn changing_compression_discards_pending_picker_result_and_package_changes_clear_it() {
    let mut rt = runtime((900.0, 800.0));
    click(&mut rt, "transfer-choose");
    let d = tempfile::tempdir().unwrap();
    rt.update(|s, _| {
        set_format(s, None);
        s.transfers.compression = XvaCompression::Gzip;
    });
    rt.resolve_file_dialog(Some(d.path().join("stale.xva")))
        .unwrap();
    rt.frame(0.0);
    assert!(rt.state.transfers.file.is_none());
    click(&mut rt, "transfer-package-ova");
    assert_eq!(rt.state.transfers.compression, XvaCompression::Uncompressed);
    assert!(matches!(
        action(&rt.state.transfers),
        Some(TransferAction::ExportOvf { .. })
    ));
    assert!(rt.find("transfer-compression-zstd").is_none());
}
