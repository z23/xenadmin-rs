use super::*;
use crate::xapi::inventory_tests::Fixture;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn vm() -> VmId {
    uuid(2).into()
}
fn cd() -> VbdId {
    uuid(3).into()
}
fn vbd() -> VbdId {
    uuid(4).into()
}
fn image() -> VdiId {
    uuid(5).into()
}
fn disk() -> VdiId {
    uuid(6).into()
}
fn nic() -> VifId {
    uuid(7).into()
}

type Records = Arc<Mutex<BTreeMap<String, Value>>>;
type Requests = Arc<Mutex<Vec<Value>>>;

async fn fixture() -> (Fixture, Records, Requests) {
    fixture_with_loss(false).await
}

async fn fixture_with_loss(drop_mutation_reply: bool) -> (Fixture, Records, Requests) {
    let records: Records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "OpaqueRef:VM:vm".into(),
            json!({"uuid":uuid(2),"is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,"power_state":"Running"}),
        ),
        (
            "OpaqueRef:VBD:cd".into(),
            json!({"uuid":uuid(3),"VM":"OpaqueRef:VM:vm","type":"CD","empty":true,"allowed_operations":["insert","eject"]}),
        ),
        (
            "OpaqueRef:VBD:disk".into(),
            json!({"uuid":uuid(4),"VM":"OpaqueRef:VM:vm","VDI":"OpaqueRef:VDI:disk","type":"Disk","currently_attached":false,"allowed_operations":["plug","unplug"]}),
        ),
        (
            "OpaqueRef:VDI:image".into(),
            json!({"uuid":uuid(5),"SR":"OpaqueRef:SR:iso","missing":false}),
        ),
        (
            "OpaqueRef:VDI:disk".into(),
            json!({"uuid":uuid(6),"type":"user","virtual_size":"1073741824","read_only":false,"sharable":false,"managed":true,"missing":false,"is_a_snapshot":false,"allowed_operations":["resize"],"VBDs":["OpaqueRef:VBD:disk"]}),
        ),
        (
            "OpaqueRef:VIF:nic".into(),
            json!({"uuid":uuid(7),"VM":"OpaqueRef:VM:vm","currently_attached":false,"allowed_operations":["plug","unplug"]}),
        ),
        (
            "OpaqueRef:SR:iso".into(),
            json!({"uuid":uuid(8),"content_type":"iso"}),
        ),
    ])));
    let mutations = Arc::new(Mutex::new(Vec::new()));
    let server = Fixture::new({
        let records = records.clone();
        let mutations = mutations.clone();
        move |request| {
            let method = request["method"].as_str().unwrap();
            if method.starts_with("Async.") {
                mutations.lock().unwrap().push(request.clone());
                if drop_mutation_reply {
                    return json!({"_fixture_close_without_reply":true});
                }
                return json!({"result":"OpaqueRef:device-task"});
            }
            let records = records.lock().unwrap();
            match method {
                "event.from" => {
                    let events = if request["params"][2] == "" {
                        let mut events = vec![json!({"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":uuid(1),"name_label":"Device fixture pool"}})];
                        events.extend(records.iter().map(|(reference, record)| json!({"class":reference.split(':').nth(1).unwrap(),"operation":"add","ref":reference,"snapshot":record})));
                        events
                    } else { vec![] };
                    return json!({"result":{"token":"fixture","events":events}});
                }
                "task.get_record" => return json!({"result":{"uuid":uuid(30),"status":"success","progress":1.0,"allowed_operations":[]}}),
                "task.destroy" | "session.logout" => return json!({"result":null}),
                _ => {}
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid") {
                return json!({"result":records.iter().find(|(reference, record)| reference.starts_with(&format!("OpaqueRef:{class}:")) && record["uuid"] == request["params"][1]).unwrap().0});
            }
            if method.ends_with(".get_record") {
                return json!({"result":records[request["params"][1].as_str().unwrap()]});
            }
            panic!("unexpected device fixture request");
        }
    }).await;
    (server, records, mutations)
}
fn change(records: &Records, reference: &str, field: &str, value: Value) {
    records.lock().unwrap().get_mut(reference).unwrap()[field] = value;
}

#[tokio::test]
async fn cd_and_attachment_commands_dispatch_exactly_one_async_task_each() {
    let (server, records, requests) = fixture().await;
    let session = server.session().await;
    for (operation, expected) in [
        (
            DeviceOperation::InsertCd {
                vbd: cd(),
                image: image(),
            },
            "Async.VBD.insert",
        ),
        (DeviceOperation::PlugDisk { vbd: vbd() }, "Async.VBD.plug"),
        (DeviceOperation::PlugNic { vif: nic() }, "Async.VIF.plug"),
    ] {
        let prepared = prepare_operation(&session, vm(), &operation).await.unwrap();
        assert_eq!(
            prepared
                .send(&session, &CancellationToken::new())
                .await
                .unwrap(),
            "OpaqueRef:device-task"
        );
        assert_eq!(requests.lock().unwrap().last().unwrap()["method"], expected);
    }
    change(&records, "OpaqueRef:VBD:cd", "empty", json!(false));
    change(
        &records,
        "OpaqueRef:VBD:disk",
        "currently_attached",
        json!(true),
    );
    change(
        &records,
        "OpaqueRef:VIF:nic",
        "currently_attached",
        json!(true),
    );
    for (operation, expected) in [
        (DeviceOperation::EjectCd { vbd: cd() }, "Async.VBD.eject"),
        (
            DeviceOperation::UnplugDisk { vbd: vbd() },
            "Async.VBD.unplug",
        ),
        (
            DeviceOperation::UnplugNic { vif: nic() },
            "Async.VIF.unplug",
        ),
    ] {
        prepare_operation(&session, vm(), &operation)
            .await
            .unwrap()
            .send(&session, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(requests.lock().unwrap().last().unwrap()["method"], expected);
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 6);
    assert_eq!(requests[0]["params"][1], "OpaqueRef:VBD:cd");
    assert_eq!(requests[0]["params"][2], "OpaqueRef:VDI:image");
}

#[tokio::test]
async fn foreign_devices_busy_devices_and_non_iso_media_are_rejected_without_dispatch() {
    let (server, records, requests) = fixture().await;
    let session = server.session().await;
    change(
        &records,
        "OpaqueRef:VIF:nic",
        "VM",
        json!("OpaqueRef:VM:someone-else"),
    );
    assert!(matches!(
        prepare_operation(&session, vm(), &DeviceOperation::PlugNic { vif: nic() }).await,
        Err(Error::Stale)
    ));
    change(
        &records,
        "OpaqueRef:VBD:disk",
        "current_operations",
        json!({"OpaqueRef:task":"unplug"}),
    );
    assert!(matches!(
        prepare_operation(&session, vm(), &DeviceOperation::PlugDisk { vbd: vbd() }).await,
        Err(Error::Busy)
    ));
    change(&records, "OpaqueRef:SR:iso", "content_type", json!("user"));
    assert!(matches!(
        prepare_operation(
            &session,
            vm(),
            &DeviceOperation::InsertCd {
                vbd: cd(),
                image: image()
            }
        )
        .await,
        Err(Error::DeviceInUse)
    ));
    change(&records, "OpaqueRef:VBD:cd", "empty", json!(false));
    assert!(matches!(
        prepare_operation(
            &session,
            vm(),
            &DeviceOperation::InsertCd {
                vbd: cd(),
                image: image()
            }
        )
        .await,
        Err(Error::DeviceConflict)
    ));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn disk_growth_checks_current_size_every_consumer_and_halted_state() {
    let (server, records, requests) = fixture().await;
    let session = server.session().await;
    let operation = DeviceOperation::GrowDisk {
        vdi: disk(),
        expected_size: 1_073_741_824,
        new_size: 2_147_483_648,
    };
    assert!(matches!(
        prepare_operation(&session, vm(), &operation).await,
        Err(Error::DeviceRequiresHaltedVm)
    ));
    change(&records, "OpaqueRef:VM:vm", "power_state", json!("Halted"));
    change(
        &records,
        "OpaqueRef:VDI:disk",
        "virtual_size",
        json!("2147483648"),
    );
    assert!(matches!(
        prepare_operation(&session, vm(), &operation).await,
        Err(Error::DeviceConflict)
    ));
    change(
        &records,
        "OpaqueRef:VDI:disk",
        "virtual_size",
        json!("1073741824"),
    );
    change(
        &records,
        "OpaqueRef:VBD:disk",
        "VM",
        json!("OpaqueRef:VM:someone-else"),
    );
    assert!(matches!(
        prepare_operation(&session, vm(), &operation).await,
        Err(Error::DeviceInUse)
    ));
    change(
        &records,
        "OpaqueRef:VBD:disk",
        "VM",
        json!("OpaqueRef:VM:vm"),
    );
    for (field, value) in [
        ("sharable", json!(true)),
        ("read_only", json!(true)),
        ("managed", json!(false)),
        ("is_a_snapshot", json!(true)),
        ("missing", json!(true)),
    ] {
        let previous = records.lock().unwrap()["OpaqueRef:VDI:disk"][field].clone();
        change(&records, "OpaqueRef:VDI:disk", field, value);
        assert!(matches!(
            prepare_operation(&session, vm(), &operation).await,
            Err(Error::DeviceInUse)
        ));
        change(&records, "OpaqueRef:VDI:disk", field, previous);
    }
    assert!(requests.lock().unwrap().is_empty());
    prepare_operation(&session, vm(), &operation)
        .await
        .unwrap()
        .send(&session, &CancellationToken::new())
        .await
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "Async.VDI.resize");
    assert_eq!(requests[0]["params"][2], "2147483648");
}

#[tokio::test]
async fn cancelled_prepared_device_never_dispatches() {
    let (server, _, requests) = fixture().await;
    let session = server.session().await;
    let prepared = prepare_operation(&session, vm(), &DeviceOperation::PlugNic { vif: nic() })
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        prepared.send(&session, &cancel).await,
        Err(Error::Closed)
    ));
    assert!(requests.lock().unwrap().is_empty());
}

#[test]
fn resize_rejects_shrink_zero_unaligned_and_out_of_range_sizes() {
    for (expected_size, new_size) in [(1024, 512), (0, 1024), (1024, 1025), (1024, u64::MAX)] {
        assert!(
            DeviceOperation::GrowDisk {
                vdi: disk(),
                expected_size,
                new_size
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        DeviceOperation::GrowDisk {
            vdi: disk(),
            expected_size: 1024,
            new_size: 2048
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn advisory_device_eligibility_keeps_unknown_metadata_distinct_from_denial() {
    let event = |class: &str, reference: &str, record: Value| json!({"class":class,"operation":"add","ref":reference,"snapshot":record});
    let batch: crate::xapi::EventBatch = serde_json::from_value(json!({"token":"fixture","events":[
        event("pool","OpaqueRef:pool",json!({"uuid":uuid(1)})),
        event("VM","OpaqueRef:vm",json!({"uuid":uuid(2),"power_state":"Halted"})),
        event("VDI","OpaqueRef:disk",json!({"uuid":uuid(6),"type":"user","virtual_size":"1073741824","read_only":false,"sharable":false,"managed":true,"missing":false,"is_a_snapshot":false,"allowed_operations":["resize"]})),
        event("VBD","OpaqueRef:vbd",json!({"uuid":uuid(4),"VM":"OpaqueRef:vm","VDI":"OpaqueRef:disk","type":"Disk","currently_attached":false})),
    ]})).unwrap();
    let mut inventory = crate::xapi::Store::default().apply(&batch).unwrap();
    let grow = DeviceOperation::GrowDisk {
        vdi: disk(),
        expected_size: 1_073_741_824,
        new_size: 2_147_483_648,
    };
    assert_eq!(
        grow.availability(&inventory, vm()),
        ActionAvailability::Available
    );
    inventory.details.vdis.get_mut(&disk()).unwrap().missing = None;
    assert_eq!(
        grow.availability(&inventory, vm()),
        ActionAvailability::Unknown
    );
    inventory.details.vdis.get_mut(&disk()).unwrap().missing = Some(true);
    assert_eq!(
        grow.availability(&inventory, vm()),
        ActionAvailability::Unavailable
    );
    inventory.details.vdis.get_mut(&disk()).unwrap().missing = Some(false);
    inventory.details.vbds.get_mut(&vbd()).unwrap().vm = Some(uuid(20).into());
    assert_eq!(
        grow.availability(&inventory, vm()),
        ActionAvailability::Unavailable
    );
}

async fn finish(operation: &crate::Operation) -> crate::OperationState {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = operation.latest().state.clone();
            if state.terminal() || state == crate::OperationState::OutcomeUnknown {
                return state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn device_task_is_tracked_and_foreign_owner_rejection_precedes_dispatch() {
    let (server, records, requests) = fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let backend = crate::Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        server.options(),
        crate::Credentials::new("fixture".into(), "fixture-password".into()),
    );
    connection
        .wait_ready(std::time::Duration::from_secs(3))
        .await
        .unwrap();
    let observed = connection.latest().observe_vm(vm()).unwrap();
    change(
        &records,
        "OpaqueRef:VIF:nic",
        "VM",
        json!("OpaqueRef:VM:someone-else"),
    );
    let foreign = connection
        .submit(
            crate::OperationId::new(),
            crate::VmCommand::Device(DeviceCommand {
                vm: observed,
                operation: DeviceOperation::PlugNic { vif: nic() },
            }),
        )
        .unwrap();
    assert_eq!(finish(&foreign).await, crate::OperationState::Rejected);
    assert!(requests.lock().unwrap().is_empty());
    change(
        &records,
        "OpaqueRef:VIF:nic",
        "VM",
        json!("OpaqueRef:VM:vm"),
    );
    let id = crate::OperationId::new();
    let operation = connection
        .submit(
            id,
            crate::VmCommand::Device(DeviceCommand {
                vm: observed,
                operation: DeviceOperation::PlugNic { vif: nic() },
            }),
        )
        .unwrap();
    assert_eq!(finish(&operation).await, crate::OperationState::Succeeded);
    assert_eq!(
        operation.latest().task.as_deref(),
        Some("OpaqueRef:device-task")
    );
    let duplicate = connection
        .submit(
            id,
            crate::VmCommand::Device(DeviceCommand {
                vm: observed,
                operation: DeviceOperation::PlugNic { vif: nic() },
            }),
        )
        .unwrap();
    assert_eq!(finish(&duplicate).await, crate::OperationState::Succeeded);
    assert_eq!(requests.lock().unwrap().len(), 1);
    connection.close().await;
}

#[tokio::test]
async fn lost_device_reply_stays_unknown_and_blocks_a_new_device_write() {
    let (server, _, requests) = fixture_with_loss(true).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = crate::Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = backend.connect(
        server.options(),
        crate::Credentials::new("fixture".into(), "fixture-password".into()),
    );
    connection
        .wait_ready(std::time::Duration::from_secs(3))
        .await
        .unwrap();
    let observed = connection.latest().observe_vm(vm()).unwrap();
    let operation = connection
        .submit(
            crate::OperationId::new(),
            crate::VmCommand::Device(DeviceCommand {
                vm: observed,
                operation: DeviceOperation::PlugNic { vif: nic() },
            }),
        )
        .unwrap();
    assert_eq!(
        finish(&operation).await,
        crate::OperationState::OutcomeUnknown,
        "{}",
        operation.latest().detail
    );
    let repeated = connection
        .submit(
            crate::OperationId::new(),
            crate::VmCommand::Device(DeviceCommand {
                vm: observed,
                operation: DeviceOperation::PlugNic { vif: nic() },
            }),
        )
        .unwrap();
    assert_eq!(finish(&repeated).await, crate::OperationState::Rejected);
    assert_eq!(requests.lock().unwrap().len(), 1);
    connection.close().await;
}
