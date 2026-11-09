use super::*;
use xenadmin_core::{
    BootReadiness, OperationControl, OperationId, OperationState, VmCommand, VmSettingEdit,
    VmSettingsPatch,
};
fn boot(record: &mut Value, template: bool) {
    record["is_a_template"] = json!(template);
    record["is_default_template"] = json!(false);
    record["power_state"] = json!("Halted");
    record["current_operations"] = json!({});
    record["HVM_boot_policy"] = json!("BIOS order");
    record["HVM_boot_params"] = json!({"firmware":"uefi","order":"dc"});
    record["platform"] = json!({"secureboot":"false","keep":"yes"});
    record["VBDs"] = json!([]);
}
async fn connection(
    server: &Mock,
    backend: &Backend,
    read_only: bool,
) -> xenadmin_core::Connection {
    let mut options = ConnectionOptions::new(server.endpoint.clone());
    options.read_only = read_only;
    let c = backend.connect(options, Credentials::new("root".into(), "fixture".into()));
    c.wait_ready(deadline(3)).await.unwrap();
    c
}
fn command(c: &xenadmin_core::Connection, patch: VmSettingsPatch) -> VmCommand {
    let snapshot = c.latest();
    let id = snapshot
        .inventory()
        .unwrap()
        .vms
        .values()
        .next()
        .unwrap()
        .id;
    VmCommand::Configure {
        vm: snapshot.observe_vm(id).unwrap(),
        patch,
    }
}
#[tokio::test]
async fn template_settings_apply_only_to_explicit_custom_template_target() {
    let (server, data) = settings_mock(false, false, None).await;
    boot(&mut data.lock().unwrap(), true);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend, false).await;
    let edit = VmSettingEdit::SecureBoot {
        previous: false,
        value: true,
    };
    let wrong = c
        .submit(
            OperationId::new(),
            command(&c, VmSettingsPatch::new(vec![edit.clone()]).unwrap()),
        )
        .unwrap();
    assert_eq!(finish(&wrong).await.state, OperationState::Rejected);
    let op = c
        .submit(
            OperationId::new(),
            command(&c, VmSettingsPatch::custom_template(vec![edit]).unwrap()),
        )
        .unwrap();
    let result = finish(&op).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_eq!(
        data.lock().unwrap()["platform"],
        json!({"secureboot":"true","keep":"yes"})
    );
    c.close().await;
}
#[tokio::test]
async fn a_template_converted_during_settings_stops_before_the_next_mutation() {
    let (server, data) = settings_mock(false, false, None).await;
    {
        let mut r = data.lock().unwrap();
        boot(&mut r, true);
        r["fixture_flip_kind"] = json!(true);
    }
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend, false).await;
    let name = data.lock().unwrap()["name_label"]
        .as_str()
        .unwrap()
        .to_owned();
    let op = c
        .submit(
            OperationId::new(),
            command(
                &c,
                VmSettingsPatch::custom_template(vec![
                    VmSettingEdit::Name {
                        previous: name,
                        value: "Changed".into(),
                    },
                    VmSettingEdit::SecureBoot {
                        previous: false,
                        value: true,
                    },
                ])
                .unwrap(),
            ),
        )
        .unwrap();
    let result = finish(&op).await;
    assert_eq!(
        result.state,
        OperationState::OutcomeUnknown,
        "{}",
        result.detail
    );
    assert_eq!(data.lock().unwrap()["platform"]["secureboot"], "false");
    assert!(
        !server
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "VM.set_platform")
    );
    c.close().await;
}
#[tokio::test]
async fn lost_secure_boot_reply_reconciles_policy_without_repeating_write() {
    let (server, data) = settings_mock(false, false, None).await;
    {
        let mut r = data.lock().unwrap();
        boot(&mut r, false);
        r["fixture_lose_platform"] = json!(true);
    }
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend, false).await;
    let op = c
        .submit(
            OperationId::new(),
            command(
                &c,
                VmSettingsPatch::new(vec![VmSettingEdit::SecureBoot {
                    previous: false,
                    value: true,
                }])
                .unwrap(),
            ),
        )
        .unwrap();
    let result = finish(&op).await;
    assert_eq!(result.state, OperationState::OutcomeUnknown);
    c.control_operation(result.id, OperationControl::Recheck)
        .unwrap();
    let result = tokio::time::timeout(deadline(3), async {
        loop {
            if let Some(r) = c
                .operation_history()
                .iter()
                .find(|r| r.id == result.id && r.state == OperationState::Succeeded)
            {
                break r.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(result.state, OperationState::Succeeded);
    assert_eq!(
        server
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == "VM.set_platform")
            .count(),
        1
    );
    c.close().await;
}
#[tokio::test]
async fn read_only_connection_can_query_boot_readiness_without_admitting_writes() {
    let (server, data) = settings_mock(false, false, None).await;
    boot(&mut data.lock().unwrap(), false);
    let dir = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(dir.path().into()).unwrap();
    let c = connection(&server, &backend, true).await;
    let snapshot = c.latest();
    let id = snapshot
        .inventory()
        .unwrap()
        .vms
        .values()
        .next()
        .unwrap()
        .id;
    let observed = snapshot.observe_vm(id).unwrap();
    let query = c.guest_boot_status(observed).unwrap();
    let result = tokio::time::timeout(deadline(3), async {
        loop {
            if let Some(result) = query.latest() {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.policy, Some(false));
    assert_eq!(result.guest, BootReadiness::SetupMode);
    assert!(
        c.submit(
            OperationId::new(),
            command(
                &c,
                VmSettingsPatch::new(vec![VmSettingEdit::SecureBoot {
                    previous: false,
                    value: true
                }])
                .unwrap()
            )
        )
        .is_err()
    );
    c.close().await;
    assert!(c.guest_boot_status(observed).is_err());
}

async fn finish(op: &xenadmin_core::Operation) -> xenadmin_core::OperationRecord {
    tokio::time::timeout(deadline(3), async {
        loop {
            let r = op.latest();
            if r.state.terminal() || r.state == OperationState::OutcomeUnknown {
                break (*r).clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
