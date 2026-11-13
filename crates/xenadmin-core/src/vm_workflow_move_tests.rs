use super::*;
use crate::VmMigration;
fn operation(live: bool) -> VmMigration {
    VmMigration {
        destination: Uuid::from_u128(11).into(),
        expected_host: live.then(|| Uuid::from_u128(10).into()),
        expected_affinity: Some(Uuid::from_u128(10).into()),
        live,
        disks: vec![],
        networks: vec![],
        transfer_network: (!live).then(|| Uuid::from_u128(12).into()),
    }
}
fn parent_move(live: bool) -> (crate::ResourceOperation, crate::resources::ResourceRequest) {
    let selection = VmWorkflowBatch {
        items: (2..4)
            .map(|id| BatchVmWorkflow {
                vm: vm(id),
                expected_state: if live {
                    PowerState::Running
                } else {
                    PowerState::Halted
                },
                workflow: VmWorkflow::Move(operation(live)),
            })
            .collect(),
    };
    crate::resources::channel(
        OperationId::new(),
        crate::ObservedResource {
            pool: pool(),
            generation: 4,
            target: ResourceTarget::Pool(pool()),
        },
        ResourceAction::VmWorkflowBatch(selection),
        tokio::runtime::Handle::current(),
    )
}
async fn fixture_move(
    live: bool,
    fault: &str,
    dir: &Path,
    parent: OperationId,
) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let mut records = BTreeMap::from([
        ("OpaqueRef:pool:p".to_owned(), json!({"uuid":pool()})),
        (
            "OpaqueRef:host:source".into(),
            json!({"uuid":Uuid::from_u128(10),"enabled":true,"metrics":"OpaqueRef:host_metrics:source","current_operations":{}}),
        ),
        (
            "OpaqueRef:host:destination".into(),
            json!({"uuid":Uuid::from_u128(11),"enabled":true,"metrics":"OpaqueRef:host_metrics:destination","current_operations":{}}),
        ),
        ("OpaqueRef:host_metrics:source".into(), json!({"live":true})),
        (
            "OpaqueRef:host_metrics:destination".into(),
            json!({"live":true}),
        ),
        (
            "OpaqueRef:network:transfer".into(),
            json!({"uuid":Uuid::from_u128(12),"PIFs":["OpaqueRef:PIF:transfer"],"current_operations":{}}),
        ),
        (
            "OpaqueRef:PIF:transfer".into(),
            json!({"uuid":Uuid::from_u128(13),"network":"OpaqueRef:network:transfer","host":"OpaqueRef:host:destination","currently_attached":true,"IP":"192.0.2.9"}),
        ),
    ]);
    for id in 2..4 {
        records.insert(format!("OpaqueRef:VM:{id}"),json!({"uuid":vm(id),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":if live{"Running"}else{"Halted"},"resident_on":if live{"OpaqueRef:host:source"}else{"OpaqueRef:NULL"},"affinity":"OpaqueRef:host:source","VBDs":[],"VIFs":[],"snapshots":[],"allowed_operations":["pool_migrate","migrate_send"],"blocked_operations":{},"current_operations":{}}));
    }
    if fault == "preflight" {
        records.get_mut("OpaqueRef:VM:3").unwrap()["power_state"] = json!("Suspended");
    }
    let records = Arc::new(Mutex::new(records));
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = calls.clone();
    let dir = dir.to_owned();
    let fault = fault.to_owned();
    let control = Arc::new(Mutex::new((false, 0usize)));
    let f=Fixture::new(move|request| {
        let method=request["method"].as_str().unwrap();let params=&request["params"];
        log.lock().unwrap().push(method.into());
        let mut records=records.lock().unwrap();let mut control=control.lock().unwrap();
        let interrupt=|ack:bool| {let j=Journal::open(&dir).unwrap();j.workflow_interrupt(pool(),parent,false).unwrap();if ack {j.workflow_interrupt(pool(),parent,true).unwrap();}};
        if method=="VM.get_record" && params[1]=="OpaqueRef:VM:2" && control.0 {
            control.1+=1;
            if fault=="recover_affinity" && control.1==3 {interrupt(false);}
        }
        if let Some(class)=method.strip_suffix(".get_by_uuid") {
            return records.iter().find(|(k,v)|k.starts_with(&format!("OpaqueRef:{class}:"))&&v["uuid"]==params[1]).map_or(json!({"error":{"message":"UUID_INVALID"}}),|(key,_)|json!({"result":key}));
        }
        if method=="host.migrate_receive" {return json!({"result":{"host":"OpaqueRef:host:destination","session_id":"OpaqueRef:PRIVATE-RECEIVER-TOKEN"}});}
        if matches!(method,"VM.assert_can_boot_here"|"VM.assert_can_migrate") {return json!({"result":null});}
        if method.starts_with("Async.VM.") {
            let vm=params[1].as_str().unwrap();
            if live {records.get_mut(vm).unwrap()["resident_on"]=json!("OpaqueRef:host:destination");}
            if vm=="OpaqueRef:VM:2" {
                if fault=="stop" {Journal::open(&dir).unwrap().resource_control(pool(),parent,OperationControl::RequestCancellation).unwrap();}
                if fault=="recover_reply"||fault=="ack_reply" {interrupt(fault=="ack_reply");}
                if fault=="lost" {return json!({"_fixture_close_without_reply":true});}
            }
            return json!({"result":format!("OpaqueRef:task:{}",if vm.ends_with('2'){2}else{3})});
        }
        if method=="task.get_record" {
            let first=params[1]=="OpaqueRef:task:2";
            if first && !control.0 && matches!(fault.as_str(),"recover_success"|"ack_success"|"other_affinity") {
                interrupt(fault=="ack_success");
                if fault=="other_affinity" {records.get_mut("OpaqueRef:VM:2").unwrap()["affinity"]=json!("OpaqueRef:host:destination");}
            }
            if first {control.0=true;}
            return json!({"result":{"status":"success","progress":1.0,"allowed_operations":[],"result":format!("<value><string>OpaqueRef:VM:{}</string></value>",if first{2}else{3})}});
        }
        if method=="VM.set_affinity" {records.get_mut(params[1].as_str().unwrap()).unwrap()["affinity"]=params[2].clone();return json!({"result":null});}
        if method=="task.destroy" {return json!({"result":null});}
        if method.ends_with(".get_record") {return records.get(params[1].as_str().unwrap()).map_or(json!({"error":{"message":"HANDLE_INVALID"}}),|v|json!({"result":v}));}
        panic!("unexpected migration batch RPC {method}")
    }).await;
    (f, calls)
}
fn count(calls: &Arc<Mutex<Vec<String>>>, method: &str) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|v| v.as_str() == method)
        .count()
}
#[tokio::test]
async fn batch_moves_preflight_entire_selection_without_receiver_tokens() {
    for live in [false, true] {
        for fault in ["none", "preflight", "stop"] {
            let dir = tempfile::tempdir().unwrap();
            let (view, p) = parent_move(live);
            let (f, calls) = fixture_move(live, fault, dir.path(), p.record.id).await;
            run(p, &f, dir.path()).await;
            let r = view.latest();
            let n = match fault {
                "preflight" => 0,
                "stop" => 1,
                _ => 2,
            };
            assert_eq!(
                r.state,
                match fault {
                    "preflight" => OperationState::Rejected,
                    "stop" => OperationState::PartiallyCompleted,
                    _ => OperationState::Succeeded,
                },
                "{live}/{fault}: {}",
                r.detail
            );
            assert_eq!(
                count(
                    &calls,
                    if live {
                        "Async.VM.pool_migrate"
                    } else {
                        "Async.VM.migrate_send"
                    }
                ),
                n
            );
            assert_eq!(
                count(&calls, "host.migrate_receive"),
                if live { 0 } else { n }
            );
            assert_eq!(count(&calls, "VM.set_affinity"), if live { 0 } else { n });
        }
    }
}
#[tokio::test]
async fn batch_move_recovery_and_ack_keep_task_receipts_without_next_vm_or_home_host_write() {
    for fault in [
        "lost",
        "recover_reply",
        "ack_reply",
        "recover_success",
        "ack_success",
        "recover_affinity",
        "other_affinity",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (_, p) = parent_move(false);
        let id = p.record.id;
        let (f, calls) = fixture_move(false, fault, dir.path(), id).await;
        run(p, &f, dir.path()).await;
        let j = Journal::open(dir.path()).unwrap();
        let parent = j.resource_load(id).unwrap().unwrap();
        assert_eq!(
            parent.state,
            if fault.starts_with("ack") {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{fault}: {}",
            parent.detail
        );
        let plan = parent.workflow_batch_plan.as_ref().unwrap();
        assert_eq!(plan.admitted, 1);
        let child = j.load(plan.children[0]).unwrap().unwrap();
        if fault != "lost" {
            assert!(child.task.is_some());
        }
        if matches!(
            fault,
            "recover_success" | "ack_success" | "recover_affinity" | "other_affinity"
        ) {
            assert!(child.migration_plan.as_ref().unwrap().completed);
        }
        assert_eq!(count(&calls, "host.migrate_receive"), 1);
        assert_eq!(count(&calls, "Async.VM.migrate_send"), 1);
        assert_eq!(count(&calls, "VM.set_affinity"), 0);
        if !fault.starts_with("ack") {
            let before = calls.lock().unwrap().len();
            run(
                crate::resources::ResourceRequest::tracking(parent),
                &f,
                dir.path(),
            )
            .await;
            if fault == "other_affinity" {
                let parent = j.resource_load(id).unwrap().unwrap();
                assert_eq!(parent.state, OperationState::PartiallyCompleted);
                assert_eq!(parent.workflow_batch_plan.as_ref().unwrap().admitted, 1);
                let child = j.load(parent.workflow_batch[0].child).unwrap().unwrap();
                assert_eq!(child.state, OperationState::Succeeded);
                assert!(!child.migration_plan.as_ref().unwrap().affinity_received);
                assert!(child.steps[1].detail.contains("observed"));
                assert_eq!(count(&calls, "VM.set_affinity"), 0);
                assert_eq!(count(&calls, "Async.VM.migrate_send"), 1);
            }
            let log = calls.lock().unwrap();
            assert!(
                log[before..]
                    .iter()
                    .all(|m| m.ends_with(".get_record") || m.ends_with(".get_by_uuid")),
                "read-only recheck: {:?}",
                &log[before..]
            );
        }
    }
}
#[test]
fn mixed_batch_families_and_mismatched_move_states_are_refused() {
    let mut selection = selection();
    selection.items[0].workflow = VmWorkflow::Move(operation(false));
    assert!(selection.validate(ResourceTarget::Pool(pool())).is_err());
    selection.items[1].workflow = VmWorkflow::Move(operation(true));
    assert!(selection.validate(ResourceTarget::Pool(pool())).is_err());
}
