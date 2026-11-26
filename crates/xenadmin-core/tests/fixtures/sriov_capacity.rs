use super::*;
use xenadmin_core::{NetworkSriovId, SriovCapacity, SriovCapacityQuery};

pub(super) async fn request(records: &Records, request: &Value) -> Option<RpcReply> {
    if request["method"] != "network_sriov.get_remaining_capacity" {
        return None;
    }
    assert_eq!(request["params"][1], "OpaqueRef:network_sriov");
    records
        .lock()
        .unwrap()
        .insert("_capacity_seen".into(), json!(true));
    while records.lock().unwrap().contains_key("_capacity_hold") {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let mut map = records.lock().unwrap();
    match map
        .get("_capacity_change")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        "pci" => map.get_mut("PIF").unwrap()["PCI"] = json!("OpaqueRef:NULL"),
        "hardware" => map.get_mut("PCI").unwrap()["device_id"] = json!("ffff"),
        "logical" => map.get_mut("network_sriov").unwrap()["logical_PIF"] = json!("OpaqueRef:PIF"),
        _ => {}
    }
    Some(RpcReply::Json(
        if let Some(code) = map.get("_capacity_error") {
            json!({"error":{"code":1,"message":code,"data":[]}})
        } else {
            json!({"result":map.get("_capacity_value").cloned().unwrap_or(json!("7"))})
        },
    ))
}

fn install(records: &Records) {
    let mut m = records.lock().unwrap();
    let mut p = pif();
    p["uuid"] = json!(CHILD);
    p["physical"] = json!(false);
    p["PCI"] = json!("OpaqueRef:NULL");
    p["network"] = json!("OpaqueRef:network");
    p["sriov_logical_PIF_of"] = json!(["OpaqueRef:network_sriov"]);
    m.insert("PIF-child".into(), p);
    m.insert(
        "network_sriov".into(),
        json!({
            "uuid": OBJECT,
            "physical_PIF": "OpaqueRef:PIF",
            "logical_PIF": "OpaqueRef:PIF-child",
            "requires_reboot": false,
            "configuration_mode": "sysfs",
        }),
    );
    m.get_mut("PIF").unwrap()["sriov_physical_PIF_of"] = json!(["OpaqueRef:network_sriov"]);
    m.get_mut("network").unwrap()["PIFs"] = json!(["OpaqueRef:PIF-child"]);
    m.get_mut("host").unwrap()["enabled"] = json!(true);
    m.insert(
        "VM".into(),
        json!({
            "uuid": "44444444-4444-4444-4444-444444444444",
            "name_label": "Synthetic running guest",
            "resident_on": "OpaqueRef:host",
            "power_state": "Running",
            "is_control_domain": false,
        }),
    );
}

fn query(c: &xenadmin_core::Connection) -> SriovCapacityQuery {
    c.inspect_sriov_capacity(
        c.latest()
            .observe_resource(ResourceTarget::Sriov(
                NetworkSriovId::parse(OBJECT).unwrap(),
            ))
            .unwrap(),
    )
    .unwrap()
}

async fn wait(q: &SriovCapacityQuery) -> Result<SriovCapacity, String> {
    tokio::time::timeout(deadline(5), async {
        loop {
            if let Some(r) = q.latest() {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn vf_capacity_is_readonly_on_enabled_populated_hosts_and_reports_reboot_separately() {
    for reboot in [false, true] {
        let (mock, records, writes) = fixture(Kind::Sriov, Fault::None).await;
        install(&records);
        records.lock().unwrap().get_mut("network_sriov").unwrap()["requires_reboot"] =
            json!(reboot);
        records.lock().unwrap().get_mut("PIF-child").unwrap()["currently_attached"] =
            json!(!reboot);
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, true).await;
        let q = query(&c);
        let r = wait(&q).await.unwrap();
        assert_eq!(r.remaining_vfs, 7);
        assert_eq!(r.requires_reboot, Some(reboot));
        assert_eq!(r.attached, Some(!reboot));
        assert!(r.observed_at_ms > 0);
        assert!(q.is_current(Some(&c.latest())));
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "VM.get_all_records" || m == "VIF.get_all_records")
        );
    }
}

#[tokio::test]
async fn vf_capacity_refuses_invalid_negative_overflow_and_missing_method() {
    for value in [
        json!(-1),
        json!("-1"),
        json!("9223372036854775808"),
        json!(9223372036854775808u64),
        json!(1.5),
        json!(null),
        json!("seven"),
    ] {
        let (mock, records, writes) = fixture(Kind::Sriov, Fault::None).await;
        install(&records);
        records
            .lock()
            .unwrap()
            .insert("_capacity_value".into(), value.clone());
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, true).await;
        assert!(wait(&query(&c)).await.is_err(), "{value}");
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }
    for code in ["MESSAGE_METHOD_UNKNOWN", "RBAC_PERMISSION_DENIED"] {
        let (mock, records, _) = fixture(Kind::Sriov, Fault::None).await;
        install(&records);
        records
            .lock()
            .unwrap()
            .insert("_capacity_error".into(), json!(code));
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, true).await;
        assert!(wait(&query(&c)).await.unwrap_err().contains(code));
    }
}

#[tokio::test]
async fn vf_capacity_refuses_changed_hardware_or_logical_mapping() {
    for change in ["pci", "hardware", "logical"] {
        let (mock, records, writes) = fixture(Kind::Sriov, Fault::None).await;
        install(&records);
        records
            .lock()
            .unwrap()
            .insert("_capacity_change".into(), json!(change));
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &backend, true).await;
        assert!(wait(&query(&c)).await.is_err(), "{change}");
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn vf_capacity_stops_on_connection_generation_change() {
    let (mock, records, writes) = fixture(Kind::Sriov, Fault::None).await;
    install(&records);
    records
        .lock()
        .unwrap()
        .insert("_capacity_hold".into(), json!(true));
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &backend, true).await;
    let q = query(&c);
    tokio::time::timeout(deadline(5), async {
        while !records.lock().unwrap().contains_key("_capacity_seen") {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    c.disconnect();
    records.lock().unwrap().remove("_capacity_hold");
    assert!(wait(&q).await.is_err());
    assert_eq!(writes.load(Ordering::SeqCst), 0);
}
