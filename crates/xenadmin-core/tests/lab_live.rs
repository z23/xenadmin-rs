//! Live acceptance scenarios against the explicitly designated disposable lab.
//!
//! Every test is ignored by default. Run them deliberately:
//!
//! ```sh
//! XENADMIN_LAB_MANIFEST=/var/lib/libvirt/images/xenadmin-rs-lab/lab.json \
//! XENADMIN_LAB_PASSWORD_FILE=/path/to/lab-root-password \
//! cargo test -p xenadmin-core --test lab_live -- --ignored --test-threads=1
//! ```
//!
//! Targets come only from the harness manifest (`lab/manage.py plan`), never from
//! saved desktop profiles. Sanitized JSON evidence is written under the manifest
//! directory (`evidence/`), outside the repository.
#[path = "fixtures/deadlines.rs"]
mod deadlines;
use deadlines::deadline;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use xenadmin_core::{
    Backend, CertificateFingerprint, CertificateInspectionTask, Connection, ConnectionOptions,
    Credentials, Endpoint, HostId, OperationId, OperationState, PoolJoinSelection, ResourceTarget,
};

struct LabHost {
    name: String,
    ip: String,
}

struct Lab {
    id: String,
    hosts: Vec<LabHost>,
    password: String,
    evidence: PathBuf,
}

impl Lab {
    fn load() -> Lab {
        let manifest = PathBuf::from(
            std::env::var("XENADMIN_LAB_MANIFEST")
                .expect("XENADMIN_LAB_MANIFEST must name the harness lab.json"),
        );
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest).expect("readable manifest"))
                .expect("manifest JSON");
        assert_eq!(document["schema"], 1, "unsupported manifest schema");
        let id = document["id"].as_str().expect("manifest id").to_string();
        let name = document["name"].as_str().expect("manifest name");
        assert_eq!(
            name,
            format!("xenadmin-lab-{}", &id[..8]),
            "manifest identity"
        );
        let hosts = document["hosts"]
            .as_array()
            .expect("hosts")
            .iter()
            .map(|host| LabHost {
                name: host["name"].as_str().expect("host name").to_string(),
                ip: host["ip"].as_str().expect("host ip").to_string(),
            })
            .collect::<Vec<_>>();
        assert!(!hosts.is_empty() && hosts.len() <= 3);
        let password_file = std::env::var("XENADMIN_LAB_PASSWORD_FILE")
            .expect("XENADMIN_LAB_PASSWORD_FILE must name the lab-only root password file");
        let password = std::fs::read_to_string(password_file)
            .expect("readable password file")
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(password.len() >= 8, "lab password missing");
        let evidence = std::env::var("XENADMIN_LAB_EVIDENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest.parent().unwrap().join("evidence"));
        std::fs::create_dir_all(&evidence).expect("evidence directory");
        Lab {
            id,
            hosts,
            password,
            evidence,
        }
    }
    fn endpoint(&self, index: usize) -> Endpoint {
        Endpoint::parse(&format!("https://{}", self.hosts[index].ip)).unwrap()
    }
    fn credentials(&self) -> Credentials {
        Credentials::new("root".into(), self.password.clone())
    }
    fn record(&self, scenario: &str, body: Value) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let document = json!({
            "scenario": scenario,
            "lab": self.id,
            "application": env!("CARGO_PKG_VERSION"),
            "recorded_unix": stamp,
            "result": body,
        });
        let path = self.evidence.join(format!("{stamp}-{scenario}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&document).unwrap() + "\n",
        )
        .unwrap();
        eprintln!("evidence: {}", path.display());
    }
}

/// Credential-free TLS inspection, as the desktop's certificate review performs it.
async fn inspect(endpoint: &Endpoint) -> xenadmin_core::CertificateObservation {
    let task = CertificateInspectionTask::start(
        ConnectionOptions::new(endpoint.clone()),
        endpoint.clone(),
        None,
    );
    tokio::time::timeout(deadline(30), async {
        loop {
            if let Some(outcome) = task.take_result() {
                return outcome;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("certificate inspection finished")
    .expect("certificate observed")
}

async fn pinned_options(endpoint: &Endpoint) -> (ConnectionOptions, CertificateFingerprint) {
    let observation = inspect(endpoint).await;
    let fingerprint = observation.details.fingerprint.clone();
    let mut options = ConnectionOptions::new(endpoint.clone());
    options.certificate_fingerprint = Some(fingerprint.clone());
    (options, fingerprint)
}

async fn connect(lab: &Lab, backend: &Backend, index: usize, read_only: bool) -> Connection {
    let (mut options, _) = pinned_options(&lab.endpoint(index)).await;
    options.read_only = read_only;
    let connection = backend.connect(options, lab.credentials());
    connection
        .wait_ready(deadline(90))
        .await
        .unwrap_or_else(|error| panic!("{} did not become ready: {error}", lab.hosts[index].name));
    connection
}

async fn settle<F: Fn() -> bool>(seconds: u64, ready: F) -> bool {
    tokio::time::timeout(deadline(seconds), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .is_ok()
}

fn inventory_summary(connection: &Connection) -> Value {
    let snapshot = connection.latest();
    let inventory = snapshot.inventory().expect("inventory");
    json!({
        "pool_id": inventory.pool_id.to_string(),
        "pool_name": inventory.pool_name,
        "coordinator": inventory.coordinator.map(|h| h.to_string()),
        "hosts": inventory.hosts.values().map(|h| json!({
            "id": h.id.to_string(), "name": h.name, "address": h.address,
            "enabled": h.enabled, "version": h.version,
        })).collect::<Vec<_>>(),
        "vm_count": inventory.vms.len(),
        "guest_count": inventory.guests().count(),
        "storage_count": inventory.storage.len(),
        "network_count": inventory.networks.len(),
        "revision": inventory.revision,
        "state": format!("{:?}", snapshot.state()),
    })
}

/// P01-01/P01-11/P01-13/P19-01: direct read-only login through every lab member with a
/// reviewed certificate pin; wrong and absent pins are refused without credentials.
#[tokio::test]
#[ignore]
async fn lab_direct_recovery_read_only() {
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let mut hosts = Vec::new();
    for (index, host) in lab.hosts.iter().enumerate() {
        let endpoint = lab.endpoint(index);
        let observation = inspect(&endpoint).await;
        let pin = observation.details.fingerprint.clone();
        let connection = connect(&lab, &backend, index, true).await;
        let summary = inventory_summary(&connection);
        connection.close().await;

        // Wrong pin: the handshake must be refused before any login.
        let mut wrong = ConnectionOptions::new(endpoint.clone());
        wrong.certificate_fingerprint = Some(
            "0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );
        wrong.read_only = true;
        let refused = backend.connect(wrong, lab.credentials());
        let wrong_result = refused
            .wait_ready(deadline(30))
            .await
            .err()
            .map(|e| e.to_string());
        let wrong_state = format!("{:?}", refused.latest().state());
        refused.close().await;

        // No pin and no private CA: the self-signed lab certificate is untrusted.
        let mut untrusted = ConnectionOptions::new(endpoint.clone());
        untrusted.read_only = true;
        let refused = backend.connect(untrusted, lab.credentials());
        let untrusted_result = refused
            .wait_ready(deadline(30))
            .await
            .err()
            .map(|e| e.to_string());
        let untrusted_state = format!("{:?}", refused.latest().state());
        refused.close().await;

        assert!(
            wrong_result.is_some(),
            "{}: wrong pin was not refused",
            host.name
        );
        assert!(
            untrusted_result.is_some(),
            "{}: untrusted certificate was not refused",
            host.name
        );
        hosts.push(json!({
            "host": host.name,
            "address": host.ip,
            "certificate": {
                "fingerprint": pin.to_string(),
                "subject": observation.details.subject,
                "issuer": observation.details.issuer,
                "validation": observation.validation.label(),
            },
            "pinned_read_only": summary,
            "wrong_pin": {"error": wrong_result, "state": wrong_state},
            "untrusted": {"error": untrusted_result, "state": untrusted_state},
        }));
    }
    lab.record("direct-recovery-read-only", json!({"hosts": hosts}));
}

/// P12-03/P13-02: put a member into maintenance, then eject it with the reviewed ejection.
/// The ejected host reboots and returns as a standalone pool.
#[tokio::test]
#[ignore]
async fn lab_eject_member_by_reviewed_ejection() {
    use xenadmin_core::{HostOperation, PoolOperation, ResourceCommand};
    let lab = Lab::load();
    let member_index = std::env::var("XENADMIN_LAB_EJECT_HOST")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| (1..lab.hosts.len()).contains(i))
        .unwrap_or(lab.hosts.len() - 1);
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let coordinator = connect(&lab, &backend, 0, false).await;
    let before = inventory_summary(&coordinator);
    let inventory = coordinator.latest().inventory().unwrap().clone();
    let Some(host) = inventory
        .hosts
        .values()
        .find(|h| h.address == lab.hosts[member_index].ip)
        .cloned()
    else {
        lab.record(
            "eject-member-reviewed",
            json!({"skipped": "member is not in the coordinator's pool", "pool": before}),
        );
        coordinator.close().await;
        return;
    };
    assert_ne!(
        Some(host.id),
        inventory.coordinator,
        "never eject the coordinator"
    );
    let maintenance = coordinator
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Host {
                target: coordinator
                    .latest()
                    .observe_resource(ResourceTarget::Host(host.id))
                    .unwrap(),
                operation: HostOperation::EnterMaintenance {
                    expected_enabled: host.enabled,
                    resident_vms: Vec::new(),
                },
            },
        )
        .expect("maintenance accepted");
    let maintenance_outcome = wait_outcome(&maintenance, 300).await;
    let disabled = settle(120, || {
        coordinator
            .latest()
            .inventory()
            .and_then(|i| i.hosts.get(&host.id).map(|h| !h.enabled))
            .unwrap_or(false)
    })
    .await;
    let eject = coordinator
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Pool {
                target: coordinator
                    .latest()
                    .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                    .unwrap(),
                operation: PoolOperation::EjectHost {
                    host: host.id,
                    expected_name: host.name.clone(),
                },
            },
        )
        .expect("ejection accepted");
    let eject_outcome = wait_outcome(&eject, 600).await;
    let removed = settle(300, || {
        coordinator
            .latest()
            .inventory()
            .is_some_and(|i| !i.hosts.contains_key(&host.id))
    })
    .await;
    let after = inventory_summary(&coordinator);
    coordinator.close().await;
    // The ejected host reboots into a fresh standalone pool; wait for its own XAPI.
    let standalone = tokio::time::timeout(deadline(600), async {
        loop {
            if let Ok((options, _)) = tokio::time::timeout(Duration::from_secs(20), pinned_options(&lab.endpoint(member_index))).await
                && let Ok(inventory) = backend.connect({ let mut o = options; o.read_only = true; o }, lab.credentials()).wait_ready(Duration::from_secs(60)).await
                && inventory.hosts.len() == 1
                && inventory.pool_id != inventory_before_pool(&before)
            {
                return json!({"pool_id": inventory.pool_id.to_string(), "hosts": inventory.hosts.len()});
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    })
    .await
    .ok();
    lab.record("eject-member-reviewed", json!({
        "member": {"name": host.name, "address": host.address, "id": host.id.to_string()},
        "pool_before": before,
        "maintenance": {"state": format!("{:?}", maintenance_outcome.state), "detail": maintenance_outcome.detail, "host_disabled_observed": disabled},
        "ejection": {"state": format!("{:?}", eject_outcome.state), "detail": eject_outcome.detail,
                     "steps": eject_outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()},
        "member_removed_from_inventory": removed,
        "pool_after": after,
        "ejected_host_standalone": standalone,
    }));
    assert_eq!(
        maintenance_outcome.state,
        OperationState::Succeeded,
        "{}",
        maintenance_outcome.detail
    );
    assert_eq!(
        eject_outcome.state,
        OperationState::Succeeded,
        "{}",
        eject_outcome.detail
    );
    assert!(removed, "ejected member still listed");
    assert!(
        standalone.is_some(),
        "ejected host did not return as a standalone pool"
    );
}

fn inventory_before_pool(summary: &Value) -> xenadmin_core::PoolId {
    xenadmin_core::PoolId::parse(summary["pool_id"].as_str().unwrap()).unwrap()
}

async fn wait_outcome(
    operation: &xenadmin_core::ResourceOperation,
    seconds: u64,
) -> std::sync::Arc<xenadmin_core::ResourceOperationRecord> {
    tokio::time::timeout(deadline(seconds), async {
        loop {
            let row = operation.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("operation reached an outcome")
}

/// P13-02: form the primary pool by the reviewed join of the second host into the first.
/// The third host stays standalone for cross-pool scenarios.
#[tokio::test]
#[ignore]
async fn lab_form_pool_by_reviewed_join() {
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 2, "join needs two hosts");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let destination = connect(&lab, &backend, 0, false).await;
    let before = inventory_summary(&destination);
    let source_index = std::env::var("XENADMIN_LAB_JOIN_SOURCE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| (1..lab.hosts.len()).contains(i))
        .unwrap_or(1);
    let source = connect(&lab, &backend, source_index, false).await;
    let source_inventory = source.latest().inventory().unwrap().clone();
    let destination_inventory = destination.latest().inventory().unwrap().clone();
    if source_inventory.hosts.len() != 1
        || source_inventory.pool_id == destination_inventory.pool_id
    {
        lab.record(
            "form-pool-reviewed-join",
            json!({"skipped": "source is not a standalone host", "destination": before}),
        );
        source.close().await;
        destination.close().await;
        return;
    }
    let source_host: HostId = *source_inventory.hosts.keys().next().unwrap();
    let destination_coordinator = destination_inventory.coordinator.expect("coordinator");
    let selection = PoolJoinSelection {
        source: source
            .latest()
            .observe_resource(ResourceTarget::Pool(source_inventory.pool_id))
            .unwrap(),
        source_host,
        destination: destination
            .latest()
            .observe_resource(ResourceTarget::Pool(destination_inventory.pool_id))
            .unwrap(),
        destination_coordinator,
        destination_address: lab.hosts[0].ip.clone(),
    };
    let operation = source
        .join_pool(
            OperationId::new(),
            selection,
            &destination,
            lab.credentials(),
        )
        .expect("join accepted for review");
    // The joining host restarts its toolstack, which can drop the source connection
    // before the reply arrives. The durable record then surfaces through the
    // destination's history after recovery, so watch both.
    let join_id = operation.latest().id;
    let outcome = tokio::time::timeout(deadline(600), async {
        loop {
            let row = operation.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            if let Some(row) = destination
                .resource_history()
                .iter()
                .find(|r| {
                    r.id == join_id
                        && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
                })
                .cloned()
            {
                return std::sync::Arc::new(row);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("join reached an outcome");
    let initial = outcome.clone();
    let mut rechecked = None;
    let outcome = if outcome.state == OperationState::OutcomeUnknown {
        // The operator path: once the member is visible again, recheck through the
        // destination connection. Rechecks read evidence and never repeat the join.
        settle(180, || {
            destination
                .latest()
                .inventory()
                .is_some_and(|i| i.hosts.contains_key(&source_host))
        })
        .await;
        destination
            .control_resource(outcome.id, xenadmin_core::OperationControl::Recheck)
            .expect("recheck accepted");
        let row = tokio::time::timeout(deadline(120), async {
            loop {
                if let Some(row) = destination
                    .resource_history()
                    .iter()
                    .find(|r| {
                        r.id == outcome.id
                            && r.revision() > outcome.revision()
                            && (r.state.terminal() || r.state == OperationState::OutcomeUnknown)
                    })
                    .cloned()
                {
                    return row;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .expect("recheck reached an outcome");
        rechecked = Some(json!({"state": format!("{:?}", row.state), "detail": row.detail}));
        std::sync::Arc::new(row)
    } else {
        outcome
    };
    let joined = outcome.state == OperationState::Succeeded
        && settle(300, || {
            destination
                .latest()
                .inventory()
                .is_some_and(|i| i.hosts.len() >= 2 && i.hosts.contains_key(&source_host))
        })
        .await;
    let after = inventory_summary(&destination);
    let source_state = format!("{:?}", source.latest().state());
    source.close().await;
    destination.close().await;
    lab.record("form-pool-reviewed-join", json!({
        "destination_before": before,
        "initial_outcome": {"state": format!("{:?}", initial.state), "detail": initial.detail},
        "recheck": rechecked,
        "operation": {"state": format!("{:?}", outcome.state), "detail": outcome.detail,
                      "steps": outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()},
        "destination_after": after,
        "source_connection_state_after_join": source_state,
        "member_visible_on_destination": joined,
    }));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(
        joined,
        "joined host did not appear in the destination inventory"
    );
}

/// Alpine's `virt` image is an isohybrid ISO, so it boots as a plain disk. Imported as a raw
/// disk it gives the lab a bootable guest without an ISO library or guest tools.
fn guest_image() -> PathBuf {
    PathBuf::from(
        std::env::var("XENADMIN_LAB_GUEST_IMAGE")
            .expect("XENADMIN_LAB_GUEST_IMAGE must name the isohybrid guest image"),
    )
}

async fn wait_vm_outcome(
    operation: &xenadmin_core::Operation,
    seconds: u64,
) -> std::sync::Arc<xenadmin_core::OperationRecord> {
    tokio::time::timeout(deadline(seconds), async {
        loop {
            let row = operation.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("VM operation reached an outcome")
}

fn local_sr_of(inventory: &xenadmin_core::Inventory, host: HostId) -> Option<xenadmin_core::SrId> {
    inventory.details.pbds.values().find_map(|pbd| {
        let sr = pbd.sr?;
        (pbd.host == Some(host)
            && inventory.details.storage.get(&sr).and_then(|s| s.shared) == Some(false)
            && inventory
                .storage
                .iter()
                .any(|s| xenadmin_core::SrId::from(s.uuid) == sr && s.kind == "ext"))
        .then_some(sr)
    })
}

/// P11-09, P08-02, P07-03, P07-02, P03-01, P04-01: import a bootable raw image, create a
/// guest from the generic template with no disk, attach the image as its first disk, add
/// a blank data disk, start it and receive console frames.
#[tokio::test]
#[ignore]
async fn lab_guest_provisioning() {
    use xenadmin_core::{
        DeviceCommand, DeviceOperation, DiskImageFormat, GuestFirmware, PowerAction,
        TransferAction, TransferFile, TransferRequest, VmCommand, VmLifecycleOperation,
    };
    let lab = Lab::load();
    let image = guest_image();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory.coordinator.expect("coordinator");
    let sr = local_sr_of(&inventory, coordinator).expect("coordinator local EXT repository");
    let template = inventory
        .vms
        .values()
        .find(|v| v.is_template && v.name == "Other install media")
        .expect("generic template")
        .clone();
    let network = inventory
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(coordinator) && p.management_purpose.is_some())
        .and_then(|p| p.network)
        .or_else(|| {
            inventory
                .networks
                .iter()
                .find(|n| n.bridge == "xenbr0")
                .map(|n| xenadmin_core::NetworkId::from(n.uuid))
        })
        .expect("management network");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let disk_name = format!("lab-guest-image-{stamp}");

    // Import the bootable image as a raw disk (reviewed transfer, private atomic file).
    let file = TransferFile::disk_source(&image, DiskImageFormat::Raw).expect("inspect image");
    let import = c
        .transfer(
            OperationId::new(),
            c.latest()
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .unwrap(),
            TransferRequest {
                action: TransferAction::ImportDisk {
                    sr,
                    name: disk_name.clone(),
                    format: DiskImageFormat::Raw,
                },
                file,
            },
        )
        .expect("import accepted");
    let import_outcome = wait_outcome(&import, 900).await;
    assert_eq!(
        import_outcome.state,
        OperationState::Succeeded,
        "{}",
        import_outcome.detail
    );
    let imported = tokio::time::timeout(deadline(120), async {
        loop {
            if let Some(vdi) = c.latest().inventory().and_then(|i| {
                i.details
                    .vdis
                    .values()
                    .find(|v| v.name == disk_name && v.sr == Some(sr))
                    .map(|v| v.id)
            }) {
                return vdi;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("imported disk visible in inventory");

    // Create the guest from the generic template without a disk.
    let vm_name = format!("lab-guest-{stamp}");
    let create = c
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(template.id).unwrap(),
                operation: VmLifecycleOperation::Create {
                    name: vm_name.clone(),
                    description: "Disposable lab guest booting an isohybrid image".into(),
                    sr,
                    network,
                    iso: None,
                    vcpus: 1,
                    memory: 512 * 1024 * 1024,
                    disk_size: None,
                    firmware: GuestFirmware::Bios,
                    bios_host: None,
                },
            },
        )
        .expect("creation accepted");
    let create_outcome = wait_vm_outcome(&create, 300).await;
    assert_eq!(
        create_outcome.state,
        OperationState::Succeeded,
        "{}",
        create_outcome.detail
    );
    let vm = create_outcome.created_vm.expect("created VM identity");
    assert!(
        settle(60, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.vms.contains_key(&vm)))
        .await
    );

    // Attach the image as the first disk, then add a blank data disk.
    let attach = c
        .submit(
            OperationId::new(),
            VmCommand::Device(DeviceCommand {
                vm: c.latest().observe_vm(vm).unwrap(),
                operation: DeviceOperation::AttachDisk {
                    vdi: imported,
                    device: "0".into(),
                    read_only: false,
                },
            }),
        )
        .expect("attach accepted");
    let attach_outcome = wait_vm_outcome(&attach, 300).await;
    assert_eq!(
        attach_outcome.state,
        OperationState::Succeeded,
        "{}",
        attach_outcome.detail
    );
    let data_disk = c
        .submit(
            OperationId::new(),
            VmCommand::Device(DeviceCommand {
                vm: c.latest().observe_vm(vm).unwrap(),
                operation: DeviceOperation::CreateDisk {
                    sr,
                    name: format!("lab-guest-data-{stamp}"),
                    size: 1024 * 1024 * 1024,
                    device: "1".into(),
                },
            }),
        )
        .expect("disk creation accepted");
    let data_outcome = wait_vm_outcome(&data_disk, 300).await;
    assert_eq!(
        data_outcome.state,
        OperationState::Succeeded,
        "{}",
        data_outcome.detail
    );
    let attached = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vm_disks
                .get(&vm)
                .is_some_and(|vbds| vbds.len() == 2)
        })
    })
    .await;

    // Start it and receive console frames through the authenticated tunnel.
    let start = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(vm).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let start_outcome = wait_vm_outcome(&start, 300).await;
    assert_eq!(
        start_outcome.state,
        OperationState::Succeeded,
        "{}",
        start_outcome.detail
    );
    assert!(
        settle(120, || {
            c.latest()
                .inventory()
                .and_then(|i| {
                    i.vms
                        .get(&vm)
                        .map(|v| v.power_state == xenadmin_core::PowerState::Running)
                })
                .unwrap_or(false)
        })
        .await
    );
    let console = c
        .open_console(c.latest().observe_vm(vm).unwrap())
        .expect("console opened");
    let frame = tokio::time::timeout(deadline(120), async {
        loop {
            let snapshot = console.latest();
            if let Some(frame) = &snapshot.frame
                && frame.width > 0
                && frame.rgba.iter().any(|b| *b != 0)
            {
                return (
                    frame.width,
                    frame.height,
                    frame.rgba.len(),
                    format!("{:?}", snapshot.state),
                );
            }
            if let xenadmin_core::ConsoleState::Failed(error) = &snapshot.state {
                panic!("console failed: {error}");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("console frame received");
    console.close();
    c.close().await;
    lab.record("guest-provisioning", json!({
        "repository": sr.to_string(),
        "template": template.name,
        "import": {"state": format!("{:?}", import_outcome.state), "detail": import_outcome.detail, "disk": imported.to_string()},
        "create": {"state": format!("{:?}", create_outcome.state), "detail": create_outcome.detail, "vm": vm.to_string()},
        "attach_image": {"state": format!("{:?}", attach_outcome.state), "detail": attach_outcome.detail},
        "create_data_disk": {"state": format!("{:?}", data_outcome.state), "detail": data_outcome.detail},
        "two_disks_attached": attached,
        "start": {"state": format!("{:?}", start_outcome.state), "detail": start_outcome.detail},
        "console": {"width": frame.0, "height": frame.1, "bytes": frame.2, "state": frame.3},
    }));
    assert!(attached, "guest does not show two disks");
}

/// Types one line into a guest console as RFB key events the way a physical US keyboard
/// would: shifted characters are sent with Shift held, because the console encodes
/// physical keys and the host applies no implicit shift. Ends with Return.
async fn type_line(console: &xenadmin_core::Console, text: &str) {
    use xenadmin_core::ConsoleInput;
    const SHIFT: u32 = 0xffe1;
    fn shifted(ch: char) -> bool {
        ch.is_ascii_uppercase() || "~!@#$%^&*()_+{}|:\"<>?".contains(ch)
    }
    for ch in text.chars() {
        assert!(
            ch.is_ascii() && !ch.is_ascii_control(),
            "ASCII only: {ch:?}"
        );
        let keysym = ch as u32;
        let shift = shifted(ch);
        if shift {
            console
                .input(ConsoleInput::Key {
                    keysym: SHIFT,
                    down: true,
                })
                .unwrap();
        }
        console
            .input(ConsoleInput::Key { keysym, down: true })
            .unwrap();
        console
            .input(ConsoleInput::Key {
                keysym,
                down: false,
            })
            .unwrap();
        if shift {
            console
                .input(ConsoleInput::Key {
                    keysym: SHIFT,
                    down: false,
                })
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    console
        .input(ConsoleInput::Key {
            keysym: 0xff0d,
            down: true,
        })
        .unwrap();
    console
        .input(ConsoleInput::Key {
            keysym: 0xff0d,
            down: false,
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

fn frame_digest(frame: &xenadmin_core::ConsoleFrame) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(frame.width.to_le_bytes());
    hasher.update(frame.height.to_le_bytes());
    hasher.update(&frame.rgba);
    format!("{:x}", hasher.finalize())
}

/// A tiny listener on the lab bridge address that records guest reports. The guest calls
/// `wget http://GATEWAY:PORT/report/VALUE`; nothing else is served.
struct Reports {
    received: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Reports {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn report_listener(address: &str) -> Reports {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("bind report listener");
    let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = received.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let sink = sink.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).await.is_ok()
                    && let Some(path) = line.split_whitespace().nth(1)
                    && let Some(value) = path.strip_prefix("/report/")
                {
                    sink.lock().unwrap().push(value.to_string());
                }
                let mut stream = reader.into_inner();
                let _ = stream
                    .write_all(
                        b"HTTP/1.0 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nok\n",
                    )
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Reports { received, task }
}

/// Waits until the console shows at most two alternating frames (a blinking text cursor)
/// for a few seconds, then returns the latest frame's digest.
async fn stable_frame(console: &xenadmin_core::Console, seconds: u64) -> String {
    tokio::time::timeout(deadline(seconds), async {
        let mut recent: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        loop {
            if let Some(frame) = &console.latest().frame {
                recent.push_back(frame_digest(frame));
                // Boot output pauses for a few seconds between service groups; a login
                // prompt or shell stays put far longer, so demand twelve quiet seconds.
                if recent.len() > 24 {
                    recent.pop_front();
                }
                if recent.len() == 24 {
                    let distinct: std::collections::BTreeSet<&String> = recent.iter().collect();
                    if distinct.len() <= 2 {
                        return recent.back().unwrap().clone();
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("console frame settled")
}

/// Saves the current console frame as a binary PPM next to the evidence files.
fn save_frame(lab: &Lab, console: &xenadmin_core::Console, label: &str) -> Option<String> {
    let frame = console.latest().frame.clone()?;
    let mut data = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    for pixel in frame.rgba.chunks_exact(4) {
        data.extend_from_slice(&pixel[..3]);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let path = lab.evidence.join(format!("{stamp}-console-{label}.ppm"));
    std::fs::write(&path, data).ok()?;
    Some(path.display().to_string())
}

async fn wait_report(reports: &Reports, value: &str, seconds: u64) -> bool {
    settle(seconds, || {
        reports.received.lock().unwrap().iter().any(|v| v == value)
    })
    .await
}

/// Logs into the live Alpine guest, gives it a static lab address and reports the marker
/// held in the data disk's first sector to the workstation listener. The live image has no
/// filesystem tools, so the marker is a raw sector write; the verification pass only reads,
/// and an empty sector reports MISSING so a blank disk can never pass as a restored marker.
async fn guest_report_marker(
    console: &xenadmin_core::Console,
    gateway: &str,
    port: u16,
    guest_ip: &str,
    write: Option<char>,
) {
    stable_frame(console, 240).await;
    // A bare Return re-prompts a login prompt and is harmless in a shell.
    type_line(console, "").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    stable_frame(console, 60).await;
    type_line(console, "root").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    type_line(console, "ip link set eth0 up").await;
    type_line(console, &format!("ip addr add {guest_ip}/24 dev eth0")).await;
    type_line(console, "d=/dev/xvdb; [ -b $d ] || d=/dev/sdb").await;
    if let Some(value) = write {
        type_line(
            console,
            &format!("printf {value} | dd of=$d bs=512 count=1 conv=notrunc; sync"),
        )
        .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    type_line(console, "v=$(head -c 1 $d | tr -d '\\0')").await;
    type_line(
        console,
        &format!("wget -q -O- http://{gateway}:{port}/report/${{v:-MISSING}}"),
    )
    .await;
}

/// P04-02, P05-02, P05-03, P05-04, P19-04, P19-06: console keyboard input, a disk snapshot,
/// a marker change, reviewed restore with shutdown and restart, and the marker reading its
/// snapshot value again, each confirmed by the guest reporting over the isolated network.
#[tokio::test]
#[ignore]
async fn lab_guest_console_and_snapshot_restore() {
    use xenadmin_core::{PowerState, SnapshotName, SnapshotWorkflow, VmCommand};
    let lab = Lab::load();
    let gateway = std::env::var("XENADMIN_LAB_GATEWAY").unwrap_or_else(|_| "192.168.254.1".into());
    let port: u16 = 8000;
    let guest_ip =
        std::env::var("XENADMIN_LAB_GUEST_IP").unwrap_or_else(|_| "192.168.254.100".into());
    let reports = report_listener(&format!("{gateway}:{port}")).await;
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && v.power_state == PowerState::Running)
        .expect("running lab guest from lab_guest_provisioning")
        .clone();

    let console = c
        .open_console(c.latest().observe_vm(guest.id).unwrap())
        .expect("console");
    stable_frame(&console, 180).await;
    let frame_before_login = save_frame(&lab, &console, "before-login");
    guest_report_marker(&console, &gateway, port, &guest_ip, Some('A')).await;
    let reported_a = wait_report(&reports, "A", 90).await;
    let frame_after_login = save_frame(&lab, &console, "after-marker-a");

    // Disk snapshot while running, then change the marker.
    let snapshot_name = format!(
        "lab-marker-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let snapshot = c
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                name: SnapshotName::new(&snapshot_name).unwrap(),
            },
        )
        .expect("snapshot accepted");
    let snapshot_outcome = wait_vm_outcome(&snapshot, 300).await;
    assert_eq!(
        snapshot_outcome.state,
        OperationState::Succeeded,
        "{}",
        snapshot_outcome.detail
    );
    let snapshot_id = tokio::time::timeout(deadline(60), async {
        loop {
            if let Some(id) = c.latest().inventory().and_then(|i| {
                i.snapshots
                    .for_vm(guest.id)
                    .find(|s| s.name == snapshot_name)
                    .map(|s| s.id)
            }) {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .expect("snapshot visible");
    type_line(
        &console,
        "printf B | dd of=$d bs=512 count=1 conv=notrunc; sync",
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    type_line(&console, "v=$(head -c 1 $d | tr -d '\\0')").await;
    type_line(
        &console,
        &format!("wget -q -O- http://{gateway}:{port}/report/${{v:-MISSING}}"),
    )
    .await;
    let reported_b = wait_report(&reports, "B", 60).await;
    console.close();

    // Reviewed restore: clean shutdown, revert, start again.
    let restore = c
        .submit(
            OperationId::new(),
            VmCommand::SnapshotWorkflow {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: SnapshotWorkflow::Restore {
                    snapshot: snapshot_id,
                    expected_state: PowerState::Running,
                    checkpoint: false,
                    shutdown: true,
                    start_after: true,
                },
            },
        )
        .expect("restore accepted");
    let restore_outcome = wait_vm_outcome(&restore, 600).await;
    assert_eq!(
        restore_outcome.state,
        OperationState::Succeeded,
        "{}",
        restore_outcome.detail
    );
    assert!(
        settle(120, || c
            .latest()
            .inventory()
            .and_then(|i| i
                .vms
                .get(&guest.id)
                .map(|v| v.power_state == PowerState::Running))
            .unwrap_or(false))
        .await
    );

    // The live guest boots fresh; the data disk must read the snapshot's marker again.
    reports.received.lock().unwrap().clear();
    let console = c
        .open_console(c.latest().observe_vm(guest.id).unwrap())
        .expect("console after restore");
    guest_report_marker(&console, &gateway, port, &guest_ip, None).await;
    let restored = wait_report(&reports, "A", 90).await;
    let late_b = reports.received.lock().unwrap().iter().any(|v| v == "B");
    let frame_after_restore = save_frame(&lab, &console, "after-restore");
    console.close();
    c.close().await;
    lab.record("guest-console-and-snapshot-restore", json!({
        "guest": {"name": guest.name, "id": guest.id.to_string()},
        "console_frames": {"before_login": frame_before_login, "after_marker_a": frame_after_login, "after_restore": frame_after_restore},
        "marker_reported_before_snapshot": reported_a,
        "snapshot": {"state": format!("{:?}", snapshot_outcome.state), "detail": snapshot_outcome.detail, "id": snapshot_id.to_string()},
        "marker_reported_after_change": reported_b,
        "restore": {"state": format!("{:?}", restore_outcome.state), "detail": restore_outcome.detail,
                    "steps": restore_outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()},
        "marker_reported_after_restore": restored,
        "changed_marker_seen_after_restore": late_b,
        "reports": reports.received.lock().unwrap().clone(),
    }));
    assert!(
        reported_a,
        "guest never reported marker A (console input or guest network)"
    );
    assert!(reported_b, "guest never reported the changed marker B");
    assert!(restored && !late_b, "restored disk did not read marker A");
}

/// Diagnostic: type the lines in `XENADMIN_LAB_SHELL` (separated by `;;`) into the running
/// lab guest's console and save the resulting frame. Never asserts anything about the guest.
#[tokio::test]
#[ignore]
async fn lab_guest_shell_probe() {
    use xenadmin_core::PowerState;
    let lab = Lab::load();
    let script = std::env::var("XENADMIN_LAB_SHELL").expect("XENADMIN_LAB_SHELL");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let guest = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && v.power_state == PowerState::Running)
        .expect("running lab guest")
        .clone();
    let console = c
        .open_console(c.latest().observe_vm(guest.id).unwrap())
        .expect("console");
    stable_frame(&console, 240).await;
    for line in script.split(";;") {
        type_line(&console, line.trim()).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let frame = save_frame(&lab, &console, "shell-probe");
    console.close();
    c.close().await;
    eprintln!("frame: {frame:?}");
}
