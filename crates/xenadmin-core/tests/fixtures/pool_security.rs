use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    DirectoryHostState, HostId, OperationControl, OperationId, OperationState, PoolId,
    PoolOperation, PoolSecurityOperation as SecurityOp, PoolSecurityQuery, PoolSecurityResult,
    PoolSensitiveChange, ResourceCommand, ResourceOperation, ResourceOperationRecord,
    ResourceTarget, RoleId, ServerCertificate, SubjectId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const SUBJECT: &str = "44444444-4444-4444-4444-444444444444";
const ROLE: &str = "55555555-5555-5555-5555-555555555555";
const LEAF: &str = "66666666-6666-6666-6666-666666666666";
const NEW: &str = "77777777-7777-7777-7777-777777777777";
const SECRET: &str = "synthetic-directory-password-never-persist";
const METHODS: [&str; 8] = [
    "pool.enable_external_auth",
    "pool.disable_external_auth",
    "subject.create",
    "subject.destroy",
    "subject.add_to_roles",
    "subject.remove_from_roles",
    "host.install_server_certificate",
    "host.reset_server_certificate",
];
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropReply,
    Partial,
    UnexpectedRoles,
    VerifyFailure,
    Unsupported,
    NotRoot,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
fn pem() -> (String, String) {
    let c = rcgen::generate_simple_self_signed(vec!["fixture.test".into()]).unwrap();
    (c.cert.pem(), c.key_pair.serialize_pem())
}
async fn fixture(fault: Fault) -> (Mock, Records, Arc<Mutex<String>>) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Security fixture","current_operations":{}}),
        ),
        (
            "host".into(),
            json!({"uuid":HOST,"name_label":"Fixture host","address":"192.0.2.10","enabled":true,"current_operations":{},"external_auth_type":"","external_auth_service_name":"","license_params":{"restrict_rbac":"false"}}),
        ),
        (
            "subject".into(),
            json!({"uuid":SUBJECT,"subject_identifier":"S-fixture-existing","other_config":{"subject-name":"Example User"},"roles":[]}),
        ),
        (
            "role".into(),
            json!({"uuid":ROLE,"name_label":"Read only","name_description":"Read-only access","subroles":["OpaqueRef:role-leaf"]}),
        ),
        (
            "role-leaf".into(),
            json!({"uuid":LEAF,"name_label":"permission","name_description":"Leaf permission","subroles":[]}),
        ),
    ])));
    let certificate = Arc::new(Mutex::new(pem().0));
    let r = records.clone();
    let c = certificate.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let verify = Arc::new(AtomicUsize::new(0));
    let mock=mock_overridden(override_rpc(move|request|{let r=r.clone();let c=c.clone();let writes=writes.clone();let verify=verify.clone();async move{
        let method=request["method"].as_str().unwrap();let p=&request["params"];
        if method=="event.from"&&p[2]==""{let mut initial=batch("initial","Halted");for event in initial["events"].as_array_mut().unwrap(){if let Some(v)=r.lock().unwrap().get(event["class"].as_str().unwrap()){event["snapshot"]=v.clone();}}return Some(RpcReply::Json(json!({"result":initial})));}
        if method=="host.list_methods"{return Some(RpcReply::Json(json!({"result":if fault==Fault::Unsupported{vec![]}else{METHODS.to_vec()}})));}
        if method=="session.get_record"{return Some(RpcReply::Json(json!({"result":{"is_local_superuser":fault!=Fault::NotRoot}})));}
        if method=="auth.get_subject_identifier"{assert_eq!(p[1],"example\\alice");return Some(RpcReply::Json(json!({"result":"S-fixture-new"})));}
        if method=="auth.get_subject_information_from_identifier"{return Some(RpcReply::Json(json!({"result":{"subject-name":"Fixture User","private-data":"not projected"}})));}
        if method=="host.get_server_certificate"{return Some(RpcReply::Json(json!({"result":*c.lock().unwrap()})));}
        if let Some(class)=method.strip_suffix(".get_all_records")&&["host","subject","role"].contains(&class){
            if fault==Fault::VerifyFailure&&writes.load(Ordering::SeqCst)>0&&verify.fetch_add(1,Ordering::SeqCst)==0{return Some(RpcReply::Json(json!({"error":{"message":"RBAC_PERMISSION_DENIED","data":[SECRET]}})));}
            let values:serde_json::Map<_,_>=r.lock().unwrap().iter().filter(|(key,_)|key.starts_with(class)).map(|(key,v)|(format!("OpaqueRef:{key}"),v.clone())).collect();return Some(RpcReply::Json(json!({"result":values})));
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid")&&["pool","host","subject","role"].contains(&class){let records=r.lock().unwrap();let found=records.iter().find(|(_,v)|v["uuid"]==p[1]);return Some(RpcReply::Json(match found{Some((key,_))=>json!({"result":format!("OpaqueRef:{key}")}),None=>json!({"error":{"message":"UUID_INVALID","data":[]}})}));}
        if let Some(class)=method.strip_suffix(".get_record")&&["pool","host","subject","role"].contains(&class){let key=p[1].as_str().unwrap().strip_prefix("OpaqueRef:").unwrap();return Some(RpcReply::Json(json!({"result":r.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)})));}
        if METHODS.contains(&method){writes.fetch_add(1,Ordering::SeqCst);let mut records=r.lock().unwrap();let mut result=Value::Null;
            match method{
                "pool.enable_external_auth"=>{assert_eq!(p[1],"OpaqueRef:pool");assert_eq!(p[2],json!({"user":"directory-admin","pass":SECRET}));assert_eq!(p[3],"example.test");assert_eq!(p[4],"AD");if fault!=Fault::Partial{records.get_mut("host").unwrap()["external_auth_type"]=json!("AD");records.get_mut("host").unwrap()["external_auth_service_name"]=p[3].clone();}},
                "pool.disable_external_auth"=>{assert_eq!(p[2],json!({}));records.get_mut("host").unwrap()["external_auth_type"]=json!("");records.get_mut("host").unwrap()["external_auth_service_name"]=json!("");},
                "subject.create"=>{assert_eq!(p[1],json!({"subject_identifier":"S-fixture-new","other_config":{}}));records.insert("subject-new".into(),json!({"uuid":NEW,"subject_identifier":"S-fixture-new","other_config":{},"roles":if fault==Fault::UnexpectedRoles{json!(["OpaqueRef:role"])}else{json!([])}}));result=json!("OpaqueRef:subject-new");},
                "subject.destroy"=>{records.remove("subject");},
                "subject.add_to_roles"=>{assert_eq!(p[2],"OpaqueRef:role");records.get_mut("subject").unwrap()["roles"]=json!(["OpaqueRef:role"]);},
                "subject.remove_from_roles"=>records.get_mut("subject").unwrap()["roles"]=json!([]),
                "host.install_server_certificate"=>{assert!(p[3].as_str().unwrap().starts_with("-----BEGIN PRIVATE KEY-----"));assert_eq!(p[4],"");*c.lock().unwrap()=p[2].as_str().unwrap().into();},
                "host.reset_server_certificate"=>*c.lock().unwrap()=pem().0,
                _=>unreachable!(),
            }
            return Some(if fault==Fault::DropReply{RpcReply::Drop}else{RpcReply::Json(json!({"result":result}))});
        }
        None
    }})).await;
    (mock, records, certificate)
}
async fn connect(mock: &Mock, backend: &Backend, readonly: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = readonly;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture-password".into()),
    );
    let mut w = c.watch();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !w.borrow().is_current() {
            w.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    c
}
fn target(c: &xenadmin_core::Connection) -> xenadmin_core::ObservedResource {
    c.latest()
        .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
        .unwrap()
}
async fn finished(op: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(Duration::from_secs(4), async {
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
async fn inspect(
    c: &xenadmin_core::Connection,
    q: PoolSecurityQuery,
) -> Result<PoolSecurityResult, String> {
    let h = c.inspect_pool_security(target(c), q).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(v) = h.latest() {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn expected(joined: bool) -> Vec<DirectoryHostState> {
    vec![DirectoryHostState {
        host: HostId::parse(HOST).unwrap(),
        auth_type: if joined { "AD" } else { "" }.into(),
        service_name: if joined { "example.test" } else { "" }.into(),
    }]
}
fn join(c: &xenadmin_core::Connection) -> ResourceCommand {
    ResourceCommand::PoolSensitive {
        target: target(c),
        change: PoolSensitiveChange::JoinDirectory {
            domain: "example.test".into(),
            expected_hosts: expected(false),
            credentials: Credentials::new("directory-admin".into(), SECRET.into()),
        },
    }
}
fn plain(c: &xenadmin_core::Connection, op: SecurityOp) -> ResourceCommand {
    ResourceCommand::Pool {
        target: target(c),
        operation: PoolOperation::Security(op),
    }
}
fn mutations(mock: &Mock) -> usize {
    mock.methods
        .lock()
        .unwrap()
        .iter()
        .filter(|m| METHODS.contains(&m.as_str()))
        .count()
}
fn privacy(record: &ResourceOperationRecord, dir: &std::path::Path) {
    let output = serde_json::to_string(record).unwrap();
    assert!(
        !output.contains(SECRET)
            && !output.contains("OpaqueRef")
            && !output.contains("PRIVATE KEY")
    );
    let connection = rusqlite::Connection::open_with_flags(
        dir.join("operations.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection.busy_timeout(Duration::from_secs(5)).unwrap();
    let mut query=connection.prepare(
        "SELECT payload || COALESCE(task, '') AS evidence FROM operations UNION ALL SELECT payload || COALESCE(task, '') || COALESCE(result_reference, '') FROM resource_operations"
    ).unwrap();
    for value in query.query_map([], |row| row.get::<_, String>(0)).unwrap() {
        let value = value.unwrap();
        assert!(!value.contains(SECRET) && !value.contains("PRIVATE KEY"));
    }
}
#[tokio::test]
async fn security_discovery_is_read_only_and_projects_identities_without_raw_fields() {
    let (mock, _, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, true).await;
    let PoolSecurityResult::Overview(v) = inspect(&c, PoolSecurityQuery::Overview).await.unwrap()
    else {
        panic!()
    };
    assert!(v.errors.is_empty());
    assert!(v.is_local_superuser);
    assert_eq!(v.roles.len(), 1);
    assert_eq!(v.subjects[0].id, SubjectId::parse(SUBJECT).unwrap());
    let encoded = serde_json::to_string(&v).unwrap();
    assert!(!encoded.contains("OpaqueRef"));
    let resolved = inspect(
        &c,
        PoolSecurityQuery::ResolveSubject {
            name: "example\\alice".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(resolved,PoolSecurityResult::Subject{query,identifier,..}if query=="example\\alice"&&identifier=="S-fixture-new")
    );
    assert!(c.submit_resource(OperationId::new(), join(&c)).is_err());
    assert_eq!(mutations(&mock), 0);
}
#[tokio::test]
async fn directory_join_leave_use_transient_credentials_and_verify_members() {
    let (mock, _, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let r = finished(&c.submit_resource(OperationId::new(), join(&c)).unwrap()).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    privacy(&r, dir.path());
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::LeaveDirectory {
                    expected_hosts: expected(true),
                    remove_machine_accounts: false,
                },
            ),
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(mutations(&mock), 2);
}
#[tokio::test]
async fn directory_stale_members_and_missing_privilege_or_methods_refuse_before_write() {
    for fault in [Fault::None, Fault::Unsupported, Fault::NotRoot] {
        let (mock, records, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        if fault == Fault::None {
            records.lock().unwrap().get_mut("host").unwrap()["external_auth_service_name"] =
                json!("changed");
        }
        let r = finished(&c.submit_resource(OperationId::new(), join(&c)).unwrap()).await;
        assert_eq!(r.state, OperationState::Rejected);
        assert_eq!(mutations(&mock), 0);
    }
}
#[tokio::test]
async fn directory_partial_or_lost_reply_remains_unknown_without_compensation_or_replay() {
    for fault in [Fault::DropReply, Fault::Partial] {
        let (mock, _, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        let h = c.submit_resource(OperationId::new(), join(&c)).unwrap();
        let r = finished(&h).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown);
        privacy(&r, dir.path());
        assert_eq!(recheck(&c, &r).await.state, OperationState::OutcomeUnknown);
        assert_eq!(mutations(&mock), 1);
    }
}
#[tokio::test]
async fn security_completed_receipt_recovers_read_failure_without_credentials_or_replay() {
    let (mock, _, _) = fixture(Fault::VerifyFailure).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let r = finished(&c.submit_resource(OperationId::new(), join(&c)).unwrap()).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    privacy(&r, dir.path());
    let r = recheck(&c, &r).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(mutations(&mock), 1);
}
#[tokio::test]
async fn subject_creation_requires_explicit_rbac_and_verifies_no_automatic_privileges() {
    for (fault, restricted) in [
        (Fault::None, true),
        (Fault::None, false),
        (Fault::UnexpectedRoles, false),
    ] {
        let (mock, records, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connect(&mock, &b, false).await;
        {
            let mut r = records.lock().unwrap();
            r.get_mut("host").unwrap()["external_auth_type"] = json!("AD");
            if restricted {
                r.get_mut("host").unwrap()["license_params"] = json!({});
            }
        }
        let r = finished(
            &c.submit_resource(
                OperationId::new(),
                plain(
                    &c,
                    SecurityOp::AddSubject {
                        identifier: "S-fixture-new".into(),
                    },
                ),
            )
            .unwrap(),
        )
        .await;
        assert_eq!(
            r.state,
            if restricted {
                OperationState::Rejected
            } else if fault == Fault::UnexpectedRoles {
                OperationState::OutcomeUnknown
            } else {
                OperationState::Succeeded
            },
            "{}",
            r.detail
        );
        assert_eq!(mutations(&mock), usize::from(!restricted));
        privacy(&r, dir.path());
    }
}
#[tokio::test]
async fn subject_roles_use_exact_reviewed_role_set_and_reject_leaf_permissions() {
    let (mock, _, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let op = |role: &str, expected_roles, grant| SecurityOp::ChangeSubjectRole {
        subject: SubjectId::parse(SUBJECT).unwrap(),
        expected_identifier: "S-fixture-existing".into(),
        expected_roles,
        role: RoleId::parse(role).unwrap(),
        grant,
    };
    let r = finished(
        &c.submit_resource(OperationId::new(), plain(&c, op(LEAF, vec![], true)))
            .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert_eq!(mutations(&mock), 0);
    let r = finished(
        &c.submit_resource(OperationId::new(), plain(&c, op(ROLE, vec![], true)))
            .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::RemoveSubject {
                    subject: SubjectId::parse(SUBJECT).unwrap(),
                    expected_identifier: "S-fixture-existing".into(),
                    expected_roles: vec![],
                },
            ),
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert_eq!(mutations(&mock), 1);
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            plain(&c, op(ROLE, vec![RoleId::parse(ROLE).unwrap()], false)),
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded);
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::RemoveSubject {
                    subject: SubjectId::parse(SUBJECT).unwrap(),
                    expected_identifier: "S-fixture-existing".into(),
                    expected_roles: vec![],
                },
            ),
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded);
}
#[tokio::test]
async fn certificate_install_reset_recheck_fingerprints_and_never_persist_key() {
    let (mock, _, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let host = HostId::parse(HOST).unwrap();
    let PoolSecurityResult::Certificate {
        fingerprint: old, ..
    } = inspect(&c, PoolSecurityQuery::Certificate { host })
        .await
        .unwrap()
    else {
        panic!()
    };
    let (cert, key) = pem();
    let cert = ServerCertificate::new(cert, key, String::new()).unwrap();
    let new = cert.fingerprint();
    let cmd = ResourceCommand::PoolSensitive {
        target: target(&c),
        change: PoolSensitiveChange::InstallCertificate {
            host,
            expected: old.clone(),
            certificate: cert,
        },
    };
    let r = finished(&c.submit_resource(OperationId::new(), cmd).unwrap()).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    privacy(&r, dir.path());
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::ResetCertificate {
                    host,
                    expected: old,
                },
            ),
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Rejected);
    assert_eq!(mutations(&mock), 1);
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::ResetCertificate {
                    host,
                    expected: new,
                },
            ),
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::Succeeded);
    assert_eq!(mutations(&mock), 2);
}
#[tokio::test]
async fn lost_certificate_install_reply_does_not_claim_completion_from_matching_certificate() {
    let (mock, _, _) = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let host = HostId::parse(HOST).unwrap();
    let PoolSecurityResult::Certificate {
        fingerprint: expected,
        ..
    } = inspect(&c, PoolSecurityQuery::Certificate { host })
        .await
        .unwrap()
    else {
        panic!()
    };
    let (cert, key) = pem();
    let r = finished(
        &c.submit_resource(
            OperationId::new(),
            ResourceCommand::PoolSensitive {
                target: target(&c),
                change: PoolSensitiveChange::InstallCertificate {
                    host,
                    expected,
                    certificate: ServerCertificate::new(cert, key, String::new()).unwrap(),
                },
            },
        )
        .unwrap(),
    )
    .await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    privacy(&r, dir.path());
    let r = recheck(&c, &r).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(mutations(&mock), 1);
}

async fn recheck(
    c: &xenadmin_core::Connection,
    old: &ResourceOperationRecord,
) -> ResourceOperationRecord {
    c.control_resource(old.id, OperationControl::Recheck)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(record) = c.resource_history().iter().find(|v| {
                v.id == old.id
                    && v.revision() > old.revision()
                    && (v.state.terminal() || v.state == OperationState::OutcomeUnknown)
            }) {
                return record.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn security_discovery_reports_missing_sections_without_exposing_bad_record_data() {
    let (mock, records, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, true).await;
    {
        let mut r = records.lock().unwrap();
        r.get_mut("host")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("external_auth_type");
        r.get_mut("role").unwrap()["name_label"] = json!(format!("{SECRET}\u{001b}"));
    }
    let PoolSecurityResult::Overview(v) = inspect(&c, PoolSecurityQuery::Overview).await.unwrap()
    else {
        panic!()
    };
    assert!(v.errors.contains_key("directory") && v.errors.contains_key("roles"));
    assert!(v.directory_hosts.is_empty() && v.roles.is_empty() && v.subjects.is_empty());
    assert!(!serde_json::to_string(&v).unwrap().contains(SECRET));
    assert_eq!(mutations(&mock), 0);
}
#[tokio::test]
async fn secret_requiring_commands_cannot_be_submitted_through_serializable_action() {
    let (mock, _, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    assert!(
        c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::JoinDirectory {
                    domain: "example.test".into(),
                    expected_hosts: expected(false)
                }
            )
        )
        .is_err()
    );
    assert!(
        c.submit_resource(
            OperationId::new(),
            plain(
                &c,
                SecurityOp::LeaveDirectory {
                    expected_hosts: expected(true),
                    remove_machine_accounts: true
                }
            )
        )
        .is_err()
    );
    assert_eq!(mutations(&mock), 0);
}
#[tokio::test]
async fn reconnect_does_not_restore_or_replay_directory_credentials() {
    let (mock, _, _) = fixture(Fault::DropReply).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connect(&mock, &b, false).await;
    let r = finished(&c.submit_resource(OperationId::new(), join(&c)).unwrap()).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    c.close().await;
    let next = connect(&mock, &b, false).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if next
                .resource_history()
                .iter()
                .any(|v| v.id == r.id && v.state == OperationState::OutcomeUnknown)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let r = recheck(&next, &r).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    privacy(&r, dir.path());
    assert_eq!(mutations(&mock), 1);
}
