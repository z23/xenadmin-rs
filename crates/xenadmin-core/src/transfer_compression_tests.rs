use super::*;
use std::io::{Read, Write};

pub(super) fn encode(bytes: &[u8], compression: &str) -> Vec<u8> {
    match compression {
        "false" => bytes.to_vec(),
        "gzip" => {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(bytes).unwrap();
            e.finish().unwrap()
        }
        "zstd" => zstd::stream::encode_all(bytes, 1).unwrap(),
        _ => panic!("unexpected synthetic compression"),
    }
}

#[test]
fn compressed_sources_keep_their_format_and_validate_complete_frames_and_checksums() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("input.xva");
    for compression in XvaCompression::ALL {
        let bytes = encode(&archive(2), compression.query());
        std::fs::write(&path, &bytes).unwrap();
        let file = TransferFile::source(&path).unwrap();
        assert_eq!(file.archive().unwrap().compression, Some(compression));
        let digest = file.archive().unwrap().metadata_digest;
        archive_stream::verify(
            &mut std::fs::File::open(&path).unwrap(),
            &digest,
            &CancellationToken::new(),
        )
        .unwrap();
        if compression != XvaCompression::Uncompressed {
            for end in [4, bytes.len() - 1] {
                std::fs::write(&path, &bytes[..end]).unwrap();
                assert!(
                    archive_stream::verify(
                        &mut std::fs::File::open(&path).unwrap(),
                        &digest,
                        &CancellationToken::new()
                    )
                    .is_err()
                );
            }
            let mut bad = bytes.clone();
            bad.extend(encode(&archive(2), compression.query()));
            std::fs::write(&path, bad).unwrap();
            assert!(
                archive_stream::verify(
                    &mut std::fs::File::open(&path).unwrap(),
                    &digest,
                    &CancellationToken::new()
                )
                .is_err()
            );
        }
    }
    let mut invalid = archive(2);
    let position = {
        let mut a = tar::Archive::new(invalid.as_slice());
        a.entries()
            .unwrap()
            .nth(1)
            .unwrap()
            .unwrap()
            .raw_file_position() as usize
    };
    invalid[position] ^= 1;
    std::fs::write(&path, encode(&invalid, "zstd")).unwrap();
    let file = TransferFile::source(&path).unwrap();
    assert!(
        archive_stream::verify(
            &mut std::fs::File::open(&path).unwrap(),
            &file.archive().unwrap().metadata_digest,
            &CancellationToken::new()
        )
        .is_err()
    );
}

#[test]
fn zstd_window_bound_and_cancellation_apply_before_unbounded_frame_scanning() {
    // Unknown-size frame, 256 MiB declared window, empty final block.
    let large = [0x28, 0xb5, 0x2f, 0xfd, 0, 0x90, 1, 0, 0];
    let mut reader =
        super::super::compression::decoder(large.as_slice(), XvaCompression::Zstd, &|| false)
            .unwrap();
    assert!(reader.read_to_end(&mut Vec::new()).is_err());
    struct Stops {
        data: std::io::Cursor<Vec<u8>>,
        reads: usize,
        cancel: CancellationToken,
    }
    impl Read for Stops {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            let len = out.len().min(7);
            let n = std::io::Read::read(&mut self.data, &mut out[..len])?;
            if self.reads == 8 {
                self.cancel.cancel();
            }
            Ok(n)
        }
    }
    let token = CancellationToken::new();
    let mut input = Stops {
        data: std::io::Cursor::new(encode(&[], "zstd").repeat(1000)),
        reads: 0,
        cancel: token.clone(),
    };
    let stop = || token.is_cancelled();
    let result = {
        let mut reader =
            super::super::compression::decoder(&mut input, XvaCompression::Zstd, &stop).unwrap();
        reader.read_to_end(&mut Vec::new())
    };
    assert!(result.is_err());
    assert_eq!(input.reads, 8);
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("stopped.xva.zst");
    std::fs::write(&path, encode(&archive(2), "zstd")).unwrap();
    assert!(matches!(
        TransferFile::source_with_cancel(&path, &std::sync::atomic::AtomicBool::new(true)),
        Err(Error::Closed)
    ));
}

#[tokio::test]
async fn compressed_exports_request_and_verify_exact_format_including_memory_checkpoints() {
    for compression in [XvaCompression::Gzip, XvaCompression::Zstd] {
        for checkpoint in [false, true] {
            let f = Fixture::new(Fault::None).await;
            let d = tempfile::tempdir().unwrap();
            let mut action = if checkpoint {
                checkpoint_tests::setup(&f);
                checkpoint_tests::action(true)
            } else {
                TransferAction::ExportXva {
                    source: ExportSource::Vm {
                        vm: uuid(2).into(),
                        expected_template: false,
                    },
                    compression,
                }
            };
            if let TransferAction::ExportXva { compression: c, .. } = &mut action {
                *c = compression;
            }
            f.records.lock().unwrap().insert(
                "fixture:expected_compression".into(),
                json!(compression.query()),
            );
            let path = d.path().join(format!("output.{}", compression.extension()));
            let r = checkpoint_tests::run(
                &f,
                d.path(),
                action,
                TransferFile::destination(&path).unwrap(),
            )
            .await;
            assert_eq!(
                r.state,
                OperationState::Succeeded,
                "{compression:?}: {}",
                r.detail
            );
            let file = TransferFile::source(&path).unwrap();
            assert_eq!(file.archive().unwrap().compression, Some(compression));
            assert_eq!(
                file.archive().unwrap().memory_images(),
                usize::from(checkpoint)
            );
            assert_eq!(f.count("GET /export"), 1);
        }
    }
}

#[tokio::test]
async fn ignored_compression_and_corrupt_compressed_exports_never_publish_or_replay() {
    for fault in [Fault::None, Fault::BadChunkChecksum] {
        let f = Fixture::new(fault).await;
        let d = tempfile::tempdir().unwrap();
        {
            let mut r = f.records.lock().unwrap();
            r.insert("fixture:expected_compression".into(), json!("zstd"));
            if fault == Fault::None {
                r.insert("fixture:response_compression".into(), json!("false"));
            }
        }
        let path = d.path().join("output.xva.zst");
        let r = checkpoint_tests::run(
            &f,
            d.path(),
            TransferAction::ExportXva {
                source: ExportSource::Vm {
                    vm: uuid(2).into(),
                    expected_template: false,
                },
                compression: XvaCompression::Zstd,
            },
            TransferFile::destination(&path).unwrap(),
        )
        .await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert!(!path.exists());
        let _ = recheck(&f, d.path(), r).await;
        assert!(!path.exists());
        assert_eq!(f.count("GET /export"), 1);
    }
}

#[tokio::test]
async fn compressed_import_keeps_original_bytes_and_existing_provenance_rules() {
    for compression in [XvaCompression::Gzip, XvaCompression::Zstd] {
        let f = Fixture::new(Fault::None).await;
        let d = tempfile::tempdir().unwrap();
        let bytes = encode(&archive(2), compression.query());
        f.records
            .lock()
            .unwrap()
            .insert("fixture:upload_bytes".into(), json!(&bytes));
        let path = d.path().join(compression.extension());
        std::fs::write(&path, bytes).unwrap();
        let r = checkpoint_tests::run(
            &f,
            d.path(),
            TransferAction::ImportXva { sr: uuid(4).into() },
            TransferFile::source(&path).unwrap(),
        )
        .await;
        assert_eq!(
            r.state,
            OperationState::Succeeded,
            "{compression:?}: {}",
            r.detail
        );
        assert_eq!(r.transfer_created.vms, vec![uuid(20).into()]);
        assert_eq!(f.count("PUT /import"), 1);
    }
}

#[tokio::test]
async fn compressed_checkpoint_import_preserves_memory_provenance_without_resume_or_replay() {
    for compression in [XvaCompression::Gzip, XvaCompression::Zstd] {
        for lost in [false, true] {
            let f = Fixture::new(if lost { Fault::LostImport } else { Fault::None }).await;
            checkpoint_tests::setup(&f);
            let d = tempfile::tempdir().unwrap();
            let bytes = encode(&checkpoint_tests::archive(2, 10, true), compression.query());
            f.records
                .lock()
                .unwrap()
                .insert("fixture:upload_bytes".into(), json!(&bytes));
            let path = d.path().join(compression.extension());
            std::fs::write(&path, bytes).unwrap();
            let r = checkpoint_tests::run(
                &f,
                d.path(),
                TransferAction::ImportCheckpointXva { sr: uuid(4).into() },
                TransferFile::source(&path).unwrap(),
            )
            .await;
            assert_eq!(
                r.state,
                if lost {
                    OperationState::PartiallyCompleted
                } else {
                    OperationState::Succeeded
                },
                "{compression:?}: {}",
                r.detail
            );
            assert_eq!(r.transfer_created.suspend_images, vec![uuid(24).into()]);
            assert_eq!(r.transfer_created.vms, vec![uuid(20).into()]);
            if lost {
                let _ = recheck(&f, d.path(), r).await;
            }
            assert_eq!(f.count("PUT /import"), 1);
            assert_eq!(f.count("VM.start"), 0);
            assert_eq!(f.count("VM.resume"), 0);
        }
    }
}

#[test]
fn old_export_intents_default_to_plain_without_changing_source_identity() {
    let action: TransferAction = serde_json::from_value(
        json!({"ExportXva":{"source":{"Vm":{"vm":uuid(2),"expected_template":false}}}}),
    )
    .unwrap();
    assert!(matches!(
        action,
        TransferAction::ExportXva {
            compression: XvaCompression::Uncompressed,
            ..
        }
    ));
}
