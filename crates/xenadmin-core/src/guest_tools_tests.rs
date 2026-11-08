use super::*;
use crate::{DeviceOperation, xapi::inventory_tests::Fixture};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Mutex};
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Calls = Arc<Mutex<Vec<String>>>;
fn uuid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
#[derive(Clone, Copy)]
enum Fault {
    None,
    LostEject,
    LostInsert,
    RefuseInsert,
    ChangeAfterEject,
}
async fn fixture(occupied: bool, fault: Fault) -> (Fixture, Records, Calls) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "OpaqueRef:VM:guest".into(),
            json!({"uuid":uuid(2),"power_state":"Running","resident_on":"OpaqueRef:host:host","is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,"current_operations":{}}),
        ),
        (
            "OpaqueRef:SR:tools".into(),
            json!({"uuid":uuid(3),"content_type":"iso"}),
        ),
        (
            "OpaqueRef:VDI:tools".into(),
            json!({"uuid":uuid(4),"name_label":"guest-tools.iso","is_tools_iso":true,"SR":"OpaqueRef:SR:tools","read_only":true,"missing":false,"current_operations":{}}),
        ),
        (
            "OpaqueRef:VDI:old".into(),
            json!({"uuid":uuid(6),"SR":"OpaqueRef:SR:tools","name_label":"installer.iso"}),
        ),
        (
            "OpaqueRef:VBD:cd".into(),
            json!({"uuid":uuid(5),"VM":"OpaqueRef:VM:guest","type":"CD","empty":!occupied,"VDI":if occupied{"OpaqueRef:VDI:old"}else{"OpaqueRef:NULL"},"current_operations":{},"allowed_operations":if occupied{vec!["eject"]}else{vec!["insert"]}}),
        ),
        (
            "OpaqueRef:host:host".into(),
            json!({"uuid":uuid(7),"enabled":true}),
        ),
        (
            "OpaqueRef:PBD:tools".into(),
            json!({"uuid":uuid(8),"SR":"OpaqueRef:SR:tools","host":"OpaqueRef:host:host","currently_attached":true}),
        ),
    ])));
    let calls: Calls = Default::default();
    let fixture=Fixture::new({let records=records.clone();let calls=calls.clone();move|r|{
        let method=r["method"].as_str().unwrap();calls.lock().unwrap().push(method.into());let mut records=records.lock().unwrap();
        if let Some(class)=method.strip_suffix(".get_by_uuid"){return records.iter().find(|(reference,record)|reference.starts_with(&format!("OpaqueRef:{class}:"))&&record["uuid"]==r["params"][1]).map_or_else(||json!({"error":{"message":"UUID_INVALID"}}),|(reference,_)|json!({"result":reference}));}
        if let Some(class)=method.strip_suffix(".get_all_records"){return json!({"result":records.iter().filter(|(reference,_)|reference.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>()});}
        if method.ends_with(".get_record"){return records.get(r["params"][1].as_str().unwrap()).map_or_else(||json!({"error":{"message":"HANDLE_INVALID"}}),|record|json!({"result":record}));}
        match method{
            "VBD.eject"=>{assert_eq!(r["params"][1],"OpaqueRef:VBD:cd");let cd=records.get_mut("OpaqueRef:VBD:cd").unwrap();cd["empty"]=json!(true);cd["VDI"]=json!("OpaqueRef:NULL");cd["allowed_operations"]=json!(["insert"]);if matches!(fault,Fault::ChangeAfterEject){records.get_mut("OpaqueRef:VDI:tools").unwrap()["missing"]=json!(true);}
if matches!(fault,Fault::LostEject){json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}}
            "VBD.insert"=>{assert_eq!(r["params"][1],"OpaqueRef:VBD:cd");assert_eq!(r["params"][2],"OpaqueRef:VDI:tools");if matches!(fault,Fault::RefuseInsert){return json!({"error":{"message":"SR_BACKEND_FAILURE"}});}let cd=records.get_mut("OpaqueRef:VBD:cd").unwrap();assert_eq!(cd["empty"],true);cd["empty"]=json!(false);cd["VDI"]=json!("OpaqueRef:VDI:tools");if matches!(fault,Fault::LostInsert){json!({"_fixture_close_without_reply":true})}else{json!({"result":null})}}
            _=>panic!("Unexpected call {method}"),
        }
    }}).await;
    (fixture, records, calls)
}
fn operation(occupied: bool) -> DeviceOperation {
    DeviceOperation::MountGuestTools {
        vbd: uuid(5).into(),
        image: uuid(4).into(),
        sr: uuid(3).into(),
        previous: occupied.then(|| uuid(6).into()),
    }
}
async fn execute(f: &Fixture, path: &Path, action: DeviceOperation) -> OperationRecord {
    let (handle, request) = channel(
        OperationId::new(),
        uuid(1).into(),
        uuid(2).into(),
        1,
        OperationAction::Device(action),
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(128);
    tokio::time::timeout(
        Duration::from_secs(5),
        request.run(
            Arc::new(f.session().await),
            Some("OpaqueRef:VM:guest".into()),
            path.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*handle.latest()).clone()
}
async fn recheck(f: &Fixture, path: &Path, record: OperationRecord) -> OperationRecord {
    let id = record.id;
    let (tx, _rx) = mpsc::channel(128);
    MutationRequest::tracking(record)
        .run(
            Arc::new(f.session().await),
            None,
            path.into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    Journal::open(path).unwrap().load(id).unwrap().unwrap()
}
fn writes(c: &Calls) -> Vec<String> {
    c.lock()
        .unwrap()
        .iter()
        .filter(|m| matches!(m.as_str(), "VBD.eject" | "VBD.insert"))
        .cloned()
        .collect()
}
#[tokio::test]
async fn tools_mounts_exact_media_into_empty_or_reviewed_occupied_drive() {
    for occupied in [false, true] {
        let (f, records, calls) = fixture(occupied, Fault::None).await;
        let dir = tempfile::tempdir().unwrap();
        let record = execute(&f, dir.path(), operation(occupied)).await;
        assert_eq!(record.state, OperationState::Succeeded, "{}", record.detail);
        assert_eq!(
            writes(&calls),
            if occupied {
                vec!["VBD.eject", "VBD.insert"]
            } else {
                vec!["VBD.insert"]
            }
        );
        assert_eq!(
            records.lock().unwrap()["OpaqueRef:VBD:cd"]["VDI"],
            "OpaqueRef:VDI:tools"
        );
        assert!(record.created_devices.is_empty());
    }
}
#[tokio::test]
async fn tools_rechecks_media_and_keeps_partial_or_unknown_outcomes_without_replay() {
    for fault in [
        Fault::LostEject,
        Fault::LostInsert,
        Fault::RefuseInsert,
        Fault::ChangeAfterEject,
    ] {
        let (f, _records, calls) = fixture(true, fault).await;
        let dir = tempfile::tempdir().unwrap();
        let record = execute(&f, dir.path(), operation(true)).await;
        assert_ne!(record.state, OperationState::Succeeded);
        let before = writes(&calls);
        if matches!(fault, Fault::LostEject | Fault::ChangeAfterEject) {
            assert_eq!(before, vec!["VBD.eject"]);
        }
        let _ = recheck(&f, dir.path(), record).await;
        assert_eq!(writes(&calls), before);
    }
}
#[tokio::test]
async fn tools_changed_drive_source_repository_or_guest_fail_before_eject() {
    for (reference, field, value) in [
        ("OpaqueRef:VBD:cd", "VM", json!("OpaqueRef:foreign")),
        ("OpaqueRef:VBD:cd", "VDI", json!("OpaqueRef:VDI:tools")),
        ("OpaqueRef:VDI:tools", "missing", json!(true)),
        ("OpaqueRef:VDI:tools", "read_only", json!(false)),
        ("OpaqueRef:PBD:tools", "currently_attached", json!(false)),
        ("OpaqueRef:VM:guest", "is_a_template", json!(true)),
        (
            "OpaqueRef:VM:guest",
            "current_operations",
            json!({"OpaqueRef:task":"clone"}),
        ),
        ("OpaqueRef:SR:tools", "content_type", json!("user")),
    ] {
        let (f, records, calls) = fixture(true, Fault::None).await;
        records.lock().unwrap().get_mut(reference).unwrap()[field] = value;
        let dir = tempfile::tempdir().unwrap();
        let record = execute(&f, dir.path(), operation(true)).await;
        assert_ne!(
            record.state,
            OperationState::Succeeded,
            "{reference}.{field}"
        );
        assert!(writes(&calls).is_empty(), "{reference}.{field}");
    }
}
