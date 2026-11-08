use super::*;
use crate::HostArchiveOperation;
pub(super) const CAPS: &str = r#"<system-status-capabilities><capability key="host-crashdump-logs" pii="yes" min-size="-1" max-size="-1" max-time="30" default-checked="no"/></system-status-capabilities>"#;
pub(super) fn body(backup: bool, host: u128) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    let data = if backup {
        format!(
            "INSTALLATION_UUID='{}'\nPRODUCT_VERSION='8.3.0'\n",
            uuid(host)
        )
    } else {
        "<system-status-inventory><system-summary hostname='fixture'/></system-status-inventory>"
            .into()
    };
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_data(
        &mut header,
        if backup {
            "./etc/xensource-inventory"
        } else {
            "bundle/inventory.xml"
        },
        data.as_bytes(),
    )
    .unwrap();
    let tar = tar.into_inner().unwrap();
    if backup {
        use std::io::Write;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&tar).unwrap();
        gzip.finish().unwrap()
    } else {
        tar
    }
}
async fn fixture(fault: &str) -> Fixture {
    let f = Fixture::transport(Fault::None, true).await;
    {
        let mut r = f.records.lock().unwrap();
        r.get_mut("OpaqueRef:host:h").unwrap()["software_version"] =
            json!({"product_version":"8.3.0"});
        r.insert("fixture:hostfault".into(), json!(fault));
    }
    f
}
async fn execute_host(
    f: &Fixture,
    dir: &Path,
    operation: HostArchiveOperation,
) -> crate::ResourceOperationRecord {
    let (handle, request) = resources::transfer_channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Pool(uuid(1).into()),
        },
        TransferRequest {
            action: TransferAction::Host {
                host: uuid(3).into(),
                expected_version: "8.3.0".into(),
                operation,
            },
            file: TransferFile::destination(dir.join("host-archive")).unwrap(),
        },
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        request.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*handle.latest()).clone()
}
#[tokio::test]
async fn host_backup_and_selected_support_are_task_verified_private_archives() {
    use std::os::unix::fs::PermissionsExt;
    for operation in [
        HostArchiveOperation::Backup,
        HostArchiveOperation::Support {
            entries: vec!["host-crashdump-logs".into()],
        },
    ] {
        let f = fixture("").await;
        let d = tempfile::tempdir().unwrap();
        let r = execute_host(&f, d.path(), operation).await;
        assert_eq!(
            r.state,
            OperationState::Succeeded,
            "{} {:?}",
            r.detail,
            r.local_error
        );
        assert_eq!(
            d.path()
                .join("host-archive")
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(f.count("task.create"), 1);
        assert_eq!(f.count("task.destroy"), 1);
        let public = serde_json::to_string(&r).unwrap();
        assert!(!public.contains("OpaqueRef:"));
        assert!(!public.contains(d.path().to_str().unwrap()));
        assert!(!public.contains("synthetic-secret"));
    }
}
#[tokio::test]
async fn host_export_wrong_identity_truncation_expiry_redirect_failure_or_lost_stream_never_publishes()
 {
    for fault in [
        "wrong-host",
        "truncated",
        "expired",
        "redirect",
        "task-failure",
        "lost",
    ] {
        let f = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute_host(&f, d.path(), HostArchiveOperation::Backup).await;
        assert_ne!(r.state, OperationState::Succeeded, "fault {fault}");
        assert!(!d.path().join("host-archive").exists(), "fault {fault}");
        let r = recheck(&f, d.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded, "fault {fault}");
        assert_eq!(f.count("GET /host_backup"), 1, "fault {fault}");
    }
}
#[tokio::test]
async fn host_export_revalidates_version_and_advertised_categories_before_http() {
    for (version, entries) in [
        ("8.2.0", vec!["host-crashdump-logs".into()]),
        ("8.3.0", vec!["unadvertised".into()]),
    ] {
        let f = fixture("").await;
        f.records
            .lock()
            .unwrap()
            .get_mut("OpaqueRef:host:h")
            .unwrap()["software_version"]["product_version"] = json!(version);
        let d = tempfile::tempdir().unwrap();
        let r = execute_host(&f, d.path(), HostArchiveOperation::Support { entries }).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.count("task.create"), 0);
        assert_eq!(f.count("GET /system-status"), 0);
    }
}
#[test]
fn capabilities_preserve_privacy_and_unknown_estimates_but_reject_ambiguous_keys() {
    let c = crate::host_support::capabilities(CAPS).unwrap();
    assert_eq!(c[0].privacy, "yes");
    assert_eq!(c[0].maximum_bytes, None);
    assert_eq!(c[0].maximum_seconds, Some(30));
    assert!(!c[0].default_selected);
    for xml in [
        CAPS.replace("host-crashdump-logs", "a,b"),
        CAPS.replace(
            "</system-status-capabilities>",
            "<capability key='host-crashdump-logs'/></system-status-capabilities>",
        ),
        "<!DOCTYPE x [<!ENTITY a 'a'>]><system-status-capabilities/>".into(),
    ] {
        assert!(crate::host_support::capabilities(&xml).is_err());
    }
}
#[tokio::test]
async fn support_discovery_exposes_only_selected_host_crashdump_metadata() {
    let f = fixture("").await;
    {
        let mut records = f.records.lock().unwrap();
        records.insert("OpaqueRef:host_crashdump:selected".into(),json!({"uuid":uuid(90),"host":"OpaqueRef:host:h","timestamp":"20261108T03:00:00Z","size":"4096","filename":"private-server-path","other_config":{"private":"private-record-value"}}));
        records.insert("OpaqueRef:host_crashdump:other".into(),json!({"uuid":uuid(91),"host":"OpaqueRef:host:other","timestamp":"private-other-time","size":"8192"}));
    }
    let overview =
        crate::host_support::discover(&f.session().await, uuid(1).into(), uuid(3).into())
            .await
            .unwrap();
    assert_eq!(overview.host, uuid(3).into());
    assert_eq!(overview.version, "8.3.0");
    assert_eq!(overview.crashdumps.len(), 1);
    assert_eq!(overview.crashdumps[0].uuid, uuid(90));
    assert!(overview.capabilities_error.is_none());
    assert!(overview.crashdumps_error.is_none());
    let json = serde_json::to_string(&overview).unwrap();
    for private in [
        "OpaqueRef:",
        "private-server-path",
        "private-record-value",
        "private-other-time",
    ] {
        assert!(!json.contains(private));
    }
}
async fn restore_host(f: &Fixture, dir: &Path, changed: bool) -> crate::ResourceOperationRecord {
    let input = dir.join("backup.tar.gz");
    std::fs::write(&input, body(true, 3)).unwrap();
    let file = TransferFile::host_backup_source(
        &input,
        uuid(3).into(),
        "8.3.0",
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let archive = file.host_backup().unwrap().clone();
    if changed {
        std::fs::write(&input, body(true, 9)).unwrap();
    }
    let (handle, request) = resources::transfer_channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Pool(uuid(1).into()),
        },
        TransferRequest {
            action: TransferAction::Host {
                host: uuid(3).into(),
                expected_version: "8.3.0".into(),
                operation: HostArchiveOperation::StageRestore { archive },
            },
            file,
        },
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        request.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*handle.latest()).clone()
}
#[tokio::test]
async fn host_restore_stages_exact_archive_and_never_claims_running_host_recovery() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let r = restore_host(&f, d.path(), false).await;
    assert_eq!(
        r.state,
        OperationState::Succeeded,
        "{} {:?}",
        r.detail,
        r.local_error
    );
    assert!(r.detail.contains("Running configuration was not restored"));
    assert_eq!(f.count("PUT /host_restore"), 1);
    assert_eq!(f.count("task.create"), 1);
    assert!(r.transfer_created.vms.is_empty());
    assert!(r.transfer.as_ref().unwrap().stream_complete);
    let public = serde_json::to_string(&r).unwrap();
    assert!(!public.contains(d.path().to_str().unwrap()));
    assert!(!public.contains("OpaqueRef:"));
}
#[tokio::test]
async fn staging_lost_reply_or_failed_task_keeps_backup_partition_unknown_and_never_reuploads() {
    for fault in ["lost", "task-failure", "redirect", "expired"] {
        let f = fixture(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = restore_host(&f, d.path(), false).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            r.detail
        );
        let r = recheck(&f, d.path(), r).await;
        assert_eq!(
            r.state,
            OperationState::OutcomeUnknown,
            "{fault}: {}",
            r.detail
        );
        assert_eq!(f.count("PUT /host_restore"), 1);
    }
}
#[tokio::test]
async fn staging_refuses_changed_source_version_read_only_or_busy_host_before_any_http() {
    for fault in ["source", "version", "read-only", "busy"] {
        let mut f = fixture("").await;
        if fault == "read-only" {
            f.options.read_only = true;
        }
        if fault == "version" {
            f.records
                .lock()
                .unwrap()
                .get_mut("OpaqueRef:host:h")
                .unwrap()["software_version"]["product_version"] = json!("8.2.0");
        }
        if fault == "busy" {
            f.records
                .lock()
                .unwrap()
                .get_mut("OpaqueRef:host:h")
                .unwrap()["current_operations"] = json!({"OpaqueRef:other-task":"reboot"});
        }
        let d = tempfile::tempdir().unwrap();
        let r = restore_host(&f, d.path(), fault == "source").await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.count("PUT /host_restore"), 0);
        assert_eq!(f.count("task.create"), 0);
    }
}

#[tokio::test]
async fn restore_requires_unique_current_host_certificate_and_withholds_cookie_and_body_from_wrong_peer()
 {
    for fault in ["wrong", "shared", "missing"] {
        let f = fixture("").await;
        {
            let mut records = f.records.lock().unwrap();
            if fault == "wrong" {
                records.insert(
                    "fixture:hostcertificate".into(),
                    json!(
                        rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()])
                            .unwrap()
                            .cert
                            .pem()
                    ),
                );
            } else if fault == "missing" {
                records.insert("fixture:hostcertificate".into(), Value::Null);
            } else if fault == "shared" {
                records.insert(
                    "OpaqueRef:host:other".into(),
                    json!({"uuid":uuid(90),"address":"127.0.0.2"}),
                );
            }
        }
        let d = tempfile::tempdir().unwrap();
        let row = restore_host(&f, d.path(), false).await;
        assert_ne!(row.state, OperationState::Succeeded, "{fault}");
        assert_eq!(
            f.count("HEADERS /host_restore"),
            0,
            "{fault}: cookie must not cross TLS identity mismatch"
        );
        assert_eq!(
            f.count("PUT /host_restore"),
            0,
            "{fault}: body must not reach wrong peer"
        );
        assert_eq!(
            f.count("task.create"),
            usize::from(fault == "wrong"),
            "{fault}"
        );
    }
}

#[tokio::test]
async fn recovered_or_acknowledged_staging_never_reuploads_or_overwrites_decision() {
    for ack in [false, true] {
        let f = fixture("delayed-restore").await;
        let d = tempfile::tempdir().unwrap();
        let interrupt = async {
            for _ in 0..300 {
                if f.count("PUT /host_restore") > 0 {
                    let journal = Journal::open(d.path()).unwrap();
                    let rows = journal.resource_recover(uuid(1).into()).unwrap();
                    assert_eq!(rows.len(), 1);
                    if ack {
                        journal
                            .resource_control(
                                uuid(1).into(),
                                rows[0].id,
                                crate::OperationControl::AcknowledgeUnknown,
                            )
                            .unwrap();
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("restore was not dispatched")
        };
        let (row, _) = tokio::join!(restore_host(&f, d.path(), false), interrupt);
        assert_eq!(
            row.state,
            if ack {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            row.detail
        );
        assert!(row.interrupted);
        assert_eq!(f.count("PUT /host_restore"), 1);
        if !ack {
            let row = recheck(&f, d.path(), row).await;
            assert!(matches!(
                row.state,
                OperationState::Succeeded | OperationState::OutcomeUnknown
            ));
            assert_eq!(f.count("PUT /host_restore"), 1);
        }
    }
}
#[tokio::test]
async fn staged_restore_journal_refuses_fabricated_success_or_changed_archive_identity() {
    let f = fixture("").await;
    let d = tempfile::tempdir().unwrap();
    let row = restore_host(&f, d.path(), false).await;
    assert_eq!(row.state, OperationState::Succeeded);
    for fault in 0..7 {
        let mut bad = row.clone();
        match fault {
            0 => bad.transfer_plan.as_mut().unwrap().task_success = false,
            1 => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .stream_receipt
                    .as_mut()
                    .unwrap()
                    .sha256 = [99; 32]
            }
            2 => {
                bad.transfer_plan
                    .as_mut()
                    .unwrap()
                    .file
                    .host_backup
                    .as_mut()
                    .unwrap()
                    .host = uuid(99).into()
            }
            3 => bad.transfer_plan.as_mut().unwrap().task_uuid = None,
            4 => bad.target = ResourceTarget::Host(uuid(3).into()),
            5 => bad.steps.clear(),
            _ => bad.steps[0].label = "forged step".into(),
        }
        assert!(
            crate::host_support::restore::validate_record(&bad).is_err(),
            "{fault}"
        );
        assert!(
            Journal::open(d.path())
                .unwrap()
                .resource_save(&mut bad)
                .is_err(),
            "{fault}"
        );
        let journal = Journal::open(d.path()).unwrap();
        let original: String = journal
            .0
            .query_row(
                "SELECT payload FROM resource_operations WHERE id=?1",
                [row.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut payload = serde_json::to_value(&bad).unwrap();
        payload["transfer_plan"] = serde_json::to_value(&bad.transfer_plan).unwrap();
        journal
            .0
            .execute(
                "UPDATE resource_operations SET payload=?1 WHERE id=?2",
                rusqlite::params![payload.to_string(), row.id.to_string()],
            )
            .unwrap();
        assert!(
            journal.resource_load(row.id).is_err(),
            "corrupt load {fault}"
        );
        journal
            .0
            .execute(
                "UPDATE resource_operations SET payload=?1 WHERE id=?2",
                rusqlite::params![original, row.id.to_string()],
            )
            .unwrap();
    }
}
