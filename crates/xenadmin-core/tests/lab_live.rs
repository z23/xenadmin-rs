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

/// Inspection that tolerates an unreachable member instead of panicking.
async fn inspect_result(endpoint: &Endpoint) -> Option<xenadmin_core::CertificateObservation> {
    let task = CertificateInspectionTask::start(
        ConnectionOptions::new(endpoint.clone()),
        endpoint.clone(),
        None,
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(outcome) = task.take_result() {
                return outcome.ok();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .ok()
    .flatten()
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
    // Consoles and redirects reach other members directly, so pin every reachable member's
    // certificate as an additional verified host, as the desktop profile editor records.
    for other in 0..lab.hosts.len() {
        if other == index {
            continue;
        }
        let endpoint = lab.endpoint(other);
        if let Some(observation) = inspect_result(&endpoint).await {
            options.add_host_certificate(endpoint, observation.details.fingerprint.clone());
        }
    }
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
        .find(|p| p.host == Some(coordinator) && p.management == Some(true))
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
    for pixel in frame.rgba.as_chunks::<4>().0 {
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

/// P03-01, P03-02, P03-03, P04-07: every power action the host advertises for the lab guest,
/// in a sequence that returns it to Running, with the console reopened after each restart.
/// Actions the host does not advertise are recorded as unavailable, not attempted.
#[tokio::test]
#[ignore]
async fn lab_guest_power_matrix() {
    use xenadmin_core::{PowerAction, PowerState, VmCommand};
    let lab = Lab::load();
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
    let sequence: Vec<(PowerAction, PowerState)> = vec![
        (PowerAction::CleanShutdown, PowerState::Halted),
        (PowerAction::Start, PowerState::Running),
        (PowerAction::Reboot, PowerState::Running),
        (PowerAction::Pause, PowerState::Paused),
        (PowerAction::Unpause, PowerState::Running),
        (PowerAction::Suspend, PowerState::Suspended),
        (PowerAction::Resume, PowerState::Running),
        (PowerAction::HardReboot, PowerState::Running),
        (PowerAction::HardShutdown, PowerState::Halted),
        (PowerAction::Start, PowerState::Running),
    ];
    let mut results = Vec::new();
    for (action, expected) in sequence {
        let before = c.latest().inventory().unwrap().vms[&guest.id].clone();
        if !before.allowed_actions.contains(&action) {
            results.push(json!({"action": format!("{action:?}"), "advertised": false, "power_state": format!("{:?}", before.power_state)}));
            continue;
        }
        let console = (expected == PowerState::Running
            && before.power_state == PowerState::Running)
            .then(|| {
                c.open_console(c.latest().observe_vm(guest.id).unwrap())
                    .expect("console")
            });
        let operation = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action,
                },
            )
            .expect("power action accepted");
        let outcome = wait_vm_outcome(&operation, 300).await;
        let reached = settle(180, || {
            c.latest()
                .inventory()
                .and_then(|i| i.vms.get(&guest.id).map(|v| v.power_state == expected))
                .unwrap_or(false)
        })
        .await;
        let console_after = if let Some(console) = console {
            let frame = tokio::time::timeout(deadline(180), async {
                loop {
                    let snapshot = console.latest();
                    if snapshot.state == xenadmin_core::ConsoleState::Connected
                        && snapshot.frame.is_some()
                    {
                        return "frame after restart";
                    }
                    if let xenadmin_core::ConsoleState::Failed(error) = &snapshot.state {
                        return if error.is_empty() {
                            "failed"
                        } else {
                            "failed with error"
                        };
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            })
            .await
            .unwrap_or("no frame within deadline");
            console.close();
            Some(frame)
        } else {
            None
        };
        results.push(json!({
            "action": format!("{action:?}"), "advertised": true,
            "state": format!("{:?}", outcome.state), "detail": outcome.detail,
            "expected": format!("{expected:?}"), "reached": reached,
            "console_after_restart": console_after,
        }));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{action:?}: {}",
            outcome.detail
        );
        assert!(reached, "{action:?}: guest did not reach {expected:?}");
    }
    c.close().await;
    lab.record(
        "guest-power-matrix",
        json!({"guest": guest.name, "actions": results}),
    );
}

/// P05-08, P08-08: delete every lab snapshot with its exclusively owned disks through the
/// reviewed deletion, then delete the device-less guest left by the partial creation.
#[tokio::test]
#[ignore]
async fn lab_cleanup_snapshots_and_partial_guest() {
    use xenadmin_core::{VmCommand, VmLifecycleOperation};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let mut deleted = Vec::new();
    for snapshot in inventory
        .snapshots
        .for_vm(
            inventory
                .guests()
                .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
                .map(|v| v.id)
                .expect("lab guest"),
        )
        .filter(|s| s.name.starts_with("lab-marker-") || s.name.starts_with("probe-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        let vm = snapshot.vm.expect("snapshot parent");
        let disks = inventory
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(snapshot.id) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let operation = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(vm, snapshot.id).unwrap(),
                    disks: disks.clone(),
                },
            )
            .expect("snapshot deletion accepted");
        let outcome = wait_vm_outcome(&operation, 300).await;
        let gone = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.snapshots.get(&snapshot.id).is_none())
        })
        .await;
        deleted.push(json!({"snapshot": snapshot.name, "disks": disks.len(), "state": format!("{:?}", outcome.state), "detail": outcome.detail, "gone": gone}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}: {}",
            snapshot.name,
            outcome.detail
        );
        assert!(gone, "{} still listed", snapshot.name);
    }
    let mut partial = None;
    if let Some(leftover) = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| {
            v.name.starts_with("lab-guest-")
                && !v.is_template
                && c.latest()
                    .inventory()
                    .unwrap()
                    .details
                    .vm_disks
                    .get(&v.id)
                    .is_none_or(|d| d.is_empty())
        })
        .cloned()
    {
        let operation = c
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: c.latest().observe_vm(leftover.id).unwrap(),
                    operation: VmLifecycleOperation::Delete {
                        expected_template: false,
                        disks: Vec::new(),
                        snapshots: Vec::new(),
                    },
                },
            )
            .expect("deletion accepted");
        let outcome = wait_vm_outcome(&operation, 300).await;
        let gone = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| !i.vms.contains_key(&leftover.id))
        })
        .await;
        partial = Some(
            json!({"vm": leftover.name, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "gone": gone}),
        );
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(gone, "partial guest still listed");
    }
    c.close().await;
    lab.record(
        "cleanup-snapshots-and-partial-guest",
        json!({"snapshots": deleted, "partial_guest": partial}),
    );
}

/// P11-01, P11-03, P11-05: live-migrate the running lab guest to another pool member with
/// both disks relocated to that member's local repository, then open its console there.
#[tokio::test]
#[ignore]
async fn lab_guest_live_migration_with_storage() {
    use xenadmin_core::{DiskPlacement, NicPlacement, PowerState, SrId, VmCommand, VmMigration};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && v.power_state == PowerState::Running)
        .expect("running lab guest")
        .clone();
    let source_host = guest.host.expect("resident host");
    let destination = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != source_host && inventory.hosts[h].enabled)
        .expect("another enabled member");
    let destination_sr =
        local_sr_of(&inventory, destination).expect("destination local repository");
    let disks = inventory.details.vm_disks[&guest.id]
        .iter()
        .filter_map(|vbd| inventory.details.vbds.get(vbd))
        .filter(|b| b.kind == "Disk")
        .filter_map(|b| {
            let vdi = b.vdi?;
            let sr = inventory.details.vdis.get(&vdi)?.sr?;
            Some(DiskPlacement {
                vdi,
                expected_sr: sr,
                destination_sr,
            })
        })
        .collect::<Vec<_>>();
    let networks = inventory
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(guest.id))
        .filter_map(|v| {
            let network = v.network?;
            Some(NicPlacement {
                vif: v.id,
                expected_network: network,
                destination_network: network,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(disks.len(), 2, "expected the image and data disks");
    // Storage relocation needs an explicit IP-configured transfer network on the destination.
    let transfer_network = inventory
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(destination) && p.management == Some(true))
        .and_then(|p| p.network)
        .expect("destination management network");
    let migration = c
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: VmMigration {
                    destination,
                    expected_host: Some(source_host),
                    expected_affinity: None,
                    live: true,
                    disks: disks.clone(),
                    networks,
                    transfer_network: Some(transfer_network),
                },
            },
        )
        .expect("migration accepted");
    let outcome = wait_vm_outcome(&migration, 1200).await;
    let relocated = settle(300, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms.get(&guest.id).is_some_and(|v| {
                v.host == Some(destination) && v.power_state == PowerState::Running
            }) && i.details.vm_disks.get(&guest.id).is_some_and(|vbds| {
                vbds.iter()
                    .filter_map(|b| i.details.vbds.get(b))
                    .filter(|b| b.kind == "Disk")
                    .all(|b| {
                        b.vdi
                            .and_then(|v| i.details.vdis.get(&v))
                            .and_then(|v| v.sr)
                            == Some(destination_sr)
                    })
            })
        })
    })
    .await;
    let console = c
        .open_console(c.latest().observe_vm(guest.id).unwrap())
        .expect("console on destination");
    let frame = tokio::time::timeout(deadline(120), async {
        loop {
            let snapshot = console.latest();
            if snapshot.frame.is_some() && snapshot.state == xenadmin_core::ConsoleState::Connected
            {
                return true;
            }
            if matches!(snapshot.state, xenadmin_core::ConsoleState::Failed(_)) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .unwrap_or(false);
    let console_state = format!("{:?}", console.latest().state);
    console.close();
    c.close().await;
    lab.record(
        "guest-live-migration-with-storage",
        json!({
            "console_state": console_state,
            "guest": guest.name, "from": source_host.to_string(), "to": destination.to_string(),
            "destination_repository": SrId::to_string(&destination_sr),
            "operation": {"state": format!("{:?}", outcome.state), "detail": outcome.detail},
            "running_on_destination_with_relocated_disks": relocated,
            "console_on_destination": frame,
        }),
    );
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(
        relocated,
        "guest or its disks did not reach the destination"
    );
    assert!(frame, "no console frame from the destination host");
}

/// Diagnostic for P04-07: keep a console open across a hard reboot and record every
/// console state change with timestamps, including any failure message.
#[tokio::test]
#[ignore]
async fn lab_console_reconnect_after_hard_reboot() {
    use xenadmin_core::{ConsoleState, PowerAction, PowerState, VmCommand};
    let lab = Lab::load();
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
    stable_frame(&console, 180).await;
    let started = std::time::Instant::now();
    let mut transitions: Vec<Value> = Vec::new();
    let mut last = String::new();
    let note = |console: &xenadmin_core::Console,
                transitions: &mut Vec<Value>,
                last: &mut String| {
        let snapshot = console.latest();
        let state = match &snapshot.state {
            ConsoleState::Failed(error) => format!("Failed: {error}"),
            other => format!("{other:?}"),
        };
        let frame = snapshot.frame.as_ref().map(|f| (f.width, f.height));
        let key = format!("{state} {frame:?}");
        if key != *last {
            transitions.push(json!({"at_ms": started.elapsed().as_millis() as u64, "state": state, "frame": frame}));
            *last = key;
        }
    };
    note(&console, &mut transitions, &mut last);
    let operation = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::HardReboot,
            },
        )
        .expect("hard reboot accepted");
    let deadline_at = std::time::Instant::now() + deadline(180);
    let mut outcome = None;
    while std::time::Instant::now() < deadline_at {
        note(&console, &mut transitions, &mut last);
        if outcome.is_none() {
            let row = operation.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                outcome = Some(
                    json!({"state": format!("{:?}", row.state), "detail": row.detail, "at_ms": started.elapsed().as_millis() as u64}),
                );
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let final_state = console.latest().state.clone();
    console.close();
    // The core console is one tunnel; the desktop reopens it after a restart. Emulate the
    // operator's reopen once the guest is running again and record whether frames arrive.
    let running = settle(180, || {
        c.latest()
            .inventory()
            .and_then(|i| {
                i.vms
                    .get(&guest.id)
                    .map(|v| v.power_state == PowerState::Running)
            })
            .unwrap_or(false)
    })
    .await;
    let reopened = c
        .open_console(c.latest().observe_vm(guest.id).unwrap())
        .expect("console reopened");
    let frame_after_reopen = tokio::time::timeout(deadline(120), async {
        loop {
            let snapshot = reopened.latest();
            if snapshot.frame.is_some() && snapshot.state == ConsoleState::Connected {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .unwrap_or(false);
    reopened.close();
    c.close().await;
    lab.record(
        "console-reconnect-after-hard-reboot",
        json!({
            "guest": guest.name, "reboot": outcome, "transitions": transitions,
            "final_state": format!("{final_state:?}"),
            "running_again": running, "frame_after_manual_reopen": frame_after_reopen,
        }),
    );
    assert!(frame_after_reopen, "no frame after reopening the console");
}

/// Writes a saved connection profile for the lab coordinator, with every member's
/// certificate pinned, into `XENADMIN_LAB_DESKTOP_STATE` so the real desktop can cold-start
/// from a saved endpoint. The password is never stored; the desktop prompts for it.
#[tokio::test]
#[ignore]
async fn lab_write_desktop_profile() {
    use xenadmin_core::{ConnectionProfile, ProfileStore};
    let lab = Lab::load();
    let directory = PathBuf::from(
        std::env::var("XENADMIN_LAB_DESKTOP_STATE").expect("XENADMIN_LAB_DESKTOP_STATE"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    let mut store = ProfileStore::open(directory.clone()).expect("profile store");
    let mut profile = ConnectionProfile::new("Lab pool".into(), lab.endpoint(0), "root".into());
    for (index, host) in lab.hosts.iter().enumerate() {
        let endpoint = lab.endpoint(index);
        let observation = inspect_result(&endpoint)
            .await
            .unwrap_or_else(|| panic!("{} certificate", host.name));
        profile.host_certificates.insert(
            endpoint.to_string(),
            observation.details.fingerprint.clone(),
        );
    }
    let pins = profile.host_certificates.len();
    store.save(profile).expect("profile saved");
    eprintln!(
        "wrote profile with {pins} member pins to {}",
        directory.display()
    );
}

/// P14-07, P14-08: direct RRD metrics for the running lab guest and its host; records the
/// advertised sources and whether bounded sample series arrive within the ten-minute window.
#[tokio::test]
#[ignore]
async fn lab_metrics_guest_and_host() {
    use xenadmin_core::{MetricTarget, MetricsOptions, MetricsState, MetricsWindow, PowerState};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, true).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && v.power_state == PowerState::Running)
        .expect("running lab guest")
        .clone();
    let host = guest.host.expect("resident host");
    let mut results = Vec::new();
    for target in [MetricTarget::Vm(guest.id), MetricTarget::Host(host)] {
        let observed = c
            .latest()
            .observe_metrics(target)
            .expect("metrics target observed");
        let metrics = c
            .subscribe_metrics(
                observed,
                MetricsOptions {
                    window: MetricsWindow::TenMinutes,
                    sources: Vec::new(),
                },
            )
            .expect("metrics subscription accepted");
        let first = tokio::time::timeout(deadline(120), async {
            loop {
                let snapshot = metrics.latest();
                if !matches!(snapshot.state, MetricsState::Loading) {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .expect("metrics left the loading state");
        let chosen: Vec<String> = first
            .sources
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.name.clone())
            .filter(|n| {
                n.starts_with("cpu")
                    || n.starts_with("memory")
                    || n.contains("vif")
                    || n.contains("vbd")
                    || n.contains("pif")
            })
            .take(6)
            .collect();
        metrics.close();
        let metrics = c
            .subscribe_metrics(
                observed,
                MetricsOptions {
                    window: MetricsWindow::TenMinutes,
                    sources: chosen.clone(),
                },
            )
            .expect("metrics subscription with sources accepted");
        let with_samples = tokio::time::timeout(deadline(180), async {
            loop {
                let snapshot = metrics.latest();
                if snapshot.series.iter().any(|s| !s.samples.is_empty())
                    || matches!(snapshot.state, MetricsState::Failed)
                {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .expect("metrics samples or failure");
        results.push(json!({
            "target": format!("{target:?}"),
            "initial_state": format!("{:?}", first.state), "initial_error": first.error,
            "advertised_sources": first.sources.len(),
            "source_examples": first.sources.iter().take(8).map(|s| json!({"name": s.name, "units": s.units, "enabled": s.enabled})).collect::<Vec<_>>(),
            "requested": chosen,
            "state": format!("{:?}", with_samples.state), "error": with_samples.error,
            "sample_step_seconds": with_samples.sample_step_seconds,
            "series": with_samples.series.iter().map(|s| json!({"source": s.source, "samples": s.samples.len()})).collect::<Vec<_>>(),
        }));
        assert!(
            with_samples.series.iter().any(|s| !s.samples.is_empty()),
            "{target:?}: no samples ({:?})",
            with_samples.error
        );
        metrics.close();
    }
    c.close().await;
    lab.record("metrics-guest-and-host", json!({"targets": results}));
}

/// P06-01, P06-02, P06-04: reviewed metadata edits on the running guest, then vCPU and
/// memory edits while halted, each read back from the inventory; the guest is started again.
#[tokio::test]
#[ignore]
async fn lab_guest_settings_edits() {
    use xenadmin_core::{
        MemoryLimits, PowerAction, PowerState, VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut results = Vec::new();

    // Metadata while running.
    let new_description = format!("Renamed by the live harness at {stamp}");
    let patch = VmSettingsPatch::new(vec![VmSettingEdit::Description {
        previous: guest.description.clone(),
        value: new_description.clone(),
    }])
    .unwrap();
    let operation = c
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                patch,
            },
        )
        .expect("metadata edit accepted");
    let outcome = wait_vm_outcome(&operation, 120).await;
    let applied = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.description == new_description)
        })
    })
    .await;
    results.push(json!({"edit": "description", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": applied}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(applied);

    // Halt, then vCPUs and memory.
    if c.latest().inventory().unwrap().vms[&guest.id].power_state == PowerState::Running {
        let shutdown = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(
            settle(120, || c
                .latest()
                .inventory()
                .is_some_and(
                    |i| i.vms[&guest.id].power_state == PowerState::Halted
                ))
            .await
        );
    }
    let details = c.latest().inventory().unwrap().details.vms[&guest.id].clone();
    let previous = MemoryLimits {
        static_min: details.memory_static_min.expect("static min"),
        dynamic_min: details.memory_dynamic_min.expect("dynamic min"),
        dynamic_max: details.memory_dynamic_max.expect("dynamic max"),
        static_max: details.memory_static_max.expect("static max"),
    };
    let mib = 1024 * 1024;
    let target_memory = if previous.static_max == 768 * mib {
        640 * mib
    } else {
        768 * mib
    };
    let target_cpus = if details.vcpus_max == Some(2) { 1 } else { 2 };
    let patch = VmSettingsPatch::new(vec![
        VmSettingEdit::Cpus {
            previous_max: details.vcpus_max.expect("vcpus max"),
            previous_startup: details.vcpus_at_startup.expect("vcpus startup"),
            max: target_cpus,
            startup: target_cpus,
        },
        VmSettingEdit::Memory {
            previous: previous.clone(),
            value: MemoryLimits {
                static_min: previous.static_min,
                dynamic_min: target_memory,
                dynamic_max: target_memory,
                static_max: target_memory,
            },
        },
    ])
    .unwrap();
    let operation = c
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                patch,
            },
        )
        .expect("resource edit accepted");
    let outcome = wait_vm_outcome(&operation, 120).await;
    let applied = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details.vms.get(&guest.id).is_some_and(|d| {
                d.vcpus_max == Some(target_cpus)
                    && d.memory_static_max == Some(target_memory)
                    && d.memory_dynamic_max == Some(target_memory)
            })
        })
    })
    .await;
    results.push(json!({"edit": "cpus+memory", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": applied,
                        "steps": outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state)})).collect::<Vec<_>>(),
                        "cpus": target_cpus, "memory_bytes": target_memory}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(applied, "resources did not read back");

    let start = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(
                |i| i.vms[&guest.id].power_state == PowerState::Running
            ))
        .await
    );
    c.close().await;
    lab.record(
        "guest-settings-edits",
        json!({"guest": guest.name, "edits": results}),
    );
}

/// Runs a libvirt domain command for a lab host. The step changes lab infrastructure, so it
/// runs only when `XENADMIN_LAB_HOST_CONTROL=libvirt` names that authorisation explicitly.
fn virsh(args: &[&str]) -> String {
    assert_eq!(
        std::env::var("XENADMIN_LAB_HOST_CONTROL").as_deref(),
        Ok("libvirt"),
        "set XENADMIN_LAB_HOST_CONTROL=libvirt to let the harness stop and start lab hosts"
    );
    let output = std::process::Command::new("virsh")
        .args(["--connect", "qemu:///system"])
        .args(args)
        .output()
        .expect("virsh runs");
    assert!(
        output.status.success(),
        "virsh {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Samples a connection's visible state until `done`, keeping each change with its time.
async fn state_timeline<F: Fn(&Connection) -> bool>(
    connection: &Connection,
    seconds: u64,
    done: F,
) -> (Vec<Value>, bool) {
    let started = std::time::Instant::now();
    let mut timeline = Vec::new();
    let mut last = None;
    let finished = tokio::time::timeout(deadline(seconds), async {
        loop {
            let snapshot = connection.latest();
            let key = (
                format!("{:?}", snapshot.state()),
                snapshot.generation(),
                snapshot.active_endpoint().map(|e| e.to_string()),
                snapshot.error().map(str::to_string),
            );
            if last.as_ref() != Some(&key) {
                timeline.push(json!({
                    "at_ms": started.elapsed().as_millis() as u64,
                    "state": key.0, "generation": key.1, "endpoint": key.2, "error": key.3,
                }));
                last = Some(key);
            }
            if done(connection) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .is_ok();
    (timeline, finished)
}

/// Direct recovery, failure path: the coordinator loses power under a live read-write
/// session. The client must leave Ready, refuse a write while stale, come back under a
/// fresh generation once the host returns, and keep the write it completed before the
/// outage without repeating it. The lab guest runs on the coordinator, so the outage halts
/// it; the scenario starts it again through the recovered connection.
#[tokio::test]
#[ignore]
async fn lab_coordinator_outage_under_live_session() {
    use xenadmin_core::{PowerAction, PowerState, VmCommand, VmSettingEdit, VmSettingsPatch};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let coordinator = inventory.coordinator.expect("coordinator");
    assert_eq!(
        inventory.hosts[&coordinator].address, lab.hosts[0].ip,
        "the first manifest host is expected to be the coordinator"
    );
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // A completed write before the outage: it must survive untouched and unrepeated.
    let probe = format!("Outage probe {stamp}");
    let patch = VmSettingsPatch::new(vec![VmSettingEdit::Description {
        previous: guest.description.clone(),
        value: probe.clone(),
    }])
    .unwrap();
    let operation = c
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                patch,
            },
        )
        .expect("probe edit accepted");
    let probe_outcome = wait_vm_outcome(&operation, 120).await;
    assert_eq!(
        probe_outcome.state,
        OperationState::Succeeded,
        "{}",
        probe_outcome.detail
    );
    assert!(
        settle(60, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].description == probe))
        .await
    );
    let generation_before = c.latest().generation();
    let observed_before = c.latest().observe_vm(guest.id).unwrap();
    let resource_records_before = c.resource_history().len();
    let guest_state_before = format!(
        "{:?}",
        c.latest().inventory().unwrap().vms[&guest.id].power_state
    );

    // Outage: cut power to the coordinator's domain. (An ACPI shutdown request is ignored
    // by the nested XCP-ng dom0: the host kept serving and the client rightly stayed Ready.)
    let domain = lab.hosts[0].name.clone();
    let shutdown_at = std::time::Instant::now();
    virsh(&["destroy", &domain]);
    let (stale_timeline, stale_seen) = state_timeline(&c, 300, |c| !c.latest().is_current()).await;
    let stale_after_ms = shutdown_at.elapsed().as_millis() as u64;
    assert!(
        stale_seen,
        "the connection stayed Ready after the coordinator shut down"
    );
    let stale_state = format!("{:?}", c.latest().state());

    // A write while stale must be refused locally, before any request.
    let refused = c
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm: observed_before,
                patch: VmSettingsPatch::new(vec![VmSettingEdit::Description {
                    previous: probe.clone(),
                    value: format!("Must not apply {stamp}"),
                }])
                .unwrap(),
            },
        )
        .err()
        .map(|e| e.to_string());
    assert!(
        refused.is_some(),
        "a write was accepted while the connection was stale"
    );

    // The domain is off; the recovery below is a real cold return.
    let off = settle(60, || virsh(&["domstate", &domain]) == "shut off").await;
    assert!(off, "coordinator domain did not shut off");
    let off_after_ms = shutdown_at.elapsed().as_millis() as u64;
    let (down_timeline, _) = state_timeline(&c, 20, |_| false).await;

    // Return: start the domain and wait for a fresh generation.
    let start_at = std::time::Instant::now();
    virsh(&["start", &domain]);
    let (recovery_timeline, recovered) = state_timeline(&c, 900, |c| {
        let s = c.latest();
        s.is_current() && s.generation() > generation_before
    })
    .await;
    let ready_after_ms = start_at.elapsed().as_millis() as u64;
    assert!(
        recovered,
        "connection did not recover under a fresh generation"
    );
    let generation_after = c.latest().generation();
    let fresh = c.latest().inventory().unwrap().clone();
    // The host's own truth after the power cut: XAPI acknowledges metadata writes from
    // its in-memory database, so a coordinator power loss can discard a recent one. The
    // client must show what the host now holds, never its own memory of the write.
    let description_after = fresh.vms[&guest.id].description.clone();
    let write_survived = description_after == probe;
    let write_rolled_back = description_after == guest.description;
    let guest_state_after = format!("{:?}", fresh.vms[&guest.id].power_state);

    // The host shutdown halted the guest; start it again through the recovered session.
    let restart = if fresh.vms[&guest.id].power_state == PowerState::Halted {
        let start = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::Start,
                },
            )
            .expect("start accepted after recovery");
        let outcome = wait_vm_outcome(&start, 300).await;
        let running = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running)
        })
        .await;
        json!({"state": format!("{:?}", outcome.state), "detail": outcome.detail, "running": running})
    } else {
        json!({"skipped": guest_state_after})
    };

    lab.record(
        "coordinator-outage-under-live-session",
        json!({
            "coordinator": lab.hosts[0].name,
            "probe_write": {"state": format!("{:?}", probe_outcome.state), "value": probe, "survived_power_cut": write_survived, "rolled_back_to_previous": write_rolled_back},
            "guest_power_before": guest_state_before,
            "stale": {"detected_after_ms": stale_after_ms, "state": stale_state, "timeline": stale_timeline},
            "write_while_stale": refused,
            "domain_off_after_ms": off_after_ms,
            "while_down": down_timeline,
            "recovery": {"ready_after_ms": ready_after_ms, "generation_before": generation_before, "generation_after": generation_after, "timeline": recovery_timeline},
            "description_after": description_after,
            "guest_power_after": guest_state_after,
            "guest_restart": restart,
            "resource_records": {"before": resource_records_before, "after": c.resource_history().len()},
        }),
    );
    assert!(
        write_survived || write_rolled_back,
        "the description is neither the completed write nor its predecessor: {description_after}"
    );
    assert_eq!(
        c.resource_history().len(),
        resource_records_before,
        "the outage added resource operation records"
    );
    c.close().await;
}

/// The disk VDIs currently attached to a VM.
fn disks_of(
    inventory: &xenadmin_core::Inventory,
    vm: xenadmin_core::VmId,
) -> Vec<xenadmin_core::VdiId> {
    inventory
        .details
        .vm_disks
        .get(&vm)
        .into_iter()
        .flatten()
        .filter_map(|vbd| inventory.details.vbds.get(vbd))
        .filter(|b| b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect()
}

/// Waits for a VM operation record to reach an outcome in a connection's history, keeping
/// every state change seen on the way.
async fn wait_history_outcome(
    connection: &Connection,
    id: OperationId,
    seconds: u64,
) -> (Option<xenadmin_core::OperationRecord>, Vec<Value>) {
    let started = std::time::Instant::now();
    let mut seen = Vec::new();
    let mut last = None;
    let _ = tokio::time::timeout(deadline(seconds), async {
        loop {
            let record = connection
                .operation_history()
                .iter()
                .find(|r| r.id == id)
                .cloned();
            let key = record.as_ref().map(|r| (format!("{:?}", r.state), r.detail.clone()));
            if key != last {
                if let Some((state, detail)) = &key {
                    seen.push(json!({"at_ms": started.elapsed().as_millis() as u64, "state": state, "detail": detail}));
                }
                last = key;
            }
            if record
                .as_ref()
                .is_some_and(|r| r.state.terminal() || r.state == OperationState::OutcomeUnknown)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    let record = connection
        .operation_history()
        .iter()
        .find(|r| r.id == id)
        .cloned();
    (record, seen)
}

/// Snapshot marker row, failure column: a snapshot create interrupted by the application
/// closing mid-flight is reconciled on restart from the journal, never repeated, and the
/// host ends with at most one snapshot of that name.
#[tokio::test]
#[ignore]
async fn lab_interrupted_snapshot_create_resolves_on_restart() {
    use xenadmin_core::{OperationControl, SnapshotName, VmCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let name = format!("lab-interrupt-{stamp}");
    let id = OperationId::new();
    let guest_id;
    let state_at_close;
    {
        let backend = Backend::with_state_directory(state.path().into()).unwrap();
        let c = connect(&lab, &backend, 0, false).await;
        let guest = c
            .latest()
            .inventory()
            .unwrap()
            .guests()
            .find(|v| v.name.starts_with("lab-guest-"))
            .expect("lab guest")
            .clone();
        guest_id = guest.id;
        let operation = c
            .submit(
                id,
                VmCommand::CreateSnapshot {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    name: SnapshotName::new(&name).unwrap(),
                },
            )
            .expect("snapshot accepted");
        // Let the request leave, then stop like an application exiting mid-operation.
        tokio::time::sleep(Duration::from_millis(150)).await;
        state_at_close = format!("{:?}", operation.latest().state);
        c.close().await;
        drop(backend);
    }

    // Same state directory, new process: the journal must reconcile, not replay.
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let (record, timeline) = wait_history_outcome(&c, id, 300).await;
    let record = record.expect("interrupted record recovered into the history");
    let mut recheck = None;
    let record = if record.state == OperationState::OutcomeUnknown {
        // A recheck starts a tracking worker; wait for the record to move before judging it.
        let before = (format!("{:?}", record.state), record.detail.clone());
        c.control_operation(id, OperationControl::Recheck)
            .expect("recheck accepted");
        let moved = settle(120, || {
            c.operation_history()
                .iter()
                .find(|r| r.id == id)
                .is_some_and(|r| (format!("{:?}", r.state), r.detail.clone()) != before)
        })
        .await;
        let (after, more) = wait_history_outcome(&c, id, 300).await;
        let message = c.latest().error().map(str::to_string);
        recheck = Some(
            json!({"before": before.1, "moved": moved, "timeline": more, "connection_message": message}),
        );
        after.expect("record after recheck")
    } else {
        record
    };
    // Host truth: never more than one snapshot of that name.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let matching = inventory
        .snapshots
        .for_vm(guest_id)
        .filter(|s| s.name == name)
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert!(
        matching.len() <= 1,
        "duplicate snapshots: {}",
        matching.len()
    );
    if record.state == OperationState::Succeeded {
        assert_eq!(matching.len(), 1, "succeeded record without its snapshot");
    }

    // Leave the lab as found.
    let mut cleanup = Vec::new();
    for snapshot in &matching {
        let disks = inventory
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(*snapshot) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(guest_id, *snapshot).unwrap(),
                    disks,
                },
            )
            .expect("snapshot deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        cleanup.push(json!({"state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    }
    lab.record(
        "interrupted-snapshot-create",
        json!({
            "snapshot": name,
            "state_at_close": state_at_close,
            "recovered": {"state": format!("{:?}", record.state), "detail": record.detail, "timeline": timeline},
            "recheck": recheck,
            "snapshots_named": matching.len(),
            "cleanup": cleanup,
        }),
    );
    c.close().await;
}

/// P05-09, P08-05, P08-06, P08-08: a snapshot cloned to a new VM and to a template, the
/// clone booted and its data-disk marker read back through the console, the clone
/// converted to a template, and both templates deleted with their own disks.
#[tokio::test]
#[ignore]
async fn lab_snapshot_clone_to_vm_and_template() {
    use xenadmin_core::{PowerAction, PowerState, SnapshotName, VmCommand, VmLifecycleOperation};
    let lab = Lab::load();
    let gateway = std::env::var("XENADMIN_LAB_GATEWAY").unwrap_or_else(|_| "192.168.254.1".into());
    let port: u16 = 8000;
    let clone_ip =
        std::env::var("XENADMIN_LAB_CLONE_IP").unwrap_or_else(|_| "192.168.254.101".into());
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Leftovers of an earlier interrupted run go first, through the same reviewed deletions.
    let mut leftovers = Vec::new();
    for vm in inventory
        .vms
        .values()
        .filter(|v| v.name.starts_with("lab-clone-") && v.id != guest.id)
        .cloned()
        .collect::<Vec<_>>()
    {
        let disks = disks_of(c.latest().inventory().unwrap(), vm.id);
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: c.latest().observe_vm(vm.id).unwrap(),
                    operation: VmLifecycleOperation::Delete {
                        expected_template: vm.is_template,
                        disks,
                        snapshots: Vec::new(),
                    },
                },
            )
            .expect("leftover deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        leftovers.push(json!({"vm": vm.name, "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    }
    for snapshot in inventory
        .snapshots
        .for_vm(guest.id)
        .filter(|s| s.name.starts_with("lab-clone-src-"))
        .map(|s| s.id)
        .collect::<Vec<_>>()
    {
        let disks = inventory
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(snapshot) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(guest.id, snapshot).unwrap(),
                    disks,
                },
            )
            .expect("leftover snapshot deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        leftovers.push(json!({"snapshot": snapshot.to_string(), "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    }
    if !leftovers.is_empty() {
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let inventory = c.latest().inventory().unwrap().clone();
    let vm_count_before = inventory.vms.len();

    // Source snapshot.
    let source_name = format!("lab-clone-src-{stamp}");
    let snapshot = c
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                name: SnapshotName::new(&source_name).unwrap(),
            },
        )
        .expect("snapshot accepted");
    let outcome = wait_vm_outcome(&snapshot, 300).await;
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let source = tokio::time::timeout(deadline(60), async {
        loop {
            if let Some(id) = c.latest().inventory().and_then(|i| {
                i.snapshots
                    .for_vm(guest.id)
                    .find(|s| s.name == source_name)
                    .map(|s| s.id)
            }) {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("source snapshot visible");

    // Clone to a new VM, then to a template.
    let mut clones = Vec::new();
    let mut created = Vec::new();
    for (as_template, label) in [(false, "vm"), (true, "tpl")] {
        let name = format!("lab-clone-{label}-{stamp}");
        let operation = c
            .submit(
                OperationId::new(),
                VmCommand::CloneSnapshot {
                    snapshot: c.latest().observe_snapshot(guest.id, source).unwrap(),
                    name: SnapshotName::new(&name).unwrap(),
                    as_template,
                },
            )
            .expect("clone accepted");
        let outcome = wait_vm_outcome(&operation, 600).await;
        let found = tokio::time::timeout(deadline(120), async {
            loop {
                if let Some(vm) = c.latest().inventory().and_then(|i| {
                    i.vms
                        .values()
                        .find(|v| v.name == name && v.id != guest.id)
                        .cloned()
                }) {
                    return vm;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{name} not visible: {}", outcome.detail));
        let disks = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| !disks_of(i, found.id).is_empty())
        })
        .await;
        let inventory = c.latest().inventory().unwrap().clone();
        clones.push(json!({
            "name": name, "as_template": as_template, "state": format!("{:?}", outcome.state), "detail": outcome.detail,
            "id": found.id.to_string(), "is_template": found.is_template, "power_state": format!("{:?}", found.power_state),
            "disks": disks_of(&inventory, found.id).len(), "disks_visible": disks,
        }));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert_eq!(found.is_template, as_template);
        assert_ne!(found.id, guest.id);
        created.push(found);
    }
    let clone = created[0].clone();
    let template = created[1].clone();
    let source_disks = disks_of(c.latest().inventory().unwrap(), guest.id);
    let clone_disks = disks_of(c.latest().inventory().unwrap(), clone.id);
    let shared_disks = clone_disks
        .iter()
        .filter(|d| source_disks.contains(d))
        .map(|d| d.to_string())
        .collect::<Vec<_>>();
    let disk_sets = json!({
        "source": source_disks.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        "clone": clone_disks.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
        "shared": shared_disks,
    });

    // Boot the clone and read the marker its data disk carries. A new VM's allowed
    // actions arrive with a later event than its record, so wait for Start to be offered.
    let start_offered = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&clone.id)
                .is_some_and(|v| v.allowed_actions.contains(&PowerAction::Start))
        })
    })
    .await;
    assert!(start_offered, "the clone never offered Start");
    let reports = report_listener(&format!("{gateway}:{port}")).await;
    let start = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(clone.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("clone start accepted");
    let start_outcome = wait_vm_outcome(&start, 300).await;
    assert_eq!(
        start_outcome.state,
        OperationState::Succeeded,
        "{}",
        start_outcome.detail
    );
    assert!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(
                |i| i.vms[&clone.id].power_state == PowerState::Running
            ))
        .await
    );
    let console = c
        .open_console(c.latest().observe_vm(clone.id).unwrap())
        .expect("clone console");
    guest_report_marker(&console, &gateway, port, &clone_ip, None).await;
    let marker_reported = wait_report(&reports, "A", 120).await;
    let reported = reports.received.lock().unwrap().clone();
    drop(console);

    // Halt the clone and convert it to a template.
    let shutdown = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(clone.id).unwrap(),
                action: PowerAction::CleanShutdown,
            },
        )
        .expect("clone shutdown accepted");
    let shutdown_outcome = wait_vm_outcome(&shutdown, 300).await;
    assert_eq!(
        shutdown_outcome.state,
        OperationState::Succeeded,
        "{}",
        shutdown_outcome.detail
    );
    assert!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(
                |i| i.vms[&clone.id].power_state == PowerState::Halted
            ))
        .await
    );
    let convert = c
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(clone.id).unwrap(),
                operation: VmLifecycleOperation::ConvertToTemplate,
            },
        )
        .expect("conversion accepted");
    let convert_outcome = wait_vm_outcome(&convert, 300).await;
    let converted = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms.get(&clone.id).is_some_and(|v| v.is_template))
    })
    .await;
    assert_eq!(
        convert_outcome.state,
        OperationState::Succeeded,
        "{}",
        convert_outcome.detail
    );
    assert!(converted, "clone did not become a template");

    // Delete both templates with their own disks, then the source snapshot.
    let mut deletions = Vec::new();
    for target in [clone.id, template.id] {
        let disks = disks_of(c.latest().inventory().unwrap(), target);
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: c.latest().observe_vm(target).unwrap(),
                    operation: VmLifecycleOperation::Delete {
                        expected_template: true,
                        disks: disks.clone(),
                        snapshots: Vec::new(),
                    },
                },
            )
            .expect("template deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        let gone = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                !i.vms.contains_key(&target)
                    && disks.iter().all(|d| !i.details.vdis.contains_key(d))
            })
        })
        .await;
        deletions.push(json!({"target": target.to_string(), "disks": disks.len(), "state": format!("{:?}", outcome.state), "detail": outcome.detail, "gone_with_disks": gone}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(gone);
    }
    let inventory = c.latest().inventory().unwrap().clone();
    let snapshot_disks = inventory
        .details
        .vbds
        .values()
        .filter(|b| b.snapshot == Some(source) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect::<Vec<_>>();
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(guest.id, source).unwrap(),
                disks: snapshot_disks,
            },
        )
        .expect("source snapshot deletion accepted");
    let snapshot_deletion = wait_vm_outcome(&deletion, 300).await;
    let source_intact = c.latest().inventory().unwrap().vms.contains_key(&guest.id)
        && disks_of(c.latest().inventory().unwrap(), guest.id) == source_disks;
    let vm_count_after = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms.len() == vm_count_before)
    })
    .await;
    reports.task.abort();
    lab.record(
        "snapshot-clone-to-vm-and-template",
        json!({
            "leftovers_removed_first": leftovers,
            "source_snapshot": source_name,
            "clones": clones,
            "disk_sets": disk_sets,
            "clone_marker": {"reported": marker_reported, "received": reported},
            "clone_start": {"state": format!("{:?}", start_outcome.state), "detail": start_outcome.detail},
            "convert_to_template": {"state": format!("{:?}", convert_outcome.state), "detail": convert_outcome.detail},
            "deletions": deletions,
            "source_snapshot_deletion": {"state": format!("{:?}", snapshot_deletion.state), "detail": snapshot_deletion.detail},
            "source_guest_intact": source_intact,
            "vm_count_restored": vm_count_after,
        }),
    );
    assert!(source_intact);
    assert!(
        disk_sets["shared"].as_array().is_some_and(|v| v.is_empty()),
        "the clone shared a disk with its source: {}",
        disk_sets["shared"]
    );
    c.close().await;
}

/// P07-04, P07-06, P07-07, P07-08, P07-09: a scratch disk on the halted lab guest is
/// renamed, repositioned, detached with its data kept, reattached, grown, copied to another
/// member's local repository, the copy cleaned up and the scratch disk deleted. The guest's
/// own image and data disks are never touched, and the guest is started again at the end.
#[tokio::test]
#[ignore]
async fn lab_disk_lifecycle_on_halted_guest() {
    use xenadmin_core::{
        DeviceCommand, DeviceOperation, DeviceProperty, DiskOperation, OwnedDiskSource,
        PowerAction, PowerState, ResourceCommand, ResourceTarget, ReviewedDisk, VmCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let original_disks = disks_of(&inventory, guest.id);
    let mut steps = Vec::new();
    let mib = 1024 * 1024;

    async fn device(
        c: &Connection,
        vm: xenadmin_core::VmId,
        operation: DeviceOperation,
        seconds: u64,
    ) -> std::sync::Arc<xenadmin_core::OperationRecord> {
        let op = c
            .submit(
                OperationId::new(),
                VmCommand::Device(DeviceCommand {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    operation,
                }),
            )
            .expect("device command accepted");
        wait_vm_outcome(&op, seconds).await
    }
    fn step(
        steps: &mut Vec<Value>,
        label: &str,
        record: &xenadmin_core::OperationRecord,
        check: bool,
    ) {
        steps.push(json!({"step": label, "state": format!("{:?}", record.state), "detail": record.detail, "verified": check}));
        assert_eq!(
            record.state,
            OperationState::Succeeded,
            "{label}: {}",
            record.detail
        );
        assert!(check, "{label}: host state did not read back");
    }

    // Halt the guest for the attachment changes.
    let was_running = guest.power_state == PowerState::Running;
    if was_running {
        let shutdown = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Halted)
        })
        .await;
        step(&mut steps, "clean shutdown", &outcome, halted);
    }
    let host = c.latest().inventory().unwrap().vms[&guest.id]
        .host
        .or(inventory.coordinator)
        .expect("guest host");
    let sr = local_sr_of(c.latest().inventory().unwrap(), host).expect("local repository");

    // Scratch disk.
    let scratch_name = format!("lab-scratch-{stamp}");
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::CreateDisk {
            sr,
            name: scratch_name.clone(),
            size: 256 * mib,
            device: "2".into(),
        },
        300,
    )
    .await;
    let create_id = outcome.id;
    let scratch = tokio::time::timeout(deadline(60), async {
        loop {
            let found = c.latest().inventory().and_then(|i| {
                i.details
                    .vdis
                    .values()
                    .find(|v| v.name == scratch_name)
                    .map(|v| v.id)
            });
            if let Some(id) = found {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    step(
        &mut steps,
        "create 256 MiB scratch disk",
        &outcome,
        scratch.is_ok(),
    );
    let scratch = scratch.unwrap();
    let vbd_of = |c: &Connection, vdi| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vbds
                .values()
                .find(|b| b.vdi == Some(vdi) && b.vm == Some(guest.id))
                .map(|b| (b.id, b.userdevice.clone()))
        })
    };
    let (vbd, position) = vbd_of(&c, scratch).expect("scratch attachment");

    // Rename and describe.
    let renamed = format!("lab-scratch-renamed-{stamp}");
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::Property {
            change: DeviceProperty::DiskMetadata {
                vbd,
                vdi: scratch,
                previous_name: scratch_name.clone(),
                previous_description: String::new(),
                name: renamed.clone(),
                description: "Scratch disk for the live disk lifecycle scenario".into(),
            },
        },
        120,
    )
    .await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vdis
                .get(&scratch)
                .is_some_and(|v| v.name == renamed)
        })
    })
    .await;
    step(&mut steps, "rename disk", &outcome, check);

    // Reposition.
    let new_position = if position == "3" {
        "4".to_string()
    } else {
        "3".to_string()
    };
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::Property {
            change: DeviceProperty::DiskPosition {
                vbd,
                vdi: scratch,
                previous: position.clone(),
                value: new_position.clone(),
            },
        },
        120,
    )
    .await;
    let check = settle(60, || {
        vbd_of(&c, scratch).is_some_and(|(_, p)| p == new_position)
    })
    .await;
    step(&mut steps, "change device position", &outcome, check);

    // Detach, keeping the data; the VDI must survive without an attachment.
    let (vbd, _) = vbd_of(&c, scratch).expect("scratch attachment");
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::DetachDisk { vbd, vdi: scratch },
        120,
    )
    .await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details.vdis.contains_key(&scratch) && !i.details.vbds.contains_key(&vbd)
        })
    })
    .await;
    step(&mut steps, "detach without deleting", &outcome, check);

    // Grow while detached, then copy to another member's local repository.
    let reviewed = ReviewedDisk::from_inventory(c.latest().inventory().unwrap(), scratch)
        .expect("reviewed disk");
    let grow = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Disk {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Disk(scratch))
                    .unwrap(),
                operation: DiskOperation::Grow {
                    expected: reviewed,
                    size: 512 * mib,
                },
            },
        )
        .expect("grow accepted");
    let outcome = wait_outcome(&grow, 300).await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details.vdis.get(&scratch).and_then(|v| v.virtual_size) == Some(512 * mib)
        })
    })
    .await;
    steps.push(json!({"step": "grow to 512 MiB", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(check);

    let other = c
        .latest()
        .inventory()
        .unwrap()
        .hosts
        .keys()
        .copied()
        .find(|h| *h != host)
        .expect("another member");
    let destination =
        local_sr_of(c.latest().inventory().unwrap(), other).expect("destination repository");
    let copy_name = format!("lab-scratch-copy-{stamp}");
    let reviewed = ReviewedDisk::from_inventory(c.latest().inventory().unwrap(), scratch)
        .expect("reviewed disk");
    let copy = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Disk {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Disk(scratch))
                    .unwrap(),
                operation: DiskOperation::Copy {
                    expected: reviewed,
                    destination,
                    name: copy_name.clone(),
                },
            },
        )
        .expect("copy accepted");
    let copy_id = copy.latest().id;
    let outcome = wait_outcome(&copy, 600).await;
    let copied = tokio::time::timeout(deadline(60), async {
        loop {
            let found = c.latest().inventory().and_then(|i| {
                i.details
                    .vdis
                    .values()
                    .find(|v| v.name == copy_name && v.sr == Some(destination))
                    .map(|v| v.id)
            });
            if let Some(id) = found {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    steps.push(json!({"step": "copy to another member's repository", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": copied.is_ok(), "destination": destination.to_string()}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let copied = copied.expect("copy visible on the destination repository");

    // Delete the detached copy through the owned-disk cleanup.
    let reviewed = ReviewedDisk::from_inventory(c.latest().inventory().unwrap(), copied)
        .expect("reviewed copy");
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Disk {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Disk(copied))
                    .unwrap(),
                operation: DiskOperation::Cleanup {
                    expected: reviewed,
                    source: OwnedDiskSource::ResourceOperation(copy_id),
                },
            },
        )
        .expect("cleanup accepted");
    let outcome = wait_outcome(&cleanup, 300).await;
    let check = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| !i.details.vdis.contains_key(&copied))
    })
    .await;
    steps.push(json!({"step": "delete the detached copy", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(check);

    // Reattach (the new attachment does not own the disk, so deleting through it is
    // refused by design), detach again, and delete the detached disk through its owner.
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::AttachDisk {
            vdi: scratch,
            device: "2".into(),
            read_only: false,
        },
        120,
    )
    .await;
    let check = settle(60, || vbd_of(&c, scratch).is_some()).await;
    step(&mut steps, "reattach", &outcome, check);
    let (vbd, _) = vbd_of(&c, scratch).expect("scratch attachment");
    // The inventory does not yet know the new attachment's ownership, so the request is
    // accepted for review and refused by the fresh host checks at dispatch.
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::DeleteDisk { vbd, vdi: scratch },
        120,
    )
    .await;
    let still_there = c.latest().inventory().is_some_and(|i| {
        i.details.vdis.contains_key(&scratch) && i.details.vbds.contains_key(&vbd)
    });
    steps.push(json!({"step": "delete through an attachment that does not own the disk", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "disk_and_attachment_kept": still_there}));
    assert_eq!(
        outcome.state,
        OperationState::Rejected,
        "deletion through a non-owning attachment: {}",
        outcome.detail
    );
    assert!(
        still_there,
        "the refused deletion changed the disk or its attachment"
    );
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::DetachDisk { vbd, vdi: scratch },
        120,
    )
    .await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details.vdis.contains_key(&scratch) && !i.details.vbds.contains_key(&vbd)
        })
    })
    .await;
    step(&mut steps, "detach again", &outcome, check);
    let reviewed = ReviewedDisk::from_inventory(c.latest().inventory().unwrap(), scratch)
        .expect("reviewed disk");
    let cleanup = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Disk {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Disk(scratch))
                    .unwrap(),
                operation: DiskOperation::Cleanup {
                    expected: reviewed,
                    source: OwnedDiskSource::VmOperation(create_id),
                },
            },
        )
        .expect("owned disk cleanup accepted");
    let outcome = wait_outcome(&cleanup, 300).await;
    let check = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| !i.details.vdis.contains_key(&scratch))
    })
    .await;
    steps.push(json!({"step": "delete the detached disk through its creating operation", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(check);

    // A second owned disk deleted together with its attachment.
    let second_name = format!("lab-scratch-second-{stamp}");
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::CreateDisk {
            sr,
            name: second_name.clone(),
            size: 64 * mib,
            device: "2".into(),
        },
        300,
    )
    .await;
    let second = tokio::time::timeout(deadline(60), async {
        loop {
            let found = c.latest().inventory().and_then(|i| {
                i.details
                    .vdis
                    .values()
                    .find(|v| v.name == second_name)
                    .map(|v| v.id)
            });
            if let Some(id) = found {
                return id;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    step(
        &mut steps,
        "create a second owned disk",
        &outcome,
        second.is_ok(),
    );
    let second = second.unwrap();
    let owned = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vbds
                .values()
                .any(|b| b.vdi == Some(second) && b.owned && b.currently_attached == Some(false))
        })
    })
    .await;
    assert!(
        owned,
        "the creating attachment never showed as owning the disk"
    );
    let (vbd, _) = vbd_of(&c, second).expect("second attachment");
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::DeleteDisk { vbd, vdi: second },
        120,
    )
    .await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            !i.details.vdis.contains_key(&second) && !i.details.vbds.contains_key(&vbd)
        })
    })
    .await;
    step(
        &mut steps,
        "delete disk with its owning attachment",
        &outcome,
        check,
    );
    let intact = disks_of(c.latest().inventory().unwrap(), guest.id) == original_disks;

    // The lab's steady state is a running guest.
    let start = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    let running = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running)
    })
    .await;
    step(&mut steps, "start", &outcome, running);
    lab.record(
        "disk-lifecycle-on-halted-guest",
        json!({"guest": guest.name, "steps": steps, "original_disks_intact": intact}),
    );
    assert!(intact, "the guest's original disks changed");
    c.close().await;
}

/// P01-02, P01-07: login through a non-coordinator member follows the coordinator redirect
/// to the same pool; a second connection to another member of that pool with the same
/// credentials shares the pool; wrong credentials through a member are refused.
#[tokio::test]
#[ignore]
async fn lab_member_login_and_pool_alias() {
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 2, "two pool members are required");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();

    // Through the member first.
    let member = connect(&lab, &backend, 1, false).await;
    let member_summary = inventory_summary(&member);
    let member_snapshot = member.latest();
    let member_route = json!({
        "requested": lab.hosts[1].ip,
        "active_endpoint": member_snapshot.active_endpoint().map(|e| e.to_string()),
        "route": member_snapshot.connection_route(),
        "verified_members": member_snapshot.verified_members().iter().map(|e| e.to_string()).collect::<Vec<_>>(),
    });

    // Then the coordinator, in the same backend: same pool, shared ownership.
    let coordinator = connect(&lab, &backend, 0, false).await;
    let coordinator_summary = inventory_summary(&coordinator);
    let same_pool = member_summary["pool_id"] == coordinator_summary["pool_id"];
    let both_ready = settle(30, || {
        member.latest().is_current() && coordinator.latest().is_current()
    })
    .await;
    let same_revision = settle(30, || {
        member.latest().inventory().map(|i| i.revision)
            == coordinator.latest().inventory().map(|i| i.revision)
    })
    .await;

    // Closing the member's handle leaves the coordinator's connection in place.
    member.close().await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let coordinator_after_member_closed = format!("{:?}", coordinator.latest().state());
    coordinator.close().await;

    // Wrong credentials through the member are refused once the redirect target is
    // pinned; with only the member pinned, the redirect itself is refused before any
    // credential reaches the coordinator.
    let wrong = || Credentials::new("root".into(), format!("{}-wrong", lab.password));
    let (member_only, _) = pinned_options(&lab.endpoint(1)).await;
    let refused = backend.connect(member_only, wrong());
    let unpinned_redirect = refused
        .wait_ready(deadline(60))
        .await
        .err()
        .map(|e| e.to_string());
    refused.close().await;
    let (mut all_pinned, _) = pinned_options(&lab.endpoint(1)).await;
    for other in 0..lab.hosts.len() {
        if other != 1
            && let Some(observation) = inspect_result(&lab.endpoint(other)).await
        {
            all_pinned
                .add_host_certificate(lab.endpoint(other), observation.details.fingerprint.clone());
        }
    }
    let refused = backend.connect(all_pinned, wrong());
    let refused_error = refused
        .wait_ready(deadline(60))
        .await
        .err()
        .map(|e| e.to_string());
    let refused_state = format!("{:?}", refused.latest().state());
    refused.close().await;

    lab.record(
        "member-login-and-pool-alias",
        json!({
            "member": member_route,
            "member_inventory": member_summary,
            "coordinator_inventory": coordinator_summary,
            "same_pool": same_pool,
            "both_ready": both_ready,
            "same_revision": same_revision,
            "coordinator_after_member_closed": coordinator_after_member_closed,
            "wrong_password_via_member": {"error": refused_error, "state": refused_state},
            "redirect_to_unpinned_coordinator": unpinned_redirect,
        }),
    );
    assert!(same_pool, "member login reached a different pool");
    assert!(both_ready);
    assert!(refused_error.is_some(), "wrong credentials were accepted");
}

/// P10-01, P10-02, P10-04 and the NIC rows of P07: an internal network is created, its MTU
/// and name changed, a NIC on it added to the halted guest and removed again, and the
/// network deleted; the guest's own NIC and network are untouched and it is started again.
#[tokio::test]
#[ignore]
async fn lab_internal_network_and_nic_lifecycle() {
    use xenadmin_core::{
        DeviceCommand, DeviceOperation, PowerAction, PowerState, ResourceCommand, ResourceTarget,
        VmCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    fn step(steps: &mut Vec<Value>, label: &str, state: OperationState, detail: &str, check: bool) {
        steps.push(json!({"step": label, "state": format!("{state:?}"), "detail": detail, "verified": check}));
        assert_eq!(state, OperationState::Succeeded, "{label}: {detail}");
        assert!(check, "{label}: host state did not read back");
    }
    let original_nics = inventory
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(guest.id))
        .map(|v| v.id)
        .collect::<Vec<_>>();
    let network_count = inventory.networks.len();

    // Create.
    let name = format!("lab-internal-{stamp}");
    let create = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::CreateInternalNetwork {
                pool: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                    .unwrap(),
                name: name.clone(),
                description: "Internal network for the live network scenario".into(),
                mtu: 1500,
            },
        )
        .expect("network creation accepted");
    let outcome = wait_outcome(&create, 300).await;
    let network = tokio::time::timeout(deadline(60), async {
        loop {
            if let Some(n) = c
                .latest()
                .inventory()
                .and_then(|i| i.networks.iter().find(|n| n.name == name).cloned())
            {
                return n;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    step(
        &mut steps,
        "create internal network",
        outcome.state.clone(),
        &outcome.detail,
        network.is_ok(),
    );
    let network = network.unwrap();
    let network_id: xenadmin_core::NetworkId = network.uuid.into();
    let bridge = network.bridge.clone();

    // MTU and name.
    let configure = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::ConfigureInternalNetwork {
                network: c
                    .latest()
                    .observe_resource(ResourceTarget::Network(network_id))
                    .unwrap(),
                expected_mtu: 1500,
                mtu: 1400,
            },
        )
        .expect("mtu change accepted");
    let outcome = wait_outcome(&configure, 120).await;
    let check = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.details.networks.get(&network_id).and_then(|n| n.mtu) == Some(1400))
    })
    .await;
    step(
        &mut steps,
        "set MTU 1400",
        outcome.state.clone(),
        &outcome.detail,
        check,
    );
    let renamed = format!("lab-internal-renamed-{stamp}");
    let rename = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::RenameMetadata {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Network(network_id))
                    .unwrap(),
                expected_name: name.clone(),
                expected_description: "Internal network for the live network scenario".into(),
                name: renamed.clone(),
                description: "Renamed by the live network scenario".into(),
            },
        )
        .expect("rename accepted");
    let outcome = wait_outcome(&rename, 120).await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.networks
                .iter()
                .any(|n| n.uuid == network.uuid && n.name == renamed)
        })
    })
    .await;
    step(
        &mut steps,
        "rename and describe",
        outcome.state.clone(),
        &outcome.detail,
        check,
    );

    // A NIC on the halted guest.
    if guest.power_state == PowerState::Running {
        let shutdown = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Halted)
        })
        .await;
        step(
            &mut steps,
            "clean shutdown",
            outcome.state.clone(),
            &outcome.detail,
            halted,
        );
    }
    let add = c
        .submit(
            OperationId::new(),
            VmCommand::Device(DeviceCommand {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: DeviceOperation::AddNic {
                    network: network_id,
                    device: "1".into(),
                    mac: None,
                },
            }),
        )
        .expect("nic addition accepted");
    let outcome = wait_vm_outcome(&add, 120).await;
    let nic = tokio::time::timeout(deadline(60), async {
        loop {
            let found = c.latest().inventory().and_then(|i| {
                i.details
                    .vifs
                    .values()
                    .find(|v| v.vm == Some(guest.id) && v.network == Some(network_id))
                    .map(|v| (v.id, v.mac.clone(), v.device.clone()))
            });
            if let Some(found) = found {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    step(
        &mut steps,
        "add NIC on the internal network",
        outcome.state.clone(),
        &outcome.detail,
        nic.is_ok(),
    );
    let (vif, mac, device) = nic.unwrap();
    steps.push(json!({"step": "nic", "device": device, "mac_assigned": !mac.is_empty()}));
    let remove = c
        .submit(
            OperationId::new(),
            VmCommand::Device(DeviceCommand {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: DeviceOperation::DeleteNic { vif },
            }),
        )
        .expect("nic removal accepted");
    let outcome = wait_vm_outcome(&remove, 120).await;
    let check = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| !i.details.vifs.contains_key(&vif))
    })
    .await;
    step(
        &mut steps,
        "remove NIC",
        outcome.state.clone(),
        &outcome.detail,
        check,
    );

    // Delete the network, then start the guest.
    let delete = c
        .submit_resource(
            OperationId::new(),
            ResourceCommand::DeleteInternalNetwork {
                network: c
                    .latest()
                    .observe_resource(ResourceTarget::Network(network_id))
                    .unwrap(),
                expected_name: renamed.clone(),
            },
        )
        .expect("network deletion accepted");
    let outcome = wait_outcome(&delete, 120).await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            !i.networks.iter().any(|n| n.uuid == network.uuid) && i.networks.len() == network_count
        })
    })
    .await;
    step(
        &mut steps,
        "delete internal network",
        outcome.state.clone(),
        &outcome.detail,
        check,
    );
    let start = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    let running = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running)
    })
    .await;
    step(
        &mut steps,
        "start",
        outcome.state.clone(),
        &outcome.detail,
        running,
    );
    let nics_after = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(guest.id))
        .map(|v| v.id)
        .collect::<Vec<_>>();
    lab.record(
        "internal-network-and-nic-lifecycle",
        json!({"guest": guest.name, "network": {"name": name, "bridge": bridge}, "steps": steps, "original_nics_intact": nics_after == original_nics}),
    );
    assert_eq!(nics_after, original_nics, "the guest's own NICs changed");
    c.close().await;
}

/// P01-06, P11-07 and the transfer rows: with the third host standalone, the halted lab
/// guest is copied across pools over the lab network, the copy is booted on the other pool
/// and reports the data-disk marker through its console, and the copy is then deleted with
/// its disks; both pools stay connected in one client throughout and the guest is started
/// again at the end.
#[tokio::test]
#[ignore]
async fn lab_cross_pool_copy_and_verify() {
    use xenadmin_core::{
        CrossPoolMode, CrossPoolVmOperation, CrossPoolVmSelection, DiskPlacement, NicPlacement,
        PowerAction, PowerState, ResourceTarget, VmCommand, VmLifecycleOperation,
    };
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 3, "three hosts are required");
    let gateway = std::env::var("XENADMIN_LAB_GATEWAY").unwrap_or_else(|_| "192.168.254.1".into());
    let port: u16 = 8000;
    let copy_ip =
        std::env::var("XENADMIN_LAB_COPY_IP").unwrap_or_else(|_| "192.168.254.102".into());
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let source = connect(&lab, &backend, 0, false).await;
    let destination = connect(&lab, &backend, 2, false).await;
    let source_inventory = source.latest().inventory().unwrap().clone();
    let destination_inventory = destination.latest().inventory().unwrap().clone();
    assert_ne!(
        source_inventory.pool_id, destination_inventory.pool_id,
        "the third host must be a separate pool; run the reviewed ejection first"
    );
    let guest = source_inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let mut steps = Vec::new();

    // Halt the guest for a halted copy.
    if guest.power_state == PowerState::Running {
        let shutdown = source
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: source.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            source
                .latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Halted)
        })
        .await;
        steps.push(json!({"step": "clean shutdown", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": halted}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(halted);
    }

    // Placement: every disk to the destination host's local repository, the NIC to the
    // destination network with the same name, transfer over the destination host's address.
    let source_inventory = source.latest().inventory().unwrap().clone();
    let destination_host = *destination_inventory
        .hosts
        .keys()
        .next()
        .expect("destination host");
    let destination_sr =
        local_sr_of(&destination_inventory, destination_host).expect("destination repository");
    let disks = source_inventory.details.vm_disks[&guest.id]
        .iter()
        .filter_map(|vbd| source_inventory.details.vbds.get(vbd))
        .filter(|b| b.kind == "Disk")
        .filter_map(|b| {
            let vdi = b.vdi?;
            Some(DiskPlacement {
                vdi,
                expected_sr: source_inventory.details.vdis.get(&vdi)?.sr?,
                destination_sr,
            })
        })
        .collect::<Vec<_>>();
    let networks = source_inventory
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(guest.id))
        .filter_map(|v| {
            let network = v.network?;
            let name = &source_inventory
                .networks
                .iter()
                .find(|n| xenadmin_core::NetworkId::from(n.uuid) == network)?
                .name;
            let destination_network = destination_inventory
                .networks
                .iter()
                .find(|n| &n.name == name)?;
            Some(NicPlacement {
                vif: v.id,
                expected_network: network,
                destination_network: destination_network.uuid.into(),
            })
        })
        .collect::<Vec<_>>();
    assert!(
        !disks.is_empty() && !networks.is_empty(),
        "placement incomplete"
    );
    let transfer_network = networks[0].destination_network;
    let selection = CrossPoolVmSelection {
        source: source.latest().observe_vm(guest.id).unwrap(),
        destination: destination
            .latest()
            .observe_resource(ResourceTarget::Pool(destination_inventory.pool_id))
            .unwrap(),
        operation: CrossPoolVmOperation {
            vm: guest.id,
            destination_pool: destination_inventory.pool_id,
            destination_host,
            expected_host: None,
            expected_affinity: None,
            transfer_network,
            transfer_address: destination_inventory.hosts[&destination_host]
                .address
                .clone(),
            mode: CrossPoolMode::CopyHalted,
            disks: disks.clone(),
            networks: networks.clone(),
            host_transport_reviewed: true,
        },
    };
    let started = std::time::Instant::now();
    let copy = source
        .transfer_vm_to_pool(OperationId::new(), selection, &destination)
        .expect("cross-pool copy accepted");
    let outcome = wait_outcome(&copy, 1800).await;
    let copy_ms = started.elapsed().as_millis() as u64;
    let created = outcome.transfer_created.clone();
    let copied_vm = created.vms.first().copied();
    let visible = settle(120, || {
        copied_vm.is_some_and(|id| {
            destination
                .latest()
                .inventory()
                .is_some_and(|i| i.vms.contains_key(&id))
        })
    })
    .await;
    steps.push(json!({"step": "cross-pool copy", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": copy_ms, "created_vms": created.vms.len(), "created_disks": created.disks.len(), "visible_on_destination": visible}));
    let source_still_there = source
        .latest()
        .inventory()
        .unwrap()
        .vms
        .contains_key(&guest.id);

    // Boot the copy on the other pool and read its marker through the console.
    let mut marker = json!(null);
    if let (Some(copied), true) = (copied_vm, visible) {
        let start_offered = settle(60, || {
            destination.latest().inventory().is_some_and(|i| {
                i.vms
                    .get(&copied)
                    .is_some_and(|v| v.allowed_actions.contains(&PowerAction::Start))
            })
        })
        .await;
        let reports = report_listener(&format!("{gateway}:{port}")).await;
        let start = destination
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: destination.latest().observe_vm(copied).unwrap(),
                    action: PowerAction::Start,
                },
            )
            .expect("copy start accepted");
        let start_outcome = wait_vm_outcome(&start, 300).await;
        let running = settle(120, || {
            destination
                .latest()
                .inventory()
                .is_some_and(|i| i.vms[&copied].power_state == PowerState::Running)
        })
        .await;
        let mut reported = false;
        if running {
            let console = destination
                .open_console(destination.latest().observe_vm(copied).unwrap())
                .expect("copy console");
            guest_report_marker(&console, &gateway, port, &copy_ip, None).await;
            reported = wait_report(&reports, "A", 120).await;
            drop(console);
            let shutdown = destination
                .submit(
                    OperationId::new(),
                    VmCommand::Power {
                        vm: destination.latest().observe_vm(copied).unwrap(),
                        action: PowerAction::CleanShutdown,
                    },
                )
                .expect("copy shutdown accepted");
            let outcome = wait_vm_outcome(&shutdown, 300).await;
            steps.push(json!({"step": "copy clean shutdown", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
            settle(120, || {
                destination
                    .latest()
                    .inventory()
                    .is_some_and(|i| i.vms[&copied].power_state == PowerState::Halted)
            })
            .await;
        }
        marker = json!({"start_offered": start_offered, "start": format!("{:?}", start_outcome.state), "running": running, "marker_a_reported": reported, "received": reports.received.lock().unwrap().clone()});
        reports.task.abort();

        // Delete the copy with its disks on the destination pool.
        let copy_disks = disks_of(destination.latest().inventory().unwrap(), copied);
        let deletion = destination
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: destination.latest().observe_vm(copied).unwrap(),
                    operation: VmLifecycleOperation::Delete {
                        expected_template: false,
                        disks: copy_disks.clone(),
                        snapshots: Vec::new(),
                    },
                },
            )
            .expect("copy deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        let gone = settle(120, || {
            destination.latest().inventory().is_some_and(|i| {
                !i.vms.contains_key(&copied)
                    && copy_disks.iter().all(|d| !i.details.vdis.contains_key(d))
            })
        })
        .await;
        steps.push(json!({"step": "delete the copy with its disks", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "disks": copy_disks.len(), "gone": gone}));
    }

    // Start the lab guest again.
    let start = source
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: source.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    let running = settle(120, || {
        source
            .latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running)
    })
    .await;
    steps.push(json!({"step": "start the lab guest", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": running}));
    let both_ready = source.latest().is_current() && destination.latest().is_current();
    lab.record(
        "cross-pool-copy-and-verify",
        json!({
            "source_pool": source_inventory.pool_id.to_string(),
            "destination_pool": destination_inventory.pool_id.to_string(),
            "destination_host": lab.hosts[2].name,
            "disks": disks.len(),
            "nics": networks.len(),
            "steps": steps,
            "copy_marker": marker,
            "source_guest_retained": source_still_there,
            "both_pools_connected_throughout": both_ready,
        }),
    );
    assert!(source_still_there, "a copy must keep the source guest");
    assert!(
        copied_vm.is_some() && visible,
        "the copy did not appear on the destination pool"
    );
    assert!(running, "the lab guest did not start again");
    source.close().await;
    destination.close().await;
}

/// Builds a halted cross-pool move or copy of `vm` from `source` to the first host of
/// `destination`'s pool, mapping every disk to that host's local repository and every NIC
/// to the destination network with the same name.
fn cross_pool_selection(
    source: &Connection,
    destination: &Connection,
    vm: xenadmin_core::VmId,
    mode: xenadmin_core::CrossPoolMode,
) -> xenadmin_core::CrossPoolVmSelection {
    use xenadmin_core::{
        CrossPoolVmOperation, CrossPoolVmSelection, DiskPlacement, NicPlacement, ResourceTarget,
    };
    let source_inventory = source.latest().inventory().unwrap().clone();
    let destination_inventory = destination.latest().inventory().unwrap().clone();
    let destination_host = destination_inventory
        .coordinator
        .or_else(|| destination_inventory.hosts.keys().next().copied())
        .expect("destination host");
    let destination_sr =
        local_sr_of(&destination_inventory, destination_host).expect("destination repository");
    let disks = source_inventory.details.vm_disks[&vm]
        .iter()
        .filter_map(|vbd| source_inventory.details.vbds.get(vbd))
        .filter(|b| b.kind == "Disk")
        .filter_map(|b| {
            let vdi = b.vdi?;
            Some(DiskPlacement {
                vdi,
                expected_sr: source_inventory.details.vdis.get(&vdi)?.sr?,
                destination_sr,
            })
        })
        .collect::<Vec<_>>();
    let networks = source_inventory
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(vm))
        .filter_map(|v| {
            let network = v.network?;
            let name = &source_inventory
                .networks
                .iter()
                .find(|n| xenadmin_core::NetworkId::from(n.uuid) == network)?
                .name;
            let target = destination_inventory
                .networks
                .iter()
                .find(|n| &n.name == name)?;
            Some(NicPlacement {
                vif: v.id,
                expected_network: network,
                destination_network: target.uuid.into(),
            })
        })
        .collect::<Vec<_>>();
    assert!(
        !disks.is_empty() && !networks.is_empty(),
        "placement incomplete"
    );
    CrossPoolVmSelection {
        source: source.latest().observe_vm(vm).unwrap(),
        destination: destination
            .latest()
            .observe_resource(ResourceTarget::Pool(destination_inventory.pool_id))
            .unwrap(),
        operation: CrossPoolVmOperation {
            vm,
            destination_pool: destination_inventory.pool_id,
            destination_host,
            expected_host: None,
            expected_affinity: None,
            transfer_network: networks[0].destination_network,
            transfer_address: destination_inventory.hosts[&destination_host]
                .address
                .clone(),
            mode,
            disks,
            networks,
            host_transport_reviewed: true,
        },
    }
}

/// P11-04 moves: the halted lab guest is moved to the standalone pool and back, each move
/// leaving the source pool without it and the destination with a verified new identity; the
/// guest is started again on its own pool at the end.
#[tokio::test]
#[ignore]
async fn lab_cross_pool_move_round_trip() {
    use xenadmin_core::{CrossPoolMode, PowerAction, PowerState, VmCommand};
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 3, "three hosts are required");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let home = connect(&lab, &backend, 0, false).await;
    let away = connect(&lab, &backend, 2, false).await;
    let home_pool = home.latest().inventory().unwrap().pool_id;
    let away_pool = away.latest().inventory().unwrap().pool_id;
    assert_ne!(
        home_pool, away_pool,
        "the third host must be a separate pool"
    );
    // The guest may sit on either pool after an earlier interrupted move; the round trip
    // starts where it is and always ends on its home pool.
    let find_guest = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.guests()
                .find(|v| v.name.starts_with("lab-guest-"))
                .cloned()
        })
    };
    let (guest, starting_away) = match find_guest(&home) {
        Some(guest) => (guest, false),
        None => (find_guest(&away).expect("lab guest on either pool"), true),
    };
    let holder = if starting_away { &away } else { &home };
    let mut steps = Vec::new();
    if guest.power_state == PowerState::Running {
        let shutdown = holder
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: holder.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            holder
                .latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Halted)
        })
        .await;
        steps.push(json!({"step": "clean shutdown", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": halted}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }

    let mut current = guest.id;
    let mut legs = Vec::new();
    let mut plan: Vec<(&str, &Connection, &Connection)> = vec![
        ("to the standalone pool", &home, &away),
        ("back home", &away, &home),
    ];
    if starting_away {
        plan.insert(0, ("home from the standalone pool", &away, &home));
    }
    for (label, from, to) in plan {
        let selection = cross_pool_selection(from, to, current, CrossPoolMode::MoveHalted);
        let started = std::time::Instant::now();
        let operation = from
            .transfer_vm_to_pool(OperationId::new(), selection, to)
            .expect("cross-pool move accepted");
        let outcome = wait_outcome(&operation, 1800).await;
        let moved = outcome.transfer_created.vms.first().copied();
        let arrived = settle(120, || {
            moved.is_some_and(|id| {
                to.latest()
                    .inventory()
                    .is_some_and(|i| i.vms.get(&id).is_some_and(|v| v.name == guest.name))
            })
        })
        .await;
        let departed = settle(120, || {
            from.latest()
                .inventory()
                .is_some_and(|i| !i.vms.contains_key(&current))
        })
        .await;
        legs.push(json!({
            "leg": label, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64,
            "previous_id": current.to_string(), "new_id": moved.map(|m| m.to_string()), "arrived": arrived, "departed_from_source": departed,
            "disks": outcome.transfer_created.disks.len(),
        }));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{label}: {}",
            outcome.detail
        );
        assert!(
            arrived && departed,
            "{label}: the move was not reflected on both pools"
        );
        current = moved.unwrap();
    }

    let start = home
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: home.latest().observe_vm(current).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    let running = settle(120, || {
        home.latest()
            .inventory()
            .is_some_and(|i| i.vms[&current].power_state == PowerState::Running)
    })
    .await;
    steps.push(json!({"step": "start on the home pool", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": running}));
    lab.record(
        "cross-pool-move-round-trip",
        json!({"guest": guest.name, "home_pool": home_pool.to_string(), "away_pool": away_pool.to_string(), "steps": steps, "legs": legs, "final_id": current.to_string()}),
    );
    assert!(running, "the guest did not start again on its home pool");
    home.close().await;
    away.close().await;
}

/// P11-06: the halted guest is exported to an XVA file on the workstation, the file is
/// imported into the standalone pool as a new VM with its own disks, the import is deleted
/// with those disks, the file is removed and the guest is started again.
#[tokio::test]
#[ignore]
async fn lab_xva_export_and_import_across_pools() {
    use xenadmin_core::{
        ExportSource, PowerAction, PowerState, ResourceTarget, TransferAction, TransferFile,
        TransferRequest, VmCommand, VmLifecycleOperation, XvaCompression,
    };
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 3, "three hosts are required");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let home = connect(&lab, &backend, 0, false).await;
    let away = connect(&lab, &backend, 2, false).await;
    let home_inventory = home.latest().inventory().unwrap().clone();
    let away_inventory = away.latest().inventory().unwrap().clone();
    assert_ne!(
        home_inventory.pool_id, away_inventory.pool_id,
        "the third host must be a separate pool"
    );
    let guest = home_inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    if guest.power_state == PowerState::Running {
        let shutdown = home
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: home.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            home.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Halted)
        })
        .await;
        steps.push(json!({"step": "clean shutdown", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": halted}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }

    // Export.
    let directory = lab.evidence.parent().unwrap().join("transfers");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("lab-guest-{stamp}.xva"));
    let started = std::time::Instant::now();
    let export = home
        .transfer(
            OperationId::new(),
            home.latest()
                .observe_resource(ResourceTarget::Pool(home_inventory.pool_id))
                .unwrap(),
            TransferRequest {
                action: TransferAction::ExportXva {
                    source: ExportSource::Vm {
                        vm: guest.id,
                        expected_template: false,
                    },
                    compression: XvaCompression::Uncompressed,
                },
                file: TransferFile::destination(&path).expect("export destination"),
            },
        )
        .expect("export accepted");
    let outcome = wait_outcome(&export, 1800).await;
    let size = std::fs::metadata(&path).map(|m| m.len()).ok();
    steps.push(json!({"step": "export XVA", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "bytes": size}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(size.is_some_and(|s| s > 0), "export file missing");

    // Import into the other pool.
    let destination_host = away_inventory
        .coordinator
        .or_else(|| away_inventory.hosts.keys().next().copied())
        .expect("destination host");
    let sr = local_sr_of(&away_inventory, destination_host).expect("destination repository");
    let started = std::time::Instant::now();
    let import = away
        .transfer(
            OperationId::new(),
            away.latest()
                .observe_resource(ResourceTarget::Pool(away_inventory.pool_id))
                .unwrap(),
            TransferRequest {
                action: TransferAction::ImportXva { sr },
                file: TransferFile::source(&path).expect("inspect archive"),
            },
        )
        .expect("import accepted");
    let outcome = wait_outcome(&import, 1800).await;
    let imported = outcome.transfer_created.vms.first().copied();
    let visible = settle(120, || {
        imported.is_some_and(|id| {
            away.latest()
                .inventory()
                .is_some_and(|i| i.vms.get(&id).is_some_and(|v| v.name == guest.name))
        })
    })
    .await;
    let imported_disks = imported.map(|id| disks_of(away.latest().inventory().unwrap(), id).len());
    steps.push(json!({"step": "import XVA", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "created_vms": outcome.transfer_created.vms.len(), "created_disks": outcome.transfer_created.disks.len(), "visible": visible, "disks": imported_disks}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(
        visible,
        "the imported VM did not appear on the destination pool"
    );
    let imported = imported.unwrap();
    assert_ne!(imported, guest.id, "the import must be a new VM");

    // Delete the import with its disks, remove the file, start the guest.
    let disks = disks_of(away.latest().inventory().unwrap(), imported);
    let deletion = away
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: away.latest().observe_vm(imported).unwrap(),
                operation: VmLifecycleOperation::Delete {
                    expected_template: false,
                    disks: disks.clone(),
                    snapshots: Vec::new(),
                },
            },
        )
        .expect("deletion accepted");
    let outcome = wait_vm_outcome(&deletion, 300).await;
    let gone = settle(120, || {
        away.latest().inventory().is_some_and(|i| {
            !i.vms.contains_key(&imported) && disks.iter().all(|d| !i.details.vdis.contains_key(d))
        })
    })
    .await;
    steps.push(json!({"step": "delete the import with its disks", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "disks": disks.len(), "gone": gone}));
    let removed = std::fs::remove_file(&path).is_ok();
    let start = home
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: home.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    let running = settle(120, || {
        home.latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running)
    })
    .await;
    steps.push(json!({"step": "start", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": running}));
    lab.record(
        "xva-export-and-import-across-pools",
        json!({"guest": guest.name, "file_removed": removed, "steps": steps}),
    );
    assert!(gone && running);
    home.close().await;
    away.close().await;
}

/// P12-03 and P13-03: the pool's name and description are edited and restored, and a
/// member is disabled and enabled again, each change read back from the inventory.
#[tokio::test]
#[ignore]
async fn lab_pool_metadata_and_host_enable() {
    use xenadmin_core::{ResourceCommand, ResourceTarget};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    // The reviewed name is the pool's own label, empty on a fresh XCP-ng installation,
    // not the inventory's display name.
    let original_name = inventory
        .details
        .pool
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let original_description = inventory
        .details
        .pool
        .as_ref()
        .map(|p| p.description.clone())
        .unwrap_or_default();

    async fn rename(
        c: &Connection,
        pool: xenadmin_core::PoolId,
        from: (&str, &str),
        to: (&str, &str),
        seconds: u64,
    ) -> (OperationState, String, bool) {
        let operation = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::RenameMetadata {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Pool(pool))
                        .unwrap(),
                    expected_name: from.0.into(),
                    expected_description: from.1.into(),
                    name: to.0.into(),
                    description: to.1.into(),
                },
            )
            .expect("rename accepted");
        let outcome = wait_outcome(&operation, seconds).await;
        let check = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.pool_name == to.0
                    && i.details
                        .pool
                        .as_ref()
                        .is_some_and(|p| p.description == to.1)
            })
        })
        .await;
        (outcome.state.clone(), outcome.detail.clone(), check)
    }
    // The client never sets an empty label, so a pool XCP-ng left unnamed keeps a lasting
    // name afterwards; a named pool is renamed and then restored.
    let lasting = if original_name.is_empty() {
        "lab-pool".to_string()
    } else {
        original_name.clone()
    };
    let temporary = (
        format!("lab-pool-{stamp}"),
        format!("Renamed by the live harness at {stamp}"),
    );
    let (state, detail, check) = rename(
        &c,
        inventory.pool_id,
        (&original_name, &original_description),
        (&temporary.0, &temporary.1),
        120,
    )
    .await;
    steps.push(json!({"step": "rename pool", "from": original_name, "state": format!("{state:?}"), "detail": detail, "verified": check}));
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    assert!(check);
    let (state, detail, check) = rename(
        &c,
        inventory.pool_id,
        (&temporary.0, &temporary.1),
        (&lasting, &original_description),
        120,
    )
    .await;
    steps.push(json!({"step": if original_name.is_empty() { "give the unnamed pool a lasting name" } else { "restore pool name" }, "to": lasting, "state": format!("{state:?}"), "detail": detail, "verified": check}));
    assert_eq!(state, OperationState::Succeeded, "{detail}");
    assert!(check);

    // Disable and enable a member that does not host the guest.
    let coordinator = inventory.coordinator.expect("coordinator");
    let member = *inventory
        .hosts
        .keys()
        .find(|h| **h != coordinator)
        .expect("a member");
    for enabled in [false, true] {
        let operation = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::SetHostEnabled {
                    host: c
                        .latest()
                        .observe_resource(ResourceTarget::Host(member))
                        .unwrap(),
                    expected_enabled: !enabled,
                    enabled,
                },
            )
            .expect("host enable change accepted");
        let outcome = wait_outcome(&operation, 120).await;
        let check = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.hosts[&member].enabled == enabled)
        })
        .await;
        steps.push(json!({"step": if enabled { "enable member" } else { "disable member" }, "host": inventory.hosts[&member].name, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(check);
    }
    lab.record("pool-metadata-and-host-enable", json!({"steps": steps}));
    c.close().await;
}

/// P12-03, P12-05 and P12-06: the empty second member is put into maintenance, its
/// toolstack restarted, the host rebooted through the reviewed operation and maintenance
/// exited once it is back, each step read back from the inventory.
#[tokio::test]
#[ignore]
async fn lab_member_toolstack_restart_and_reboot() {
    use xenadmin_core::{HostOperation, ResourceCommand, ResourceTarget};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory.coordinator.expect("coordinator");
    let member = *inventory
        .hosts
        .keys()
        .find(|h| **h != coordinator)
        .expect("a member");
    let member_name = inventory.hosts[&member].name.clone();
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must not host guests"
    );
    let mut steps = Vec::new();
    async fn host_op(
        c: &Connection,
        host: xenadmin_core::HostId,
        operation: HostOperation,
        seconds: u64,
    ) -> std::sync::Arc<xenadmin_core::ResourceOperationRecord> {
        let op = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Host {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Host(host))
                        .unwrap(),
                    operation,
                },
            )
            .expect("host operation accepted");
        wait_outcome(&op, seconds).await
    }
    let details = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&member).cloned())
    };

    // A toolstack restart re-enables a host that still carries the client's maintenance
    // marker (an earlier interrupted run can leave that state); exit maintenance first then.
    if inventory.hosts[&member].enabled
        && details(&c).and_then(|h| h.maintenance_mode) == Some(true)
    {
        let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
        steps.push(json!({"step": "clear a stale maintenance marker", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }

    // Maintenance.
    let outcome = host_op(
        &c,
        member,
        HostOperation::EnterMaintenance {
            expected_enabled: inventory.hosts[&member].enabled,
            resident_vms: Vec::new(),
        },
        300,
    )
    .await;
    let disabled = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| !i.hosts[&member].enabled)
    })
    .await;
    steps.push(json!({"step": "enter maintenance", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": disabled}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );

    // Toolstack restart.
    let agent_before = details(&c)
        .and_then(|h| h.agent_start_time)
        .expect("agent start time");
    let started = std::time::Instant::now();
    let outcome = host_op(
        &c,
        member,
        HostOperation::RestartToolstack {
            expected_agent_start_time: agent_before.clone(),
        },
        600,
    )
    .await;
    let restarted = settle(300, || {
        details(&c)
            .and_then(|h| h.agent_start_time)
            .is_some_and(|t| t != agent_before)
    })
    .await;
    steps.push(json!({"step": "restart toolstack", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "agent_start_time_changed": restarted}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );

    // The toolstack restart re-enables the host while the client's maintenance marker
    // stays (finding 12); the operator's way back is exiting maintenance, which clears the
    // marker, and entering it again before the reviewed reboot.
    if c.latest()
        .inventory()
        .is_some_and(|i| i.hosts[&member].enabled)
    {
        let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
        steps.push(json!({"step": "clear the marker the restart left", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        let outcome = host_op(
            &c,
            member,
            HostOperation::EnterMaintenance {
                expected_enabled: true,
                resident_vms: Vec::new(),
            },
            300,
        )
        .await;
        let disabled = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| !i.hosts[&member].enabled)
        })
        .await;
        steps.push(json!({"step": "enter maintenance again", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": disabled}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }

    // Reboot.
    let boot_before = details(&c).and_then(|h| h.boot_time).expect("boot time");
    let started = std::time::Instant::now();
    let outcome = host_op(
        &c,
        member,
        HostOperation::Reboot {
            expected_boot_time: boot_before.clone(),
        },
        900,
    )
    .await;
    let rebooted = settle(900, || {
        details(&c)
            .and_then(|h| h.boot_time)
            .is_some_and(|t| t != boot_before)
    })
    .await;
    steps.push(json!({"step": "reboot", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "boot_time_changed": rebooted}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(
        rebooted,
        "the member did not come back with a new boot time"
    );

    // Exit maintenance.
    let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
    let enabled = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.hosts[&member].enabled)
    })
    .await;
    steps.push(json!({"step": "exit maintenance", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": enabled}));
    lab.record(
        "member-toolstack-restart-and-reboot",
        json!({"member": member_name, "steps": steps}),
    );
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(enabled);
    c.close().await;
}

/// P05-05, P05-08 and P07-10: a memory checkpoint of the running, tool-less guest is
/// created and deleted through the reviewed deletion, then a CD drive is added to the
/// halted guest, the host's guest tools image inserted through the reviewed tools-media
/// workflow and ejected again.
#[tokio::test]
#[ignore]
async fn lab_checkpoint_and_cd_drive_tools_media() {
    use xenadmin_core::{
        DeviceCommand, DeviceOperation, PowerAction, PowerState, SnapshotName, VmCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    async fn device(
        c: &Connection,
        vm: xenadmin_core::VmId,
        operation: DeviceOperation,
        seconds: u64,
    ) -> std::sync::Arc<xenadmin_core::OperationRecord> {
        let op = c
            .submit(
                OperationId::new(),
                VmCommand::Device(DeviceCommand {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    operation,
                }),
            )
            .expect("device command accepted");
        wait_vm_outcome(&op, seconds).await
    }
    let snapshot_disks = |c: &Connection, snapshot| {
        c.latest()
            .inventory()
            .map(|i| {
                i.details
                    .vbds
                    .values()
                    .filter(|b| b.snapshot == Some(snapshot) && b.kind == "Disk")
                    .filter_map(|b| b.vdi)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    // Leftover checkpoints of an interrupted run go first, through the same reviewed deletion.
    for leftover in inventory
        .snapshots
        .for_vm(guest.id)
        .filter(|s| s.name.starts_with("lab-checkpoint-"))
        .map(|s| s.id)
        .collect::<Vec<_>>()
    {
        let disks = snapshot_disks(&c, leftover);
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(guest.id, leftover).unwrap(),
                    disks,
                },
            )
            .expect("leftover deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        steps.push(json!({"step": "delete a leftover checkpoint", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    }

    // Memory checkpoint of the running guest: XCP-ng 8.3.0 takes one without guest tools.
    let name = format!("lab-checkpoint-{stamp}");
    let checkpoint = c
        .submit(
            OperationId::new(),
            VmCommand::CreateCheckpoint {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                name: SnapshotName::new(&name).unwrap(),
            },
        )
        .expect("checkpoint accepted");
    let outcome = wait_vm_outcome(&checkpoint, 600).await;
    let created = tokio::time::timeout(deadline(120), async {
        loop {
            if let Some(s) = c.latest().inventory().and_then(|i| {
                i.snapshots
                    .for_vm(guest.id)
                    .find(|s| s.name == name)
                    .cloned()
            }) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    steps.push(json!({"step": "memory checkpoint of the running guest", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "visible": created.is_ok(), "power_state": created.as_ref().ok().map(|s| format!("{:?}", s.power_state))}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let created = created.expect("checkpoint visible");
    let still_running = c
        .latest()
        .inventory()
        .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running);
    let disks = snapshot_disks(&c, created.id);
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(guest.id, created.id).unwrap(),
                disks: disks.clone(),
            },
        )
        .expect("checkpoint deletion accepted");
    let outcome = wait_vm_outcome(&deletion, 300).await;
    let gone = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            i.snapshots.get(&created.id).is_none()
                && disks.iter().all(|d| !i.details.vdis.contains_key(d))
        })
    })
    .await;
    steps.push(json!({"step": "delete the checkpoint with its disks", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "disks": disks.len(), "gone": gone, "guest_kept_running": still_running}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(gone);

    // Halt for the CD drive work.
    if guest.power_state == PowerState::Running {
        let shutdown = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            )
            .expect("shutdown accepted");
        let outcome = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Halted)
        })
        .await;
        steps.push(json!({"step": "clean shutdown", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": halted}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }
    let cd_drive = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vbds
                .values()
                .find(|b| b.vm == Some(guest.id) && b.kind == "CD")
                .map(|b| (b.id, b.empty, b.vdi))
        })
    };
    if cd_drive(&c).is_none() {
        let outcome = device(
            &c,
            guest.id,
            DeviceOperation::AddCdDrive { device: "3".into() },
            120,
        )
        .await;
        let check = settle(60, || cd_drive(&c).is_some()).await;
        steps.push(json!({"step": "add CD drive", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(check);
    } else {
        steps.push(json!({"step": "add CD drive", "skipped": "the guest already has one"}));
    }
    let (vbd, _, previous) = cd_drive(&c).expect("CD drive");

    // Insert the host's guest tools image, then eject it.
    let inventory = c.latest().inventory().unwrap().clone();
    let (image, source) = inventory
        .details
        .tools_media
        .iter()
        .next()
        .map(|(id, s)| (*id, s.label()))
        .expect("guest tools media advertised by the host");
    let sr = inventory
        .details
        .vdis
        .get(&image)
        .and_then(|v| v.sr)
        .expect("tools image repository");
    let outcome = device(
        &c,
        guest.id,
        DeviceOperation::MountGuestTools {
            vbd,
            image,
            sr,
            previous,
        },
        120,
    )
    .await;
    let inserted = settle(60, || {
        cd_drive(&c).is_some_and(|(_, empty, vdi)| empty == Some(false) && vdi == Some(image))
    })
    .await;
    steps.push(json!({"step": "insert guest tools image", "source": source, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": inserted}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(inserted);
    let outcome = device(&c, guest.id, DeviceOperation::EjectCd { vbd }, 120).await;
    let ejected = settle(60, || {
        cd_drive(&c).is_some_and(|(_, empty, _)| empty == Some(true))
    })
    .await;
    steps.push(json!({"step": "eject", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": ejected}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(ejected);

    // Start the guest again.
    let start = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
            },
        )
        .expect("start accepted");
    let outcome = wait_vm_outcome(&start, 300).await;
    let running = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].power_state == PowerState::Running)
    })
    .await;
    steps.push(json!({"step": "start", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": running}));
    lab.record(
        "checkpoint-and-cd-drive-tools-media",
        json!({"guest": guest.name, "steps": steps}),
    );
    assert!(running);
    c.close().await;
}
