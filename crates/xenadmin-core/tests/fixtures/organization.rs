use super::*;
use std::collections::BTreeMap;
use xenadmin_core::{
    CustomFieldDefinition, CustomFieldType, OperationControl, OperationId, OperationState,
    OrganizationChange as Change, OrganizationEdit as Edit, OrganizationObjectId as Id,
    OrganizationPatch, PoolId, ResourceCommand, ResourceOperation, ResourceOperationRecord,
    ResourceTarget, VmId,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const VM: &str = "33333333-3333-3333-3333-333333333333";
const DEFINITIONS: &str = "<CustomFieldDefinitions><CustomFieldDefinition name=\"Owner\" type=\"String\" defaultValue=\"\"/></CustomFieldDefinitions>";
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    DropTags,
    FailMap,
    StaleTags,
    WrongType,
    StaleDefinition,
}
type Records = Arc<Mutex<BTreeMap<String, Value>>>;
async fn fixture(fault: Fault) -> (Mock, Records) {
    let mut vm = batch("initial", "Halted")["events"][2]["snapshot"].clone();
    vm["is_control_domain"] = json!(false);
    vm["is_a_template"] = json!(false);
    vm["is_a_snapshot"] = json!(false);
    vm["tags"] = json!(["service", "keep"]);
    vm["other_config"] = json!({"folder":"/Old/Child","unrelated":"preserve","XenCenter.CustomFields.Owner":"Alice"});
    let records = Arc::new(Mutex::new(BTreeMap::from([
        (
            "pool".into(),
            json!({"uuid":POOL,"name_label":"Pool","master":"OpaqueRef:host","tags":[],"other_config":{"EMPTY_FOLDERS":"/Old/Empty","pool-key":"preserve"},"gui_config":{"XenCenter.CustomFields":DEFINITIONS,"unrelated":"preserve"}}),
        ),
        ("VM".into(), vm),
    ])));
    let captured = records.clone();
    let callback = move |request: Value| {
        let records = captured.clone();
        async move {
            let method = request["method"].as_str().unwrap();
            if method == "event.from" && request["params"][2] == "" {
                let mut initial = batch("initial", "Halted");
                let mut map = records.lock().unwrap();
                for event in initial["events"].as_array_mut().unwrap() {
                    if let Some(r) = map.get(event["class"].as_str().unwrap()) {
                        event["snapshot"] = r.clone();
                    }
                }
                match fault {
                    Fault::StaleTags => map.get_mut("VM").unwrap()["tags"] = json!(["other"]),
                    Fault::WrongType => map.get_mut("VM").unwrap()["is_a_snapshot"] = json!(true),
                    Fault::StaleDefinition => {
                        map.get_mut("pool").unwrap()["gui_config"]["XenCenter.CustomFields"] =
                            json!("<CustomFieldDefinitions/>")
                    }
                    _ => {}
                }
                return Some(RpcReply::Json(json!({"result":initial})));
            }
            if let Some(class) = method.strip_suffix(".get_by_uuid")
                && ["pool", "VM"].contains(&class)
            {
                return Some(RpcReply::Json(
                    json!({"result":format!("OpaqueRef:{class}")}),
                ));
            }
            if let Some(class) = method.strip_suffix(".get_record")
                && ["pool", "VM"].contains(&class)
            {
                return Some(RpcReply::Json(
                    json!({"result":records.lock().unwrap()[class]}),
                ));
            }
            if let Some(class) = method.strip_suffix(".get_all_records")
                && ["pool", "VM", "host", "SR", "network", "VDI"].contains(&class)
            {
                let map = records.lock().unwrap();
                return Some(RpcReply::Json(
                    json!({"result":match map.get(class){Some(r)=>json!({format!("OpaqueRef:{class}"):r}),None=>json!({})}}),
                ));
            }
            if let Some((class, field)) = method.split_once(".set_")
                && ["pool", "VM"].contains(&class)
            {
                if fault == Fault::FailMap && field == "other_config" {
                    return Some(RpcReply::Json(
                        json!({"error":{"code":1,"message":"RBAC_PERMISSION_DENIED","data":[]}}),
                    ));
                }
                records.lock().unwrap().get_mut(class).unwrap()[field] =
                    request["params"][2].clone();
                return Some(if fault == Fault::DropTags && field == "tags" {
                    RpcReply::Drop
                } else {
                    RpcReply::Json(json!({"result":null}))
                });
            }
            None
        }
    };
    (mock_overridden(override_rpc(callback)).await, records)
}
fn vm() -> Id {
    Id::Vm(VmId::parse(VM).unwrap())
}
fn pool() -> Id {
    Id::Pool(PoolId::parse(POOL).unwrap())
}
fn tag() -> Edit {
    Edit {
        object: vm(),
        change: Change::Tags {
            previous: vec!["service".into(), "keep".into()],
            value: vec!["production".into(), "keep".into()],
        },
    }
}
fn folder() -> Edit {
    Edit {
        object: vm(),
        change: Change::Folder {
            previous: Some("/Old/Child".into()),
            value: Some("/New/Child".into()),
        },
    }
}
fn field(value: Option<&str>) -> Edit {
    Edit {
        object: vm(),
        change: Change::CustomField {
            definition: CustomFieldDefinition {
                name: "Owner".into(),
                kind: CustomFieldType::String,
            },
            previous: Some("Alice".into()),
            value: value.map(str::to_owned),
        },
    }
}
async fn connection(mock: &Mock, backend: &Backend, read_only: bool) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(mock.endpoint.clone());
    options.read_only = read_only;
    let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(deadline(3)).await.unwrap();
    c
}
fn command(c: &xenadmin_core::Connection, edits: Vec<Edit>) -> ResourceCommand {
    ResourceCommand::Organization {
        pool: c
            .latest()
            .observe_resource(ResourceTarget::Pool(PoolId::parse(POOL).unwrap()))
            .unwrap(),
        patch: OrganizationPatch::new(edits).unwrap(),
    }
}
async fn finish(operation: &ResourceOperation) -> Arc<ResourceOperationRecord> {
    tokio::time::timeout(deadline(5), async {
        loop {
            let record = operation.latest();
            if record.state.terminal() || record.state == OperationState::OutcomeUnknown {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn tags_folders_and_custom_values_share_review_and_preserve_unrelated_maps() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &b, false).await;
    let op = c
        .submit_resource(
            OperationId::new(),
            command(
                &c,
                vec![
                    tag(),
                    folder(),
                    field(Some("Bob")),
                    Edit {
                        object: pool(),
                        change: Change::EmptyFolders {
                            previous: vec!["/Old/Empty".into()],
                            value: vec!["/New/Empty".into()],
                        },
                    },
                ],
            ),
        )
        .unwrap();
    let result = finish(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(result.steps.len(), 4);
    let map = records.lock().unwrap().clone();
    assert_eq!(map["VM"]["other_config"]["unrelated"], "preserve");
    assert_eq!(
        map["VM"]["other_config"]["XenCenter.CustomFields.Owner"],
        "Bob"
    );
    assert_eq!(map["VM"]["tags"], json!(["production", "keep"]));
    assert_eq!(map["pool"]["other_config"]["pool-key"], "preserve");
    c.close().await;
}
#[tokio::test]
async fn stale_tag_definition_and_vm_kind_are_refused_before_any_write() {
    for fault in [Fault::StaleTags, Fault::StaleDefinition, Fault::WrongType] {
        let (mock, _) = fixture(fault).await;
        let dir = tempfile::tempdir().unwrap();
        let b = Backend::with_state_directory(dir.path().into()).unwrap();
        let c = connection(&mock, &b, false).await;
        let op = c
            .submit_resource(
                OperationId::new(),
                command(&c, vec![tag(), field(Some("Bob"))]),
            )
            .unwrap();
        assert_eq!(finish(&op).await.state, OperationState::Rejected);
        assert!(
            !mock
                .methods
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.contains(".set_"))
        );
        c.close().await;
    }
}
#[tokio::test]
async fn lost_tag_reply_keeps_pool_reserved_and_recheck_does_not_repeat_or_advance() {
    let (mock, records) = fixture(Fault::DropTags).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &b, false).await;
    let id = OperationId::new();
    let op = c
        .submit_resource(id, command(&c, vec![tag(), folder()]))
        .unwrap();
    assert_eq!(finish(&op).await.state, OperationState::OutcomeUnknown);
    let second = c
        .submit_resource(OperationId::new(), command(&c, vec![folder()]))
        .unwrap();
    assert_eq!(finish(&second).await.state, OperationState::Rejected);
    c.control_resource(id, OperationControl::Recheck).unwrap();
    tokio::time::timeout(deadline(4), async {
        loop {
            if c.resource_history()
                .iter()
                .any(|r| r.id == id && r.state == OperationState::PartiallyCompleted)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        mock.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "VM.set_tags")
            .count(),
        1
    );
    assert_eq!(
        records.lock().unwrap()["VM"]["other_config"]["folder"],
        "/Old/Child"
    );
    c.close().await;
}
#[tokio::test]
async fn later_refused_map_edit_reports_partial_completion() {
    let (mock, records) = fixture(Fault::FailMap).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &b, false).await;
    let op = c
        .submit_resource(OperationId::new(), command(&c, vec![tag(), folder()]))
        .unwrap();
    let result = finish(&op).await;
    assert_eq!(
        result.state,
        OperationState::PartiallyCompleted,
        "{}; methods: {:?}",
        result.detail,
        mock.methods.lock().unwrap()
    );
    assert_eq!(
        records.lock().unwrap()["VM"]["tags"],
        json!(["production", "keep"])
    );
    c.close().await;
}
#[tokio::test]
async fn definition_deletion_clears_reviewed_values_first_and_preserves_gui_config() {
    let (mock, records) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &b, false).await;
    let op = c
        .submit_resource(
            OperationId::new(),
            command(
                &c,
                vec![
                    field(None),
                    Edit {
                        object: pool(),
                        change: Change::Definitions {
                            previous: vec![CustomFieldDefinition {
                                name: "Owner".into(),
                                kind: CustomFieldType::String,
                            }],
                            value: vec![],
                        },
                    },
                ],
            ),
        )
        .unwrap();
    let result = finish(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert!(
        records.lock().unwrap()["VM"]["other_config"]
            .get("XenCenter.CustomFields.Owner")
            .is_none()
    );
    assert_eq!(
        records.lock().unwrap()["pool"]["gui_config"]["unrelated"],
        "preserve"
    );
    c.close().await;
}
#[tokio::test]
async fn organization_inventory_is_typed_and_readonly_submission_is_denied() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &b, true).await;
    let snapshot = c.latest();
    let inventory = snapshot.inventory().unwrap();
    let object = inventory
        .details
        .organization
        .objects
        .iter()
        .find(|o| o.id == vm())
        .unwrap();
    assert_eq!(object.folder.as_deref(), Some("/Old/Child"));
    assert_eq!(object.custom_fields["Owner"], "Alice");
    assert_eq!(
        inventory
            .details
            .organization
            .definitions
            .as_ref()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !serde_json::to_string(&inventory.details.organization)
            .unwrap()
            .contains("OpaqueRef:")
    );
    assert!(matches!(
        c.submit_resource(OperationId::new(), command(&c, vec![tag()])),
        Err(xenadmin_core::Error::ReadOnly)
    ));
    c.close().await;
}

#[tokio::test]
async fn deleting_a_definition_refuses_unreviewed_values_before_mutation() {
    let (mock, _) = fixture(Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let b = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&mock, &b, false).await;
    let op = c
        .submit_resource(
            OperationId::new(),
            command(
                &c,
                vec![Edit {
                    object: pool(),
                    change: Change::Definitions {
                        previous: vec![CustomFieldDefinition {
                            name: "Owner".into(),
                            kind: CustomFieldType::String,
                        }],
                        value: vec![],
                    },
                }],
            ),
        )
        .unwrap();
    assert_eq!(finish(&op).await.state, OperationState::Rejected);
    assert!(
        !mock
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.contains(".set_"))
    );
    c.close().await;
}
