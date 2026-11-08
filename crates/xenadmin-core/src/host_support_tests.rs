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
    let f = Fixture::new(Fault::None).await;
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
