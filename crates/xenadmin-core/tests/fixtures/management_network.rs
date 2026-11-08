use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    HostId, ManagementInterface, ManagementNetworkOperation, OperationControl, OperationId,
    OperationState, ResourceCommand, ResourceOperation, ResourceOperationRecord, ResourceTarget,
};
fn id(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
fn reference(class: &str, n: u128) -> String {
    format!("OpaqueRef:{class}:{n}")
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    LostReply,
    Failed,
    Pending,
    ReadAfterCompletion,
    WrongTask,
    Redirect,
    CertificateCopied,
    RebindForCleanup,
}
struct Control {
    rows: BTreeMap<String, Value>,
    certificate: String,
    coordinator_certificate: String,
    fault: Fault,
    writes: usize,
    wrong_certificate: Arc<std::sync::atomic::AtomicBool>,
    delay: bool,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
struct Bridge {
    member: Option<Mock>,
    endpoint: Endpoint,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
fn acceptor(cert: &rcgen::CertifiedKey) -> tokio_rustls::TlsAcceptor {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio_rustls::TlsAcceptor::from(Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
            )
            .unwrap(),
    ))
}
async fn bridge(
    mock: &Mock,
    cert: &rcgen::CertifiedKey,
    wrong: Arc<std::sync::atomic::AtomicBool>,
    coordinator: &rcgen::CertifiedKey,
    redirect: bool,
) -> Bridge {
    let good = acceptor(cert);
    let coordinator_tls = acceptor(coordinator);
    let bad = acceptor(&rcgen::generate_simple_self_signed(vec!["127.0.0.2".into()]).unwrap());
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = first.local_addr().unwrap().port();
    let listeners = vec![
        first,
        TcpListener::bind(("127.0.0.2", port)).await.unwrap(),
        TcpListener::bind(("127.0.0.3", port)).await.unwrap(),
        TcpListener::bind(("::1", port)).await.unwrap(),
    ];
    let target = url::Url::parse(mock.endpoint.as_str())
        .unwrap()
        .socket_addrs(|| None)
        .unwrap()[0];
    let coordinator_endpoint = format!("https://127.0.0.1:{port}");
    let member = if redirect {
        Some(
            mock_overridden(override_rpc(move |request: Value| {
                let endpoint = coordinator_endpoint.clone();
                async move {
                    (request["method"] == "session.login_with_password").then(|| {
                        RpcReply::Json(
                            json!({"error":{"code":1,"message":"HOST_IS_SLAVE","data":[endpoint]}}),
                        )
                    })
                }
            }))
            .await,
        )
    } else {
        None
    };
    let member_target = member.as_ref().map(|m| {
        url::Url::parse(m.endpoint.as_str())
            .unwrap()
            .socket_addrs(|| None)
            .unwrap()[0]
    });
    let tasks = listeners
        .into_iter()
        .enumerate()
        .map(|(index, listener)| {
            let good = good.clone();
            let bad = bad.clone();
            let wrong = wrong.clone();
            let coordinator_tls = coordinator_tls.clone();
            let target = if index == 0 {
                target
            } else {
                member_target.unwrap_or(target)
            };
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let acceptor = if index != 0 && wrong.load(Ordering::Relaxed) {
                        bad.clone()
                    } else if index == 0 && redirect {
                        coordinator_tls.clone()
                    } else {
                        good.clone()
                    };
                    tokio::spawn(async move {
                        if let Ok(mut tls) = acceptor.accept(stream).await
                            && let Ok(mut upstream) = tokio::net::TcpStream::connect(target).await
                        {
                            let _ = tokio::io::copy_bidirectional(&mut tls, &mut upstream).await;
                        }
                    });
                }
            })
        })
        .collect();
    Bridge {
        member,
        endpoint: Endpoint::parse(&format!("https://127.0.0.1:{port}")).unwrap(),
        tasks,
    }
}
fn rows(ipv6: bool) -> BTreeMap<String, Value> {
    let mut rows = BTreeMap::from([
        (
            reference("pool", 1),
            json!({"uuid":id(1),"name_label":"Management fixture","master":reference("host",2),"ha_enabled":false}),
        ),
        (
            reference("host", 2),
            json!({"uuid":id(2),"name_label":"Reviewed host","address":if ipv6 {"::1"}else{"127.0.0.1"},"enabled":true}),
        ),
    ]);
    for n in [3, 4] {
        rows.insert(reference("network",n),json!({"uuid":id(n),"name_label":format!("Network {n}"),"MTU":"1500","managed":true,"VIFs":[],"PIFs":[reference("PIF",n+10)]}));
        rows.insert(reference("PIF",n+10),json!({"uuid":id(n+10),"host":reference("host",2),"network":reference("network",n),"device":format!("eth{n}"),"MAC":format!("02:00:00:00:00:0{n}"),"management":n==3,"managed":true,"physical":true,"currently_attached":true,"disallow_unplug":false,"MTU":"1500","VLAN":"-1","bond_slave_of":"OpaqueRef:NULL","bond_master_of":[],"VLAN_master_of":"OpaqueRef:NULL","VLAN_slave_of":[],"tunnel_access_PIF_of":[],"tunnel_transport_PIF_of":[],"primary_address_type":if ipv6 {"IPv6"}else{"IPv4"},"ip_configuration_mode":if ipv6 {"None"}else{"Static"},"IP":if ipv6 {""}else if n==3 {"127.0.0.1"}else{"127.0.0.2"},"netmask":if ipv6 {""}else{"255.0.0.0"},"gateway":"","DNS":"","ipv6_configuration_mode":if ipv6 {"Static"}else{"None"},"IPv6":if ipv6 {json!([if n==3 {"::1/128"}else{"2001:db8::2/64"}])}else{json!([""])},"ipv6_gateway":"","other_config":{},"properties":{}}));
    }
    rows
}
async fn fixture(ipv6: bool, fault: Fault) -> (Mock, Bridge, Arc<Mutex<Control>>) {
    let cert = rcgen::generate_simple_self_signed(vec![
        "127.0.0.1".into(),
        "127.0.0.2".into(),
        "::1".into(),
    ])
    .unwrap();
    let coordinator = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let mut records = rows(ipv6);
    if fault == Fault::Redirect {
        records.insert(
            reference("host", 9),
            json!({"uuid":id(9),"address":"192.0.2.9"}),
        );
        records.get_mut(&reference("pool", 1)).unwrap()["master"] = json!(reference("host", 9));
    }
    let wrong_certificate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let control = Arc::new(Mutex::new(Control {
        rows: records,
        certificate: cert.cert.pem(),
        coordinator_certificate: coordinator.cert.pem(),
        fault,
        writes: 0,
        wrong_certificate: wrong_certificate.clone(),
        delay: false,
        entered: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    }));
    let captured = control.clone();
    let mock=mock_overridden(override_rpc(move|request:Value|{let control=captured.clone();async move {
        let method=request["method"].as_str().unwrap();
        let gate=if method.starts_with("Async.") {let mut c=control.lock().unwrap();std::mem::take(&mut c.delay).then(||(c.entered.clone(),c.release.clone()))}else{None};
        if let Some((entered,release))=gate {entered.notify_one();release.notified().await;}
        let mut c=control.lock().unwrap();
        if method=="host.get_server_certificate" && c.writes>0 && c.fault==Fault::RebindForCleanup {c.rows.get_mut(&reference("task",51)).unwrap()["uuid"]=json!(id(99));}
        if method=="host.get_server_certificate" {return Some(RpcReply::Json(json!({"result":if request["params"][1]==reference("host",9) && c.fault==Fault::Redirect {&c.coordinator_certificate}else{&c.certificate}})));}
        if method=="event.from" && request["params"][2]=="" {
            let mut batch=batch("initial","Halted");batch["events"].as_array_mut().unwrap().retain(|v|v["class"]=="VM");
            for (reference,record) in &c.rows {batch["events"].as_array_mut().unwrap().push(json!({"class":reference.split(':').nth(1).unwrap(),"operation":"add","ref":reference,"snapshot":record}));}
            return Some(RpcReply::Json(json!({"result":batch})));
        }
        if let Some(class)=method.strip_suffix(".get_all_records") && ["host","pool","PIF","network","task"].contains(&class) {
            let rows=c.rows.iter().filter(|(r,_)|r.split(':').nth(1)==Some(class)).map(|(r,v)|(r.clone(),v.clone())).collect::<serde_json::Map<_,_>>();return Some(RpcReply::Json(json!({"result":rows})));
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid") && ["host","pool","PIF","network","task"].contains(&class) {
            let row=c.rows.iter().find(|(r,v)|r.split(':').nth(1)==Some(class) && v["uuid"]==request["params"][1]);return Some(RpcReply::Json(match row {Some((r,_))=>json!({"result":r}),_=>json!({"error":{"code":1,"message":"UUID_INVALID","data":[]}})}));
        }
        if let Some(class)=method.strip_suffix(".get_record") && ["host","pool","PIF","network","task"].contains(&class) {
            if c.writes>0 && class=="PIF" && c.fault==Fault::ReadAfterCompletion {return Some(RpcReply::Json(json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}})));}
            return Some(RpcReply::Json(match c.rows.get(request["params"][1].as_str().unwrap()) {Some(v)=>json!({"result":v}),_=>json!({"error":{"code":1,"message":"HANDLE_INVALID","data":[]}})}));
        }
        if method=="task.destroy" {c.rows.remove(request["params"][1].as_str().unwrap());return Some(RpcReply::Json(json!({"result":null})));}
        if method.starts_with("Async.") {
            c.writes+=1;let pif=request["params"][1].as_str().unwrap().to_owned();
            match method {
                "Async.host.management_reconfigure"=>{for (r,v) in &mut c.rows {if r.starts_with("OpaqueRef:PIF:"){v["management"]=json!(r==&pif);}}let address=c.rows[&pif]["IP"].clone();c.rows.get_mut(&reference("host",2)).unwrap()["address"]=address;},
                "Async.PIF.reconfigure_ip"=>{let v=c.rows.get_mut(&pif).unwrap();for (i,key) in [(2,"ip_configuration_mode"),(3,"IP"),(4,"netmask"),(5,"gateway"),(6,"DNS")]{v[key]=request["params"][i].clone();}c.rows.get_mut(&reference("host",2)).unwrap()["address"]=request["params"][3].clone();},
                "Async.PIF.reconfigure_ipv6"=>{let v=c.rows.get_mut(&pif).unwrap();for (i,key) in [(2,"ipv6_configuration_mode"),(4,"ipv6_gateway"),(5,"DNS")]{v[key]=request["params"][i].clone();}v["IPv6"]=json!([request["params"][3].clone()]);},
                _=>panic!("unexpected write {method}"),
            }
            if c.fault==Fault::CertificateCopied {c.rows.insert(reference("host",9),json!({"uuid":id(9),"address":"192.0.2.9"}));}
            let status=match c.fault {Fault::Failed=>"failure",Fault::Pending=>"pending",_=>"success"};
            let task=if c.fault==Fault::WrongTask {50}else{51};
            c.rows.insert(reference("task",51),json!({"uuid":id(task),"status":status,"allowed_operations":[],"progress":1,"result":""}));
            return Some(if c.fault==Fault::LostReply {RpcReply::Drop}else{RpcReply::Json(json!({"result":reference("task",51)}))});
        }
        None
    }})).await;
    let bridge = bridge(
        &mock,
        &cert,
        wrong_certificate,
        &coordinator,
        fault == Fault::Redirect,
    )
    .await;
    (mock, bridge, control)
}
async fn connect(bridge: &Bridge, backend: &Backend) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(bridge.endpoint.clone());
    options.accept_invalid_certificates = true;
    let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(Duration::from_secs(5)).await.unwrap();
    c
}
fn operation(c: &xenadmin_core::Connection, kind: u8) -> ManagementNetworkOperation {
    let view = c.latest();
    let i = view.inventory().unwrap();
    let source = ManagementInterface::from_inventory(&i.details.pifs[&id(13).into()]).unwrap();
    match kind {
        0 => ManagementNetworkOperation::Move {
            source,
            destination: ManagementInterface::from_inventory(&i.details.pifs[&id(14).into()])
                .unwrap(),
        },
        1 => {
            let mut configuration = source.ipv4.clone();
            configuration.address = "127.0.0.3".into();
            ManagementNetworkOperation::ConfigureIpv4 {
                interface: source,
                configuration,
            }
        }
        _ => {
            let mut configuration = source.ipv6.clone();
            configuration.dns = "2001:db8::53".into();
            ManagementNetworkOperation::ConfigureIpv6 {
                interface: source,
                configuration,
            }
        }
    }
}
fn submit(c: &xenadmin_core::Connection, op: ManagementNetworkOperation) -> ResourceOperation {
    c.submit_resource(
        OperationId::new(),
        ResourceCommand::ManagementNetwork {
            host: c
                .latest()
                .observe_resource(ResourceTarget::Host(HostId::from(id(2))))
                .unwrap(),
            operation: Box::new(op),
        },
    )
    .unwrap()
}
async fn finish(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let r = op.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn management_moves_and_ipv4_ipv6_changes_verify_configuration_and_host_bound_route() {
    for kind in [0, 1, 2] {
        let (_mock, bridge, control) = fixture(kind == 2, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let result = finish(&submit(&c, operation(&c, kind))).await;
        assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
        let evidence = result.management_network.as_ref().unwrap();
        assert!(evidence.configuration_observed && evidence.recovery_endpoint_verified);
        assert_eq!(control.lock().unwrap().writes, 1);
        if kind == 0 {
            assert_eq!(
                control.lock().unwrap().rows[&reference("PIF", 13)]["IP"],
                "127.0.0.1"
            );
        }
        let public = serde_json::to_string(&*result).unwrap();
        assert!(!public.contains("management_network_plan") && !public.contains("OpaqueRef:"));
        c.close().await;
    }
}
#[tokio::test]
async fn management_revalidates_ha_identity_ip_and_plain_topology_before_dispatch() {
    for (target, field, value) in [
        ("pool", "ha_enabled", json!(true)),
        ("PIF", "IP", json!("127.0.0.9")),
        ("PIF", "MAC", json!("02:00:00:00:99:99")),
        ("PIF", "MTU", json!(9000)),
        (
            "PIF",
            "other_config",
            json!({"management_purpose":"Storage"}),
        ),
        ("PIF", "bond_master_of", json!(["OpaqueRef:Bond:9"])),
    ] {
        let (_mock, bridge, control) = fixture(false, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let op = operation(&c, 0);
        control
            .lock()
            .unwrap()
            .rows
            .get_mut(&reference(target, if target == "pool" { 1 } else { 13 }))
            .unwrap()[field] = value;
        let result = finish(&submit(&c, op)).await;
        assert_eq!(
            result.state,
            OperationState::Rejected,
            "{field}:{}",
            result.detail
        );
        assert_eq!(control.lock().unwrap().writes, 0);
        c.close().await;
    }
}
#[tokio::test]
async fn a_different_tls_host_cannot_verify_recovery_even_with_validation_disabled() {
    let (mock, bridge, control) = fixture(false, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    control
        .lock()
        .unwrap()
        .wrong_certificate
        .store(true, Ordering::Relaxed);
    let result = finish(&submit(&c, operation(&c, 0))).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    let evidence = result.management_network.as_ref().unwrap();
    assert!(evidence.configuration_observed);
    assert!(!evidence.recovery_endpoint_verified);
    assert_eq!(
        mock.logins.load(Ordering::Relaxed),
        1,
        "mismatched first-hop TLS must withhold credentials"
    );
    c.close().await;
}
#[tokio::test]
async fn shared_host_certificates_refuse_unattributable_management_handover() {
    let (_mock, bridge, control) = fixture(false, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    control.lock().unwrap().rows.insert(
        reference("host", 9),
        json!({"uuid":id(9),"address":"192.0.2.9"}),
    );
    let result = finish(&submit(&c, operation(&c, 0))).await;
    assert_eq!(result.state, OperationState::Rejected, "{}", result.detail);
    assert_eq!(control.lock().unwrap().writes, 0);
    c.close().await;
}
#[tokio::test]
async fn lost_failed_and_rebound_task_receipts_do_not_replay_management_changes() {
    for fault in [Fault::LostReply, Fault::Failed, Fault::WrongTask] {
        let (_mock, bridge, control) = fixture(false, fault).await;
        control.lock().unwrap().rows.insert(
            reference("task", 50),
            json!({"uuid":id(50),"status":"pending"}),
        );
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let result = finish(&submit(&c, operation(&c, 0))).await;
        assert_eq!(
            result.state,
            OperationState::OutcomeUnknown,
            "{}",
            result.detail
        );
        c.control_resource(result.id, OperationControl::Recheck)
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(control.lock().unwrap().writes, 1);
        assert!(
            !c.resource_history()
                .iter()
                .any(|r| r.id == result.id && r.state == OperationState::Succeeded)
        );
        c.close().await;
    }
}
#[tokio::test]
async fn saved_management_completion_survives_missing_task_and_restart_without_reconfiguration() {
    let (_mock, bridge, control) = fixture(false, Fault::ReadAfterCompletion).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let result = finish(&submit(&c, operation(&c, 1))).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    c.close().await;
    {
        let mut state = control.lock().unwrap();
        state.fault = Fault::None;
        state.rows.remove(&reference("task", 51));
    }
    let c = connect(&bridge, &backend).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|r| r.id == result.id && r.state == OperationState::Succeeded)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(control.lock().unwrap().writes, 1);
    c.close().await;
}
#[tokio::test]
async fn recovery_identity_constraint_preserves_original_ca_and_endpoint_pin_policy() {
    use sha2::{Digest, Sha256};
    for trust_recovery in [false, true] {
        let (mock, bridge, control) = fixture(false, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let pem = control.lock().unwrap().certificate.clone();
        let der = rustls_pemfile::certs(&mut pem.as_bytes())
            .next()
            .unwrap()
            .unwrap();
        let pin: xenadmin_core::CertificateFingerprint =
            format!("{:x}", Sha256::digest(der)).parse().unwrap();
        let mut options = ConnectionOptions::new(bridge.endpoint.clone());
        options.certificate_fingerprint = Some(pin.clone());
        if trust_recovery {
            options.add_host_certificate(
                Endpoint::parse(&bridge.endpoint.as_str().replace("127.0.0.1", "127.0.0.2"))
                    .unwrap(),
                pin,
            );
        }
        let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
        c.wait_ready(Duration::from_secs(5)).await.unwrap();
        let result = finish(&submit(&c, operation(&c, 0))).await;
        assert_eq!(
            result.state,
            if trust_recovery {
                OperationState::Succeeded
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            result.detail
        );
        assert_eq!(
            result
                .management_network
                .as_ref()
                .unwrap()
                .recovery_endpoint_verified,
            trust_recovery
        );
        assert_eq!(
            mock.logins.load(Ordering::Relaxed),
            if trust_recovery { 2 } else { 1 }
        );
        c.close().await;
    }
}
#[tokio::test]
async fn closing_during_pending_management_task_keeps_unknown_and_recheck_never_cancels_or_replays()
{
    let (mock, bridge, control) = fixture(false, Fault::Pending).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let op = submit(&c, operation(&c, 0));
    tokio::time::timeout(Duration::from_secs(5), async {
        while op.latest().state != OperationState::Tracking {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    c.close().await;
    let result = finish(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    let c = connect(&bridge, &backend).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(control.lock().unwrap().writes, 1);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "task.cancel")
    );
    c.close().await;
}
#[tokio::test]
async fn delayed_management_receipt_survives_recovery_without_verifying_from_original_worker() {
    for acknowledge in [false, true] {
        let (mock, bridge, control) = fixture(false, Fault::None).await;
        let (entered, release) = {
            let mut c = control.lock().unwrap();
            c.delay = true;
            (c.entered.clone(), c.release.clone())
        };
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        let op = submit(&c, operation(&c, 0));
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let second = Backend::with_state_directory(dir.path().into()).unwrap();
        let recovered = connect(&bridge, &second).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if recovered.resource_history().iter().any(|r| {
                    r.id == op.latest().id
                        && r.state == OperationState::OutcomeUnknown
                        && r.steps[0].state == xenadmin_core::StepState::OutcomeUnknown
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if acknowledge {
            recovered
                .control_resource(op.latest().id, OperationControl::AcknowledgeUnknown)
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !recovered.resource_history().iter().any(|r| {
                    r.id == op.latest().id && r.state == OperationState::AcknowledgedUnknown
                }) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        recovered.close().await;
        let logins = mock.logins.load(Ordering::Relaxed);
        release.notify_one();
        let result = finish(&op).await;
        assert_eq!(
            result.state,
            if acknowledge {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            }
        );
        assert!(
            !result
                .management_network
                .as_ref()
                .unwrap()
                .configuration_observed
        );
        assert_eq!(control.lock().unwrap().writes, 1);
        assert_eq!(mock.logins.load(Ordering::Relaxed), logins);
        c.close().await;
    }
}

#[tokio::test]
async fn member_recovery_route_binds_reviewed_host_before_following_coordinator_redirect() {
    for wrong in [false, true] {
        let (mock, bridge, control) = fixture(false, Fault::Redirect).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&bridge, &backend).await;
        control
            .lock()
            .unwrap()
            .wrong_certificate
            .store(wrong, Ordering::Relaxed);
        let result = finish(&submit(&c, operation(&c, 0))).await;
        assert_eq!(
            result.state,
            if wrong {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Succeeded
            },
            "{}",
            result.detail
        );
        assert_eq!(
            result
                .management_network
                .as_ref()
                .unwrap()
                .recovery_endpoint_verified,
            !wrong
        );
        assert_eq!(
            bridge
                .member
                .as_ref()
                .unwrap()
                .methods
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.as_str() == "session.login_with_password")
                .count(),
            if wrong { 0 } else { 1 }
        );
        assert_eq!(
            mock.logins.load(Ordering::Relaxed),
            if wrong { 1 } else { 2 }
        );
        c.close().await;
    }
}

#[tokio::test]
async fn certificate_uniqueness_is_rechecked_after_change_and_duplicate_recovery_ip_refuses_before_write()
 {
    let (_mock, bridge, control) = fixture(false, Fault::CertificateCopied).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let result = finish(&submit(&c, operation(&c, 0))).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert!(
        result
            .management_network
            .as_ref()
            .unwrap()
            .configuration_observed
    );
    assert!(
        !result
            .management_network
            .as_ref()
            .unwrap()
            .recovery_endpoint_verified
    );
    assert_eq!(control.lock().unwrap().writes, 1);
    c.close().await;
    let (_mock, bridge, control) = fixture(false, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let mut op = operation(&c, 1);
    let ManagementNetworkOperation::ConfigureIpv4 { configuration, .. } = &mut op else {
        panic!()
    };
    configuration.address = "127.0.0.2".into();
    assert!(
        !op.availability(c.latest().inventory().unwrap())
            .is_available()
    );
    let result = finish(&submit(&c, op)).await;
    assert_eq!(result.state, OperationState::Rejected);
    assert!(result.detail.contains("already configured"));
    assert_eq!(control.lock().unwrap().writes, 0);
    c.close().await;
}
#[tokio::test]
async fn readonly_management_connection_cannot_submit_handover() {
    let (_mock, bridge, control) = fixture(false, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let mut options = ConnectionOptions::new(bridge.endpoint.clone());
    options.read_only = true;
    options.accept_invalid_certificates = true;
    let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(Duration::from_secs(5)).await.unwrap();
    let result = c.submit_resource(
        OperationId::new(),
        ResourceCommand::ManagementNetwork {
            host: c
                .latest()
                .observe_resource(ResourceTarget::Host(id(2).into()))
                .unwrap(),
            operation: Box::new(operation(&c, 0)),
        },
    );
    assert!(matches!(result, Err(xenadmin_core::Error::ReadOnly)));
    assert_eq!(control.lock().unwrap().writes, 0);
    c.close().await;
}

#[tokio::test]
async fn management_cleanup_does_not_destroy_a_rebound_foreign_task() {
    let (mock, bridge, control) = fixture(false, Fault::RebindForCleanup).await;
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&bridge, &backend).await;
    let result = finish(&submit(&c, operation(&c, 0))).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    c.close().await;
    assert_eq!(
        control.lock().unwrap().rows[&reference("task", 51)]["uuid"],
        json!(id(99))
    );
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "task.destroy")
    );
}
