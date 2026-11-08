use super::*;
use xenadmin_core::{CloudInitInputs, CloudInitMedia, Error, OperationId, SrId, VmId};
fn media(vm: VmId) -> CloudInitMedia {
    CloudInitMedia::prepare(
        vm,
        CloudInitInputs::new("fixture".into(), "#cloud-config\n".into(), None).unwrap(),
    )
    .unwrap()
}
#[tokio::test]
async fn cloud_init_facade_binds_media_to_vm_pool_generation_and_write_access() {
    let server = mock(false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let options = ConnectionOptions::new(server.endpoint.clone());
    let a = backend.connect(
        options.clone(),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let i = a.wait_ready(Duration::from_secs(3)).await.unwrap();
    let vm = i.guests().next().unwrap().id;
    let observed = a.latest().observe_vm(vm).unwrap();
    let sr = SrId::from(uuid::Uuid::from_u128(1));
    assert!(matches!(
        a.prepare_cloud_init(
            OperationId::new(),
            observed,
            sr,
            "1".into(),
            media(VmId::from(uuid::Uuid::from_u128(99)))
        ),
        Err(Error::Stale)
    ));
    a.close().await;
    let b = backend.connect(
        options.clone(),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    b.wait_ready(Duration::from_secs(3)).await.unwrap();
    assert!(matches!(
        b.prepare_cloud_init(OperationId::new(), observed, sr, "1".into(), media(vm)),
        Err(Error::Stale)
    ));
    b.close().await;
    let mut options = options;
    options.read_only = true;
    let c = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture".into()),
    );
    c.wait_ready(Duration::from_secs(3)).await.unwrap();
    let fresh = c.latest().observe_vm(vm).unwrap();
    assert!(matches!(
        c.prepare_cloud_init(OperationId::new(), fresh, sr, "1".into(), media(vm)),
        Err(Error::ReadOnly)
    ));
    assert_eq!(server.mutations.load(Ordering::SeqCst), 0);
    assert!(
        !server
            .methods
            .lock()
            .unwrap()
            .iter()
            .any(|m| m == "VDI.create" || m == "VBD.create")
    );
    c.close().await;
}
