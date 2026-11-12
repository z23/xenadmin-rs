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

/// Optional lab appliances recorded in the manifest by `lab/manage.py storage-network`,
/// `nas-image` and `guest-image`.
struct LabStorage {
    subnet: String,
    /// Host name to the MACs of its virtio NICs on the isolated storage network.
    host_nics: std::collections::BTreeMap<String, Vec<String>>,
    /// The appliance's address on the storage network (falls back to its lab address).
    nas_ip: String,
    /// The appliance's address on the lab network (a second iSCSI portal).
    nas_lab_ip: String,
    /// iSCSI target names and port offered by the appliance, when built with them.
    iscsi: Option<LabIscsi>,
    /// ISO images the appliance seeds into its library export.
    seed_isos: Vec<String>,
}

struct LabIscsi {
    open_iqn: String,
    chap_iqn: String,
    port: u16,
    /// Lab-only CHAP user and secret from `XENADMIN_LAB_CHAP_FILE`, never recorded.
    chap: Option<(String, String)>,
}

/// The Samba Active Directory controller recorded in the manifest by `lab/manage.py dc-image`.
struct LabDirectory {
    ip: String,
    realm: String,
    netbios: String,
    user: String,
    group: String,
    /// Administrator and test-user passwords from `XENADMIN_LAB_DC_FILE`, never recorded.
    passwords: Option<(String, String)>,
}

struct Lab {
    id: String,
    hosts: Vec<LabHost>,
    password: String,
    evidence: PathBuf,
    storage: Option<LabStorage>,
    guest_tools_image: Option<PathBuf>,
    directory: Option<LabDirectory>,
}

impl Lab {
    /// An endpoint in the lab subnet that nothing answers on (host .250), for dead-endpoint
    /// scenarios; derived from the first host's address so no lab address is hard-coded.
    fn unused_endpoint(&self) -> Endpoint {
        let ip = &self.hosts[0].ip;
        let prefix = ip.rsplit_once('.').expect("dotted address").0;
        Endpoint::parse(&format!("https://{prefix}.250/")).expect("unused endpoint")
    }
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
        let storage = document["storage"].as_object().map(|storage| LabStorage {
            subnet: storage["subnet"]
                .as_str()
                .expect("storage subnet")
                .to_string(),
            host_nics: storage["host_nics"]
                .as_object()
                .map(|hosts| {
                    hosts
                        .iter()
                        .map(|(host, macs)| {
                            (
                                host.clone(),
                                macs.as_array()
                                    .expect("nic macs")
                                    .iter()
                                    .filter_map(|m| m.as_str().map(str::to_lowercase))
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            nas_ip: document["nas"]["storage_ip"]
                .as_str()
                .or(document["nas"]["ip"].as_str())
                .expect("appliance address")
                .to_string(),
            nas_lab_ip: document["nas"]["ip"]
                .as_str()
                .expect("appliance lab address")
                .to_string(),
            seed_isos: document["nas"]["seed_isos"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            iscsi: document["nas"]["iscsi"].as_object().map(|i| LabIscsi {
                open_iqn: i["open_iqn"].as_str().expect("open target").to_string(),
                chap_iqn: i["chap_iqn"].as_str().expect("chap target").to_string(),
                port: i["port"].as_u64().unwrap_or(3260) as u16,
                chap: std::env::var("XENADMIN_LAB_CHAP_FILE")
                    .ok()
                    .and_then(|path| {
                        let text = std::fs::read_to_string(path).ok()?;
                        let mut words = text.split_whitespace();
                        Some((words.next()?.to_string(), words.next()?.to_string()))
                    }),
            }),
        });
        let guest_tools_image = document["guest_tools_image"]["path"]
            .as_str()
            .map(PathBuf::from);
        let directory = document["dc"].as_object().map(|d| LabDirectory {
            ip: d["ip"].as_str().expect("controller address").to_string(),
            realm: d["realm"].as_str().expect("realm").to_string(),
            netbios: d["netbios"].as_str().expect("netbios domain").to_string(),
            user: d["user"].as_str().expect("test user").to_string(),
            group: d["group"].as_str().expect("test group").to_string(),
            passwords: std::env::var("XENADMIN_LAB_DC_FILE").ok().and_then(|path| {
                let text = std::fs::read_to_string(path).ok()?;
                let mut words = text.split_whitespace();
                Some((words.next()?.to_string(), words.next()?.to_string()))
            }),
        });
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
            storage,
            guest_tools_image,
            directory,
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

/// The manifest index the default (first-host) endpoint resolves to: `XENADMIN_LAB_ENDPOINT_INDEX`
/// redirects it, for example to the surviving coordinator while the first host is reinstalled.
fn default_endpoint_index(lab: &Lab) -> usize {
    std::env::var("XENADMIN_LAB_ENDPOINT_INDEX")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| *i < lab.hosts.len())
        .unwrap_or(0)
}

async fn connect(lab: &Lab, backend: &Backend, index: usize, read_only: bool) -> Connection {
    let index = if index == 0 {
        default_endpoint_index(lab)
    } else {
        index
    };
    let connection = connect_at(lab, backend, index, read_only).await;
    if !read_only {
        guest_settled(&connection).await;
    }
    connection
}

/// A scenario often begins seconds after another one started the lab guest. Until the
/// guest's kernel has announced itself, its host offers no clean shutdown or reboot, and
/// the client rightly refuses an action the host does not offer. Scenarios are about what
/// they name, not about that window, so a running lab guest is given time to be offered a
/// clean shutdown before a scenario that may change things starts. A guest that is never
/// offered one starts the scenario all the same.
async fn guest_settled(c: &Connection) {
    use xenadmin_core::{PowerAction, PowerState};
    settle(120, || {
        c.latest().inventory().is_none_or(|i| {
            i.guests()
                .filter(|v| {
                    v.name.starts_with("lab-guest-")
                        && !v.is_template
                        && v.power_state == PowerState::Running
                })
                .all(|v| v.allowed_actions.contains(&PowerAction::CleanShutdown))
        })
    })
    .await;
    let guests: Vec<_> = c
        .latest()
        .inventory()
        .map(|i| {
            i.guests()
                .filter(|v| v.name.starts_with("lab-guest-") && !v.is_template)
                .map(|v| v.id)
                .collect()
        })
        .unwrap_or_default();
    for vm in guests {
        guest_booted(c, vm).await;
    }
}

/// The offer of a clean shutdown comes from the guest's kernel. A request sent before the
/// rest of the guest is up is lost, and the host then holds its task open for many
/// minutes, so a running guest whose host says it started within the last 75 seconds is
/// given the rest of them. The lab's hosts take their clocks from this workstation; were
/// they far apart, no start would look recent and nothing would be waited for.
async fn guest_booted(c: &Connection, vm: xenadmin_core::VmId) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let age = c.latest().inventory().and_then(|i| {
        let guest = i.vms.get(&vm)?;
        if guest.power_state != xenadmin_core::PowerState::Running {
            return None;
        }
        let metrics = i.details.vms.get(&vm)?.metrics?;
        let started = host_time(i.details.vm_metrics.get(&metrics)?.start_time.as_deref()?)?;
        Some(now - started)
    });
    if let Some(age) = age.filter(|age| (0..75).contains(age)) {
        tokio::time::sleep(Duration::from_secs((75 - age) as u64)).await;
    }
}

/// Seconds since the epoch for a time as XAPI writes it, `YYYYMMDDTHH:MM:SSZ`.
fn host_time(text: &str) -> Option<i64> {
    let digits = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    if text.len() != 18 || !text.is_ascii() || &text[8..9] != "T" || !text.ends_with('Z') {
        return None;
    }
    let (year, month, day) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    let (hour, minute, second) = (digits(9..11)?, digits(12..14)?, digits(15..17)?);
    // Days from the civil date, by the usual era arithmetic.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[test]
fn a_host_time_reads_as_seconds_since_the_epoch() {
    assert_eq!(host_time("19700101T00:00:00Z"), Some(0));
    assert_eq!(host_time("20000229T12:00:01Z"), Some(951_825_601));
    assert_eq!(host_time("20261111T10:20:30Z"), Some(1_794_392_430));
    assert_eq!(host_time("2026-11-11T10:20:30Z"), None);
    assert_eq!(host_time("20261111T10:20:30"), None);
}

/// Connects to exactly the manifest host at `index`, ignoring the default-endpoint override.
async fn connect_at(lab: &Lab, backend: &Backend, index: usize, read_only: bool) -> Connection {
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
    let destination_index = default_endpoint_index(&lab);
    let destination = connect(&lab, &backend, 0, false).await;
    let before = inventory_summary(&destination);
    let source_index = std::env::var("XENADMIN_LAB_JOIN_SOURCE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|i| *i < lab.hosts.len() && *i != destination_index)
        .unwrap_or(if destination_index == 0 { 1 } else { 0 });
    let source = connect_at(&lab, &backend, source_index, false).await;
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
        destination_address: lab.hosts[destination_index].ip.clone(),
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

/// Record a step and echo it, so a scenario that panics later still leaves its trail in the log.
fn note(steps: &mut Vec<Value>, entry: Value) {
    eprintln!("step: {entry}");
    steps.push(entry);
}

/// Wait for a VM operation's outcome while sampling the progress it reports (P14-01).
async fn wait_vm_outcome_sampled(
    operation: &xenadmin_core::Operation,
    seconds: u64,
) -> (std::sync::Arc<xenadmin_core::OperationRecord>, Vec<f64>) {
    let mut samples = Vec::new();
    let row = tokio::time::timeout(deadline(seconds), async {
        loop {
            let row = operation.latest();
            if let Some(progress) = row.progress
                && samples.last() != Some(&progress)
            {
                samples.push(progress);
            }
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("VM operation reached an outcome");
    (row, samples)
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
        // A guest that is still booting loses a clean shutdown or reboot, and the host then
        // holds the task open for minutes (finding 59): give a just-started guest its time.
        // The host also advertises an action a few seconds after the state that allows it,
        // and an action taken before that would be recorded here as not offered.
        if matches!(action, PowerAction::CleanShutdown | PowerAction::Reboot) {
            guest_booted(&c, guest.id).await;
        }
        settle(30, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest.id].allowed_actions.contains(&action))
        })
        .await;
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

/// A live storage migration of every disk of `vm` onto `destination`'s local repository,
/// keeping each NIC on its network and transferring over the destination's management
/// network (storage relocation needs an explicit IP-configured transfer network).
fn migration_plan(
    inventory: &xenadmin_core::Inventory,
    vm: xenadmin_core::VmId,
    destination: HostId,
) -> xenadmin_core::VmMigration {
    use xenadmin_core::{DiskPlacement, NicPlacement, VmMigration};
    let destination_sr = local_sr_of(inventory, destination).expect("destination local repository");
    let disks = inventory.details.vm_disks[&vm]
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
        .filter(|v| v.vm == Some(vm))
        .filter_map(|v| {
            let network = v.network?;
            Some(NicPlacement {
                vif: v.id,
                expected_network: network,
                destination_network: network,
            })
        })
        .collect::<Vec<_>>();
    let transfer_network = inventory
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(destination) && p.management == Some(true))
        .and_then(|p| p.network)
        .expect("destination management network");
    VmMigration {
        destination,
        expected_host: inventory.vms[&vm].host,
        expected_affinity: None,
        live: true,
        disks,
        networks,
        transfer_network: Some(transfer_network),
    }
}

/// P11-01, P11-03, P11-05: live-migrate the running lab guest to another pool member with
/// both disks relocated to that member's local repository, then open its console there.
#[tokio::test]
#[ignore]
async fn lab_guest_live_migration_with_storage() {
    use xenadmin_core::{PowerState, SrId, VmCommand};
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
    let plan = migration_plan(&inventory, guest.id, destination);
    let destination_sr = plan.disks[0].destination_sr;
    let disks = plan.disks.clone();
    assert_eq!(disks.len(), 2, "expected the image and data disks");
    let migration = c
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: plan.clone(),
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
    let mut refresh_failures = Vec::new();
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
        // The first answer is one request; every later one is a refresh from the last
        // sample, which is where the host's reported start runs a step ahead of its rows.
        let first_end = with_samples.range_end;
        let mut refreshes = Vec::new();
        let mut errors = Vec::new();
        let followed = tokio::time::timeout(deadline(60), async {
            loop {
                let snapshot = metrics.latest();
                if let Some(error) = &snapshot.error
                    && !errors.contains(error)
                {
                    errors.push(error.clone());
                }
                if snapshot.range_end > first_end && refreshes.last() != Some(&snapshot.range_end) {
                    refreshes.push(snapshot.range_end);
                }
                if refreshes.len() >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .is_ok();
        results.last_mut().unwrap()["refresh"] = json!({"first_range_end": first_end,
            "later_range_ends": refreshes, "errors": errors, "three_refreshes_seen": followed});
        metrics.close();
        refresh_failures.extend(
            errors
                .into_iter()
                .map(|error| format!("{target:?}: {error}")),
        );
        if !followed {
            refresh_failures.push(format!("{target:?}: fewer than three refreshes in 60 s"));
        }
    }
    c.close().await;
    lab.record("metrics-guest-and-host", json!({"targets": results}));
    assert!(
        refresh_failures.is_empty(),
        "metrics refresh: {refresh_failures:?}"
    );
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

/// Power and console row, failure column: a clean shutdown and a start, each interrupted by
/// the application closing as soon as the request was admitted, are recovered from the
/// journal on restart, never sent again, and the guest ends in the state each asked for.
#[tokio::test]
#[ignore]
async fn lab_interrupted_shutdown_and_start_resolve_on_restart() {
    use xenadmin_core::{OperationControl, PowerAction, PowerState, VmCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let mut legs = Vec::new();
    let mut failures = Vec::new();
    for (action, expected) in [
        (PowerAction::CleanShutdown, PowerState::Halted),
        (PowerAction::Start, PowerState::Running),
    ] {
        let id = OperationId::new();
        let (guest_id, states, state_at_close) = {
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
            // Allowed operations lag a state change by a few seconds (observation 15).
            let advertised = settle(90, || {
                c.latest().inventory().is_some_and(|i| {
                    i.vms
                        .get(&guest.id)
                        .is_some_and(|v| v.allowed_actions.contains(&action))
                })
            })
            .await;
            assert!(advertised, "{action:?} is not advertised for the lab guest");
            let operation = c
                .submit(
                    id,
                    VmCommand::Power {
                        vm: c.latest().observe_vm(guest.id).unwrap(),
                        action,
                    },
                )
                .expect("power action accepted");
            // Only an admitted record can be recovered: one still waiting for admission, or
            // refused before it, exists on its handle alone. Stop the moment it is on its way.
            let mut states = Vec::new();
            let admitted = tokio::time::timeout(deadline(30), async {
                loop {
                    let latest = operation.latest();
                    let entry = format!("{:?}", latest.state);
                    if states.last() != Some(&entry) {
                        states.push(entry);
                    }
                    if matches!(
                        latest.state,
                        OperationState::Dispatching | OperationState::Tracking
                    ) {
                        return true;
                    }
                    if latest.state.terminal() {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap_or(false);
            let state_at_close = format!("{:?}", operation.latest().state);
            c.close().await;
            drop(backend);
            if !admitted {
                failures.push(format!(
                    "{action:?} finished before it could be interrupted: {states:?}"
                ));
            }
            (guest.id, states, state_at_close)
        };

        // Same state directory, new process: the journal must reconcile, not replay.
        let backend = Backend::with_state_directory(state.path().into()).unwrap();
        let c = connect(&lab, &backend, 0, false).await;
        let (record, timeline) = wait_history_outcome(&c, id, 300).await;
        let mut leg = json!({"action": format!("{action:?}"), "states_before_close": states,
            "state_at_close": state_at_close, "recovered_timeline": timeline});
        let Some(record) = record else {
            failures.push(format!("{action:?}: no record after the restart"));
            leg["recovered"] = json!(null);
            legs.push(leg);
            c.close().await;
            continue;
        };
        leg["recovered"] = json!({"state": format!("{:?}", record.state), "detail": record.detail});
        let record = if record.state == OperationState::OutcomeUnknown {
            let before = (format!("{:?}", record.state), record.detail.clone());
            c.control_operation(id, OperationControl::Recheck)
                .expect("recheck accepted");
            let moved = settle(180, || {
                c.operation_history()
                    .iter()
                    .find(|r| r.id == id)
                    .is_some_and(|r| (format!("{:?}", r.state), r.detail.clone()) != before)
            })
            .await;
            let (after, more) = wait_history_outcome(&c, id, 300).await;
            let after = after.expect("record after recheck");
            leg["recheck"] = json!({"moved": moved, "timeline": more,
                "state": format!("{:?}", after.state), "detail": after.detail});
            after
        } else {
            record
        };
        // Host truth: the guest reaches the state the interrupted request asked for.
        let reached = settle(240, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms
                    .get(&guest_id)
                    .is_some_and(|v| v.power_state == expected)
            })
        })
        .await;
        leg["guest_reached_expected_state"] = json!(reached);
        leg["records_with_this_id"] =
            json!(c.operation_history().iter().filter(|r| r.id == id).count());
        if !reached {
            failures.push(format!("{action:?}: the guest did not reach {expected:?}"));
        }
        if record.state == OperationState::OutcomeUnknown {
            // Cut off between the dispatch and the saved task reference, the request leaves
            // nothing a recheck can follow. Staying unknown is then the honest outcome: the
            // guest's state alone does not say whose request changed it. The operator
            // acknowledges it, which also releases the guest for the next leg.
            c.control_operation(id, OperationControl::AcknowledgeUnknown)
                .expect("acknowledgement accepted");
            let acknowledged = settle(30, || {
                c.operation_history()
                    .iter()
                    .find(|r| r.id == id)
                    .is_some_and(|r| r.state == OperationState::AcknowledgedUnknown)
            })
            .await;
            leg["acknowledged_unknown"] = json!(acknowledged);
            if !record.detail.contains("without a saved task reference") || !acknowledged {
                failures.push(format!(
                    "{action:?}: unknown after the recheck for another reason: {}",
                    record.detail
                ));
            }
        } else if record.state != OperationState::Succeeded {
            failures.push(format!(
                "{action:?}: recovered as {:?}: {}",
                record.state, record.detail
            ));
        }
        legs.push(leg);
        c.close().await;
    }
    lab.record("interrupted-shutdown-and-start", json!({"legs": legs}));
    assert!(failures.is_empty(), "{failures:?}");
}

/// Same-pool migration, failure column: a storage live migration of the running lab guest
/// to the other pool member is cut off by the application closing as soon as the request
/// is on its way. On restart the journal reconciles it without sending it again: the
/// record is followed to its end, by itself or by a recheck, and the guest ends running on
/// one host with both disks on that host's repository. The guest is then migrated back.
#[tokio::test]
#[ignore]
async fn lab_interrupted_storage_migration_resolves_on_restart() {
    use xenadmin_core::{OperationControl, PowerAction, PowerState, VmCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let id = OperationId::new();
    let disks_on =
        |i: &xenadmin_core::Inventory, vm: xenadmin_core::VmId| -> Vec<xenadmin_core::SrId> {
            i.details
                .vm_disks
                .get(&vm)
                .map(|vbds| {
                    vbds.iter()
                        .filter_map(|b| i.details.vbds.get(b))
                        .filter(|b| b.kind == "Disk")
                        .filter_map(|b| i.details.vdis.get(&b.vdi?)?.sr)
                        .collect()
                })
                .unwrap_or_default()
        };
    let (guest_id, source_host, destination, source_sr, destination_sr, states, state_at_close) = {
        let backend = Backend::with_state_directory(state.path().into()).unwrap();
        let c = connect(&lab, &backend, 0, false).await;
        let guest = c
            .latest()
            .inventory()
            .unwrap()
            .guests()
            .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
            .expect("lab guest")
            .clone();
        if guest.power_state != PowerState::Running {
            power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await;
        }
        let advertised = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.details.vms.get(&guest.id).is_some_and(|v| {
                    v.allowed_operations
                        .as_ref()
                        .is_some_and(|ops| ops.iter().any(|op| op == "migrate_send"))
                })
            })
        })
        .await;
        assert!(
            advertised,
            "storage migration is not advertised for the lab guest"
        );
        let inventory = c.latest().inventory().unwrap().clone();
        let source_host = inventory.vms[&guest.id].host.expect("resident host");
        let destination = inventory
            .hosts
            .keys()
            .copied()
            .find(|h| *h != source_host && inventory.hosts[h].enabled)
            .expect("another enabled host");
        let plan = migration_plan(&inventory, guest.id, destination);
        let source_sr = plan.disks[0].expected_sr;
        let destination_sr = plan.disks[0].destination_sr;
        let operation = c
            .submit(
                id,
                VmCommand::Migrate {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    operation: plan,
                },
            )
            .expect("migration accepted");
        let mut states = Vec::new();
        let admitted = tokio::time::timeout(deadline(60), async {
            loop {
                let latest = operation.latest();
                let entry = format!("{:?}", latest.state);
                if states.last() != Some(&entry) {
                    states.push(entry);
                }
                if matches!(
                    latest.state,
                    OperationState::Dispatching | OperationState::Tracking
                ) {
                    return true;
                }
                if latest.state.terminal() {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or(false);
        let state_at_close = format!("{:?}", operation.latest().state);
        c.close().await;
        drop(backend);
        assert!(
            admitted,
            "the migration ended before it could be interrupted: {states:?}"
        );
        (
            guest.id,
            source_host,
            destination,
            source_sr,
            destination_sr,
            states,
            state_at_close,
        )
    };

    // Same state directory, new process: the journal must reconcile, not replay.
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let (record, timeline) = wait_history_outcome(&c, id, 900).await;
    let mut evidence = json!({"states_before_close": states, "state_at_close": state_at_close, "recovered_timeline": timeline});
    let record = record.expect("the migration's record after the restart");
    evidence["recovered"] =
        json!({"state": format!("{:?}", record.state), "detail": record.detail});
    // The host goes on with a migration whose client left. Let it end before judging.
    let settled = settle(900, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vms
                .get(&guest_id)
                .is_some_and(|v| v.current_operations.is_empty())
        })
    })
    .await;
    evidence["host_task_ended"] = json!(settled);
    // A recheck reads the saved task and continues nothing. While the host is still
    // finishing the migration it says so and asks to be made again later, which is what an
    // operator does: recheck until the record says something else, a few seconds apart.
    let mut record = record;
    let mut rechecks = Vec::new();
    while record.state == OperationState::OutcomeUnknown && rechecks.len() < 20 {
        let before = (format!("{:?}", record.state), record.detail.clone());
        c.control_operation(id, OperationControl::Recheck)
            .expect("recheck accepted");
        let moved = settle(60, || {
            c.operation_history()
                .iter()
                .find(|r| r.id == id)
                .is_some_and(|r| (format!("{:?}", r.state), r.detail.clone()) != before)
        })
        .await;
        let (after, _) = wait_history_outcome(&c, id, 300).await;
        record = after.expect("record after recheck");
        rechecks.push(json!({"moved": moved, "state": format!("{:?}", record.state), "detail": record.detail}));
        if record.state == OperationState::OutcomeUnknown {
            if !record.detail.contains("still active") {
                break;
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    evidence["rechecks"] = json!(rechecks);
    // Host truth: the guest runs on one host with both disks on that host's repository.
    let placed = settle(300, || {
        c.latest().inventory().is_some_and(|i| {
            let srs = disks_on(i, guest_id);
            let v = &i.vms[&guest_id];
            v.power_state == PowerState::Running
                && srs.len() == 2
                && ((v.host == Some(destination) && srs.iter().all(|s| *s == destination_sr))
                    || (v.host == Some(source_host) && srs.iter().all(|s| *s == source_sr)))
        })
    })
    .await;
    let now = c.latest().inventory().unwrap().clone();
    let moved_to_destination = now.vms[&guest_id].host == Some(destination);
    evidence["placement"] = json!({"consistent": placed, "on_destination": moved_to_destination,
        "power_state": format!("{:?}", now.vms[&guest_id].power_state)});
    evidence["final"] = json!({"state": format!("{:?}", record.state), "detail": record.detail});
    evidence["records_with_this_id"] =
        json!(c.operation_history().iter().filter(|r| r.id == id).count());
    // Baseline: the guest back on the host it came from, by an ordinary migration.
    let mut returned = json!(null);
    if placed && moved_to_destination && record.state.terminal() {
        let inventory = c.latest().inventory().unwrap().clone();
        settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.details.vms.get(&guest_id).is_some_and(|v| {
                    v.allowed_operations
                        .as_ref()
                        .is_some_and(|ops| ops.iter().any(|op| op == "migrate_send"))
                })
            })
        })
        .await;
        let back = c
            .submit(
                OperationId::new(),
                VmCommand::Migrate {
                    vm: c.latest().observe_vm(guest_id).unwrap(),
                    operation: migration_plan(&inventory, guest_id, source_host),
                },
            )
            .expect("return migration accepted");
        let row = wait_vm_outcome(&back, 1200).await;
        let home = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest_id].host == Some(source_host))
        })
        .await;
        returned = json!({"state": format!("{:?}", row.state), "detail": row.detail, "home": home});
    }
    evidence["returned"] = returned.clone();
    c.close().await;
    lab.record("interrupted-storage-migration", evidence.clone());
    assert!(settled, "the host's migration task did not end: {evidence}");
    assert!(
        placed,
        "the guest is not running on one host with its disks together: {evidence}"
    );
    assert_eq!(evidence["records_with_this_id"], 1, "{evidence}");
    // The migration ended on the host and the guest is where it was sent: once the task
    // has ended the record must say so, without the operator having to vouch for it.
    assert_eq!(record.state, OperationState::Succeeded, "{evidence}");
    if moved_to_destination {
        assert_eq!(returned["state"], "Succeeded", "{returned}");
        assert_eq!(returned["home"], true, "{returned}");
    }
}

/// Host operations, failure column: the reviewed reboot of the empty member, in
/// maintenance, is cut off by the application closing as soon as the request is on its way.
/// On restart the journal reconciles it without sending it again: the member comes back
/// with one newer boot time, the record is followed to its end, by itself or by a recheck,
/// and the boot time does not change a second time. The member then leaves maintenance.
#[tokio::test]
#[ignore]
async fn lab_interrupted_member_reboot_resolves_on_restart() {
    use xenadmin_core::{HostOperation, OperationControl, ResourceCommand, ResourceTarget};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let id = OperationId::new();
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
    let boot_time = |c: &Connection, host: xenadmin_core::HostId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&host).and_then(|h| h.boot_time.clone()))
    };
    let (member, boot_before, states, state_at_close, entered) = {
        let backend = Backend::with_state_directory(state.path().into()).unwrap();
        let c = connect(&lab, &backend, 0, false).await;
        let inventory = c.latest().inventory().unwrap().clone();
        let coordinator = inventory.coordinator.expect("coordinator");
        let member = *inventory
            .hosts
            .keys()
            .find(|h| **h != coordinator)
            .expect("a member");
        assert!(
            !inventory
                .vms
                .values()
                .any(|v| v.host == Some(member) && !v.is_control_domain),
            "the member must not host guests"
        );
        let mut entered = json!({"skipped": "the member was already disabled"});
        if inventory.hosts[&member].enabled {
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
            settle(120, || {
                c.latest()
                    .inventory()
                    .is_some_and(|i| !i.hosts[&member].enabled)
            })
            .await;
            entered = json!({"state": format!("{:?}", outcome.state), "detail": outcome.detail});
            assert_eq!(
                outcome.state,
                OperationState::Succeeded,
                "{}",
                outcome.detail
            );
        }
        let boot_before = boot_time(&c, member).expect("boot time");
        let operation = c
            .submit_resource(
                id,
                ResourceCommand::Host {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Host(member))
                        .unwrap(),
                    operation: HostOperation::Reboot {
                        expected_boot_time: boot_before.clone(),
                    },
                },
            )
            .expect("reboot accepted");
        let mut states = Vec::new();
        let admitted = tokio::time::timeout(deadline(60), async {
            loop {
                let latest = operation.latest();
                let entry = format!("{:?}", latest.state);
                if states.last() != Some(&entry) {
                    states.push(entry);
                }
                if matches!(
                    latest.state,
                    OperationState::Dispatching | OperationState::Tracking
                ) {
                    return true;
                }
                if latest.state.terminal() {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or(false);
        let state_at_close = format!("{:?}", operation.latest().state);
        c.close().await;
        drop(backend);
        assert!(
            admitted,
            "the reboot ended before it could be interrupted: {states:?}"
        );
        (member, boot_before, states, state_at_close, entered)
    };

    // Same state directory, new process: the journal must reconcile, not replay.
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let row = |c: &Connection| c.resource_history().iter().find(|r| r.id == id).cloned();
    let started = std::time::Instant::now();
    let mut timeline = Vec::new();
    let mut last = None;
    let _ = tokio::time::timeout(deadline(900), async {
        loop {
            let record = row(&c);
            let key = record.as_ref().map(|r| (format!("{:?}", r.state), r.detail.clone()));
            if key != last {
                if let Some((state, detail)) = &key {
                    timeline.push(json!({"at_ms": started.elapsed().as_millis() as u64, "state": state, "detail": detail}));
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
    let record = row(&c).expect("the reboot's record after the restart");
    let mut evidence = json!({"enter_maintenance": entered, "states_before_close": states, "state_at_close": state_at_close,
        "recovered_timeline": timeline, "recovered": {"state": format!("{:?}", record.state), "detail": record.detail}});
    // Host truth first: the member is back, live, with a newer boot time.
    let back = settle(900, || {
        c.latest().inventory().is_some_and(|i| {
            i.details.hosts.get(&member).is_some_and(|h| {
                h.metrics
                    .and_then(|m| i.details.host_metrics.get(&m))
                    .and_then(|m| m.live)
                    == Some(true)
                    && h.boot_time.as_ref().is_some_and(|t| *t != boot_before)
            })
        })
    })
    .await;
    let boot_after = boot_time(&c, member);
    evidence["member_back_with_a_newer_boot_time"] = json!(back);
    let record = if record.state == OperationState::OutcomeUnknown {
        let before = (format!("{:?}", record.state), record.detail.clone());
        c.control_resource(id, OperationControl::Recheck)
            .expect("recheck accepted");
        let moved = settle(300, || {
            row(&c).is_some_and(|r| (format!("{:?}", r.state), r.detail.clone()) != before)
        })
        .await;
        let settled = settle(600, || {
            row(&c).is_some_and(|r| r.state.terminal() || r.state == OperationState::OutcomeUnknown)
        })
        .await;
        let after = row(&c).expect("record after recheck");
        evidence["recheck"] = json!({"moved": moved, "settled": settled, "state": format!("{:?}", after.state), "detail": after.detail});
        after
    } else {
        record
    };
    // A second reboot would show as another boot time: watch for two minutes.
    let rebooted_again = settle(120, || boot_time(&c, member) != boot_after).await;
    evidence["boot_time_changed_again"] = json!(rebooted_again);
    evidence["final"] = json!({"state": format!("{:?}", record.state), "detail": record.detail});
    evidence["records_with_this_id"] =
        json!(c.resource_history().iter().filter(|r| r.id == id).count());
    // Baseline: out of maintenance.
    let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
    let enabled = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.hosts[&member].enabled)
    })
    .await;
    evidence["exit_maintenance"] = json!({"state": format!("{:?}", outcome.state), "detail": outcome.detail, "enabled": enabled});
    c.close().await;
    lab.record("interrupted-member-reboot", evidence.clone());
    assert!(
        back,
        "the member did not come back with a newer boot time: {evidence}"
    );
    assert!(
        !rebooted_again,
        "the member's boot time changed a second time: {evidence}"
    );
    assert_eq!(evidence["records_with_this_id"], 1, "{evidence}");
    assert!(
        matches!(
            record.state,
            OperationState::Succeeded | OperationState::OutcomeUnknown
        ),
        "{evidence}"
    );
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(enabled);
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
/// renamed, repositioned, grown while still attached, detached with its data kept, grown
/// again, copied to another member's local repository, the copy cleaned up, and the
/// scratch disk reattached, detached and deleted. The guest's own image and data disks are
/// never touched, and the guest is started again at the end.
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

    // Grow while the disk is still attached to the halted guest: an attachment that is
    // not plugged does not make the disk busy.
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
                    size: 384 * mib,
                },
            },
        )
        .expect("grow accepted");
    let outcome = wait_outcome(&grow, 300).await;
    let check = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details.vdis.get(&scratch).and_then(|v| v.virtual_size) == Some(384 * mib)
        })
    })
    .await;
    steps.push(json!({"step": "grow to 384 MiB while attached to the halted guest", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(check);

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
            // The guest's home host as the pool reports it: a same-pool migration sets one.
            expected_affinity: source_inventory
                .details
                .vms
                .get(&guest.id)
                .and_then(|d| d.affinity),
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
            expected_affinity: source_inventory
                .details
                .vms
                .get(&vm)
                .and_then(|d| d.affinity),
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

/// One halted guest's part of a batch of cross-pool moves, reviewed from what the two
/// pools report now: its home host as the source pool has it (finding 62), every disk
/// mapped by `repository` and every NIC by `network`, to `destination_host` over that
/// host's management address. An error says why the single cross-pool move would not take
/// the guest.
fn cross_pool_batch_item(
    from: &Connection,
    to: &Connection,
    vm: xenadmin_core::VmId,
    destination_host: HostId,
    repository: &dyn Fn(xenadmin_core::VbdId) -> Option<xenadmin_core::SrId>,
    network: &dyn Fn(xenadmin_core::VifId, &str) -> Option<xenadmin_core::NetworkId>,
) -> Result<xenadmin_core::CrossPoolVmOperation, String> {
    use xenadmin_core::{CrossPoolMode, CrossPoolVmOperation, DiskPlacement, NicPlacement};
    let source = from
        .latest()
        .inventory()
        .ok_or("no source inventory")?
        .clone();
    let destination = to
        .latest()
        .inventory()
        .ok_or("no destination inventory")?
        .clone();
    let mut disks = Vec::new();
    for block in source
        .details
        .vbds
        .values()
        .filter(|b| b.vm == Some(vm) && b.kind == "Disk")
    {
        let vdi = block.vdi.ok_or("a disk attachment names no disk")?;
        disks.push(DiskPlacement {
            vdi,
            expected_sr: source
                .details
                .vdis
                .get(&vdi)
                .and_then(|d| d.sr)
                .ok_or("a disk reports no repository")?,
            destination_sr: repository(block.id).ok_or("a disk has no destination repository")?,
        });
    }
    let mut networks = Vec::new();
    for nic in source.details.vifs.values().filter(|v| v.vm == Some(vm)) {
        let current = nic.network.ok_or("a NIC reports no network")?;
        let name = source
            .networks
            .iter()
            .find(|n| xenadmin_core::NetworkId::from(n.uuid) == current)
            .map(|n| n.name.clone())
            .ok_or("a NIC's network is not in the inventory")?;
        networks.push(NicPlacement {
            vif: nic.id,
            expected_network: current,
            destination_network: network(nic.id, &name)
                .ok_or("a NIC has no destination network")?,
        });
    }
    let address = destination
        .hosts
        .get(&destination_host)
        .ok_or("the destination host is not in the inventory")?
        .address
        .clone();
    let transfer_network = destination
        .details
        .pifs
        .values()
        .find(|p| {
            p.host == Some(destination_host)
                && p.currently_attached == Some(true)
                && p.ip == address
        })
        .and_then(|p| p.network)
        .ok_or("the destination host's address is on no attached interface")?;
    let operation = CrossPoolVmOperation {
        vm,
        destination_pool: destination.pool_id,
        destination_host,
        expected_host: None,
        expected_affinity: source.details.vms.get(&vm).and_then(|d| d.affinity),
        transfer_network,
        transfer_address: address,
        mode: CrossPoolMode::MoveHalted,
        disks,
        networks,
        host_transport_reviewed: true,
    };
    let availability = operation.availability(&source, &destination);
    if availability.is_available() {
        Ok(operation)
    } else {
        Err(availability.explanation())
    }
}

/// Where a guest's disks and NICs are, by the attachment and interface identities a move
/// keeps: the repository of each disk and the network of each NIC.
fn guest_placement(
    inventory: &xenadmin_core::Inventory,
    vm: xenadmin_core::VmId,
) -> (
    std::collections::BTreeMap<xenadmin_core::VbdId, xenadmin_core::SrId>,
    std::collections::BTreeMap<xenadmin_core::VifId, xenadmin_core::NetworkId>,
) {
    let disks = inventory
        .details
        .vbds
        .values()
        .filter(|b| b.vm == Some(vm) && b.kind == "Disk")
        .filter_map(|b| Some((b.id, inventory.details.vdis.get(&b.vdi?)?.sr?)))
        .collect();
    let nics = inventory
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(vm))
        .filter_map(|v| Some((v.id, v.network?)))
        .collect();
    (disks, nics)
}

/// P15-06, cross-pool bulk moves: the lab guest (two local disks) and the tools guest
/// (shared NFS) are halted, moved to the standalone pool in one reviewed batch and moved
/// back in a second, each disk to the repository it came from and each NIC to its
/// network. After each batch every guest must be halted on the destination pool with its
/// disks and NICs where they were mapped, and gone from the source. Guests that were
/// running are started again on their home pool at the end. A guest the single cross-pool
/// move would not take is left out and the evidence says why; with one eligible guest the
/// batch is a batch of one, and the evidence says so.
#[tokio::test]
#[ignore]
async fn lab_cross_pool_batch_move_round_trip() {
    use xenadmin_core::{
        BatchVmWorkflow, CrossPoolBatchSelection, OperationControl, PowerAction, PowerState,
        VmCommand, VmWorkflowBatch,
    };
    let lab = Lab::load();
    // `XENADMIN_LAB_CROSS_POOL_BATCH=live` leaves running guests running, so that they
    // move live; `=stop` asks the first batch to stop during its first move.
    let mode = std::env::var("XENADMIN_LAB_CROSS_POOL_BATCH").unwrap_or_default();
    let (live, stop) = (mode == "live", mode == "stop");
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
    let mut steps = Vec::new();
    let mut left_out = Vec::new();
    // The lab guest first, then the tools guest, as the home pool reports them.
    let mut candidates = home
        .latest()
        .inventory()
        .map(|i| {
            i.guests()
                .filter(|v| v.name.starts_with("lab-guest-") || v.name.starts_with("lab-tools-"))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    candidates.sort_by_key(|v| (!v.name.starts_with("lab-guest-"), v.name.clone()));
    assert!(
        !candidates.is_empty(),
        "neither lab guest is on the home pool; lab_cross_pool_move_round_trip brings the lab guest home"
    );

    // What the lab looks like before anything is changed.
    struct Guest {
        vm: xenadmin_core::Vm,
        was_running: bool,
        home_host: Option<HostId>,
        disks: std::collections::BTreeMap<xenadmin_core::VbdId, xenadmin_core::SrId>,
        nics: std::collections::BTreeMap<xenadmin_core::VifId, xenadmin_core::NetworkId>,
    }
    let mut guests = Vec::new();
    for vm in candidates {
        if !matches!(vm.power_state, PowerState::Running | PowerState::Halted) {
            left_out.push(json!({"guest": vm.name, "reason": format!("it is {:?}; a batch moves running and halted guests", vm.power_state)}));
            continue;
        }
        let inventory = home.latest().inventory().unwrap().clone();
        let (disks, nics) = guest_placement(&inventory, vm.id);
        guests.push(Guest {
            was_running: vm.power_state == PowerState::Running,
            home_host: inventory.details.vms.get(&vm.id).and_then(|d| d.affinity),
            disks,
            nics,
            vm,
        });
    }

    // This scenario moves halted guests, as the single round trip does. A running guest is
    // shut down cleanly first and started again at the end.
    let mut halted = Vec::new();
    for guest in guests {
        if guest.was_running && !live {
            let shutdown = match home.submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: home.latest().observe_vm(guest.vm.id).unwrap(),
                    action: PowerAction::CleanShutdown,
                },
            ) {
                Ok(operation) => operation,
                Err(error) => {
                    left_out.push(json!({"guest": guest.vm.name, "reason": format!("a clean shutdown was not accepted: {error}")}));
                    continue;
                }
            };
            let outcome = wait_vm_outcome(&shutdown, 300).await;
            let verified = settle(120, || {
                home.latest().inventory().is_some_and(|i| {
                    i.vms
                        .get(&guest.vm.id)
                        .is_some_and(|v| v.power_state == PowerState::Halted)
                })
            })
            .await;
            steps.push(json!({"step": "clean shutdown", "guest": guest.vm.name, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": verified}));
            assert!(
                outcome.state == OperationState::Succeeded && verified,
                "{} was not halted: {}",
                guest.vm.name,
                outcome.detail
            );
        }
        halted.push(guest);
    }

    // One destination host on each pool for a batch. On the standalone pool it is the one
    // host there is. On the way back it is a home-pool host that has every repository the
    // selected guests' disks came from; a guest that leaves no such host is left out.
    let away_inventory = away.latest().inventory().unwrap().clone();
    let away_host = away_inventory
        .coordinator
        .or_else(|| away_inventory.hosts.keys().next().copied())
        .expect("a host on the standalone pool");
    let away_sr = local_sr_of(&away_inventory, away_host).expect("a local repository there");
    let home_inventory = home.latest().inventory().unwrap().clone();
    let attached_on =
        |host: HostId, sr: xenadmin_core::SrId| {
            home_inventory.details.pbds.values().any(|p| {
                p.host == Some(host) && p.sr == Some(sr) && p.currently_attached == Some(true)
            })
        };
    let mut selected: Vec<Guest> = Vec::new();
    let mut home_host = None;
    for guest in halted {
        let repositories = selected
            .iter()
            .flat_map(|g| g.disks.values())
            .chain(guest.disks.values())
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let host = home_inventory
            .hosts
            .values()
            .filter(|h| h.enabled)
            .map(|h| h.id)
            .find(|h| repositories.iter().all(|sr| attached_on(*h, *sr)));
        let Some(host) = host else {
            left_out.push(json!({"guest": guest.vm.name, "reason": "no one home-pool host has the repositories of this guest and of those already selected, and a batch has one destination host"}));
            continue;
        };
        let reviewed = cross_pool_batch_item(
            &home,
            &away,
            guest.vm.id,
            away_host,
            &|_| Some(away_sr),
            &|_, name| {
                away_inventory
                    .networks
                    .iter()
                    .find(|n| n.name == name)
                    .map(|n| n.uuid.into())
            },
        );
        match reviewed {
            Ok(_) => {
                home_host = Some(host);
                selected.push(guest);
            }
            Err(reason) => left_out.push(json!({"guest": guest.vm.name, "reason": reason})),
        }
    }
    assert!(
        !selected.is_empty(),
        "no guest is eligible for a cross-pool move: {left_out:?}"
    );
    let home_host = home_host.expect("a home-pool host for the way back");

    // Each leg is one batch. What the destination pool shows afterwards is compared with
    // what was reviewed for each guest.
    let mut legs = Vec::new();
    let mut failures = Vec::new();
    for (label, from, to, outward) in [
        ("to the standalone pool", &home, &away, true),
        ("back home", &away, &home, false),
    ] {
        // Only guests that are on the source pool now: after a first leg that stopped
        // early, the second brings back those that went.
        let present = selected
            .iter()
            .filter(|g| {
                from.latest()
                    .inventory()
                    .is_some_and(|i| i.vms.contains_key(&g.vm.id))
            })
            .collect::<Vec<_>>();
        if present.is_empty() {
            legs.push(json!({"leg": label, "skipped": "no selected guest is on the source pool"}));
            continue;
        }
        let to_inventory = to.latest().inventory().unwrap().clone();
        let mut items = Vec::new();
        let mut reviewed = Vec::new();
        for guest in &present {
            let operation = if outward {
                cross_pool_batch_item(
                    from,
                    to,
                    guest.vm.id,
                    away_host,
                    &|_| Some(away_sr),
                    &|_, name| {
                        to_inventory
                            .networks
                            .iter()
                            .find(|n| n.name == name)
                            .map(|n| n.uuid.into())
                    },
                )
            } else {
                cross_pool_batch_item(
                    from,
                    to,
                    guest.vm.id,
                    home_host,
                    &|vbd| guest.disks.get(&vbd).copied(),
                    &|vif, _| guest.nics.get(&vif).copied(),
                )
            };
            match operation {
                Ok(operation) => {
                    reviewed.push((guest.vm.id, guest.vm.name.clone(), operation.clone()));
                    items.push(BatchVmWorkflow::move_to_pool(operation));
                }
                Err(reason) => failures.push(format!(
                    "{label}: {} cannot be reviewed: {reason}",
                    guest.vm.name
                )),
            }
        }
        if items.is_empty() {
            legs.push(json!({"leg": label, "skipped": "no guest could be reviewed"}));
            continue;
        }
        let selection = CrossPoolBatchSelection {
            source: from
                .latest()
                .observe_resource(ResourceTarget::Pool(
                    from.latest().inventory().unwrap().pool_id,
                ))
                .unwrap(),
            destination: to
                .latest()
                .observe_resource(ResourceTarget::Pool(to_inventory.pool_id))
                .unwrap(),
            batch: VmWorkflowBatch { items },
        };
        let started = std::time::Instant::now();
        let operation = from
            .move_vms_to_pool(OperationId::new(), selection, to)
            .expect("cross-pool batch accepted");
        // Stop is asked for while the first move is under way: it lets that move finish
        // and starts no other.
        let mut stopped = false;
        if stop && outward && reviewed.len() > 1 {
            let under_way = settle(120, || {
                operation
                    .latest()
                    .workflow_batch
                    .first()
                    .is_some_and(|r| r.state.as_ref().is_some_and(|s| !s.terminal()))
            })
            .await;
            let asked = from
                .control_resource(operation.latest().id, OperationControl::RequestCancellation)
                .map(|_| true)
                .map_err(|e| e.to_string());
            steps.push(json!({"step": "stop the batch during its first move", "first_move_under_way": under_way, "asked": asked}));
            stopped = true;
        }
        let outcome = wait_outcome(&operation, 3600).await;
        let mut placed = Vec::new();
        for (index, (vm, name, operation)) in reviewed.iter().enumerate() {
            let state = outcome
                .workflow_batch
                .iter()
                .find(|r| r.vm == *vm)
                .map(|r| (r.state.clone(), r.detail.clone()));
            if stopped && index > 0 {
                // Never started: still on the source pool, nowhere on the destination,
                // and its child was never admitted.
                let stayed = from
                    .latest()
                    .inventory()
                    .is_some_and(|i| i.vms.contains_key(vm))
                    && to
                        .latest()
                        .inventory()
                        .is_some_and(|i| !i.vms.contains_key(vm));
                let never_admitted = state.as_ref().is_none_or(|(s, _)| s.is_none());
                placed.push(
                    json!({"guest": name, "vm": vm.to_string(), "not_started": true,
                    "stayed_on_source": stayed, "child_never_admitted": never_admitted}),
                );
                if !(stayed && never_admitted) {
                    failures.push(format!("{label}: {name} was to be left alone by Stop: stayed {stayed}, never admitted {never_admitted}"));
                }
                continue;
            }
            let moved_running = live && present.iter().any(|g| g.vm.id == *vm && g.was_running);
            let expected_power = if moved_running {
                PowerState::Running
            } else {
                PowerState::Halted
            };
            let arrived = settle(120, || {
                to.latest().inventory().is_some_and(|i| {
                    i.vms
                        .get(vm)
                        .is_some_and(|v| &v.name == name && v.power_state == expected_power)
                })
            })
            .await;
            let departed = settle(120, || {
                from.latest()
                    .inventory()
                    .is_some_and(|i| !i.vms.contains_key(vm))
            })
            .await;
            // A move keeps the attachment and NIC identities, so the reviewed mapping can
            // be read back by them: each disk on its mapped repository, each NIC on its
            // mapped network.
            let wanted_disks = present
                .iter()
                .find(|g| g.vm.id == *vm)
                .map(|g| {
                    g.disks
                        .keys()
                        .map(|vbd| (*vbd, if outward { away_sr } else { g.disks[vbd] }))
                        .collect::<std::collections::BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            let wanted_nics = operation
                .networks
                .iter()
                .map(|n| (n.vif, n.destination_network))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mapped = settle(120, || {
                to.latest().inventory().is_some_and(|i| {
                    let (disks, nics) = guest_placement(i, *vm);
                    disks == wanted_disks && nics == wanted_nics
                })
            })
            .await;
            placed.push(json!({
                "guest": name, "vm": vm.to_string(), "moved_running": moved_running,
                "child_state": state.as_ref().map(|(s, _)| format!("{s:?}")),
                "child_detail": state.as_ref().map(|(_, d)| d.clone()),
                "arrived_in_its_power_state": arrived, "departed_from_source": departed,
                "disks_and_nics_as_reviewed": mapped,
                "disks": operation.disks.len(), "nics": operation.networks.len(),
            }));
            if !(arrived && departed && mapped) {
                failures.push(format!(
                    "{label}: {name} arrived in its power state {arrived}, departed {departed}, placed as reviewed {mapped}"
                ));
            }
        }
        legs.push(json!({
            "leg": label, "guests": reviewed.len(),
            "state": format!("{:?}", outcome.state), "detail": outcome.detail,
            "elapsed_ms": started.elapsed().as_millis() as u64, "placed": placed,
        }));
        if stopped {
            let left = format!("{} not started", reviewed.len() - 1);
            if !outcome.detail.contains(&left) || outcome.state == OperationState::Succeeded {
                failures.push(format!(
                    "{label}: a stopped batch ended {:?}: {}",
                    outcome.state, outcome.detail
                ));
            }
        } else if outcome.state != OperationState::Succeeded {
            failures.push(format!("{label}: {:?}: {}", outcome.state, outcome.detail));
        }
    }

    // Back on the home pool, the guests that were running are started again.
    for guest in selected.iter().filter(|g| g.was_running && !live) {
        let Ok(observed) = home.latest().observe_vm(guest.vm.id) else {
            failures.push(format!(
                "{} is not on the home pool to be started",
                guest.vm.name
            ));
            continue;
        };
        let start = home
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: observed,
                    action: PowerAction::Start,
                },
            )
            .expect("start accepted");
        let outcome = wait_vm_outcome(&start, 300).await;
        let running = settle(120, || {
            home.latest().inventory().is_some_and(|i| {
                i.vms
                    .get(&guest.vm.id)
                    .is_some_and(|v| v.power_state == PowerState::Running)
            })
        })
        .await;
        steps.push(json!({"step": "start on the home pool", "guest": guest.vm.name, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": running}));
        if !running {
            failures.push(format!(
                "{} did not start again: {}",
                guest.vm.name, outcome.detail
            ));
        }
    }

    // What the lab looks like afterwards, beside what it looked like before.
    let after = home.latest().inventory().unwrap().clone();
    let summary = selected
        .iter()
        .map(|g| {
            let (disks, nics) = guest_placement(&after, g.vm.id);
            let home_host_after = after.details.vms.get(&g.vm.id).and_then(|d| d.affinity);
            json!({
                "guest": g.vm.name, "vm": g.vm.id.to_string(), "was_running": g.was_running,
                "on_home_pool": after.vms.contains_key(&g.vm.id),
                "disks_on_their_repositories": disks == g.disks,
                "nics_on_their_networks": nics == g.nics,
                "home_host_before": g.home_host.map(|h| h.to_string()),
                "home_host_after": home_host_after.map(|h| h.to_string()),
            })
        })
        .collect::<Vec<_>>();
    for guest in &selected {
        let (disks, nics) = guest_placement(&after, guest.vm.id);
        if !after.vms.contains_key(&guest.vm.id) || disks != guest.disks || nics != guest.nics {
            failures.push(format!(
                "{} is not back on the home pool with its disks and NICs where they were",
                guest.vm.name
            ));
        }
    }
    lab.record(
        "cross-pool-batch-move-round-trip",
        json!({
            "mode": mode, "home_pool": home_pool.to_string(), "away_pool": away_pool.to_string(),
            "guests_in_each_batch": selected.len(),
            "batch_of_one": selected.len() == 1,
            "left_out": left_out, "steps": steps, "legs": legs, "guests": summary,
            "failures": failures,
        }),
    );
    assert!(failures.is_empty(), "{failures:#?}");
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
    // A step that does not succeed ends the scenario, after what was seen so far is
    // recorded: a failure without its steps cannot be told from a fault of the lab.
    macro_rules! must_succeed {
        ($outcome:expr) => {
            if $outcome.state != OperationState::Succeeded {
                let seen = c.latest().inventory().map(|i| {
                    json!({"enabled": i.hosts[&member].enabled,
                           "maintenance": i.details.hosts.get(&member).and_then(|h| h.maintenance_mode),
                           "agent_start_time": i.details.hosts.get(&member).and_then(|h| h.agent_start_time.clone())})
                });
                lab.record(
                    "member-toolstack-restart-and-reboot",
                    json!({"member": member_name, "steps": steps, "failed": true, "inventory_at_failure": seen}),
                );
                panic!("{:?}: {}", $outcome.state, $outcome.detail);
            }
        };
    }
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
        must_succeed!(outcome);
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
    must_succeed!(outcome);

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
    must_succeed!(outcome);

    // The toolstack restart re-enables the host while the client's maintenance marker
    // stays (finding 12); the operator's way back is exiting maintenance, which clears the
    // marker, and entering it again before the reviewed reboot.
    if c.latest()
        .inventory()
        .is_some_and(|i| i.hosts[&member].enabled)
    {
        let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
        steps.push(json!({"step": "clear the marker the restart left", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
        must_succeed!(outcome);
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
        must_succeed!(outcome);
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
    must_succeed!(outcome);
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
    steps.push(json!({"step": "exit maintenance", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": enabled,
        "step_details": outcome.steps.iter().map(|s| json!({"label": s.label, "detail": s.detail})).collect::<Vec<_>>()}));
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

/// Finding 64: whether the inventory follows a member's toolstack restart. The empty member
/// is put into maintenance and its toolstack restarted; the scenario then times how long
/// the session's inventory takes to show the member's newer start marker and that the
/// restart enabled the host again, and compares it with what a second session, opened
/// afterwards, reads fresh. An inventory that has not followed within three minutes is
/// watched for two more and the scenario fails; its record is written first either way.
/// Meant to be run in a loop, each time shortly after the coordinator's own toolstack was
/// restarted from outside, with `XENADMIN_DEV_RPC_TRACE=1` to see the events that arrive.
#[tokio::test]
#[ignore]
async fn lab_inventory_follows_a_member_toolstack_restart() {
    use xenadmin_core::{HostOperation, ResourceCommand, ResourceTarget};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let connected = std::time::Instant::now();
    let c = connect(&lab, &backend, 0, false).await;
    let connect_ms = connected.elapsed().as_millis() as u64;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory.coordinator.expect("coordinator");
    let member = *inventory
        .hosts
        .keys()
        .find(|h| **h != coordinator)
        .expect("a member");
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must not host guests"
    );
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
    let seen = |c: &Connection| {
        c.latest().inventory().map(|i| {
            (
                i.hosts[&member].enabled,
                i.details
                    .hosts
                    .get(&member)
                    .and_then(|h| h.agent_start_time.clone()),
                i.details
                    .hosts
                    .get(&member)
                    .and_then(|h| h.maintenance_mode),
            )
        })
    };
    let mut steps = Vec::new();
    if inventory.hosts[&member].enabled && seen(&c).and_then(|s| s.2) == Some(true) {
        let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
        steps.push(json!({"step": "clear a stale maintenance marker", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    }
    let enabled_before = c.latest().inventory().unwrap().hosts[&member].enabled;
    let outcome = host_op(
        &c,
        member,
        HostOperation::EnterMaintenance {
            expected_enabled: enabled_before,
            resident_vms: Vec::new(),
        },
        300,
    )
    .await;
    let entered = std::time::Instant::now();
    let disabled_seen = settle(60, || seen(&c).is_some_and(|s| !s.0)).await;
    steps.push(json!({"step": "enter maintenance", "state": format!("{:?}", outcome.state), "detail": outcome.detail,
        "inventory_shows_disabled": disabled_seen, "after_ms": entered.elapsed().as_millis() as u64}));
    let entered_ok = outcome.state == OperationState::Succeeded;

    let agent_before = seen(&c).and_then(|s| s.1);
    let mut followed = json!(null);
    let mut ok = false;
    if entered_ok && let Some(agent_before) = agent_before.clone() {
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
        let operation_ms = started.elapsed().as_millis() as u64;
        let waited = std::time::Instant::now();
        let marker = settle(180, || {
            seen(&c).is_some_and(|s| s.1.as_ref().is_some_and(|t| *t != agent_before))
        })
        .await;
        let marker_ms = waited.elapsed().as_millis() as u64;
        let enabled = settle(60, || seen(&c).is_some_and(|s| s.0)).await;
        let enabled_ms = waited.elapsed().as_millis() as u64;
        // What a session opened now reads fresh, to tell a stale inventory from a host
        // that has not yet written its new state.
        let second_state = tempfile::tempdir().unwrap();
        let second_backend = Backend::with_state_directory(second_state.path().into()).unwrap();
        let second = connect(&lab, &second_backend, 0, false).await;
        let fresh = seen(&second);
        second.close().await;
        let first = seen(&c);
        // A session that had not followed is watched a little longer: whether it catches
        // up by itself is part of what happened.
        let mut later = json!(null);
        if !(marker && enabled) {
            let more = std::time::Instant::now();
            let caught_up = settle(120, || {
                seen(&c).is_some_and(|s| s.0 && s.1.as_ref().is_some_and(|t| *t != agent_before))
            })
            .await;
            later = json!({"caught_up": caught_up, "after_ms": more.elapsed().as_millis() as u64});
        }
        ok = outcome.state == OperationState::Succeeded && marker && enabled;
        followed = json!({
            "operation": format!("{:?}", outcome.state), "operation_detail": outcome.detail, "operation_ms": operation_ms,
            "inventory_shows_newer_start_marker": marker, "marker_after_ms": marker_ms,
            "inventory_shows_enabled": enabled, "enabled_after_ms": enabled_ms,
            "first_session": first.as_ref().map(|s| json!({"enabled": s.0, "agent_start_time_changed": s.1.as_ref() != Some(&agent_before), "maintenance": s.2})),
            "fresh_session": fresh.as_ref().map(|s| json!({"enabled": s.0, "agent_start_time_changed": s.1.as_ref() != Some(&agent_before), "maintenance": s.2})),
            "watched_longer": later,
        });
    }
    // Back out of maintenance, whatever the inventory shows: the operation reads the host.
    let outcome = host_op(&c, member, HostOperation::ExitMaintenance, 300).await;
    let out_seen = settle(60, || seen(&c).is_some_and(|s| s.0 && s.2 != Some(true))).await;
    steps.push(json!({"step": "exit maintenance", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "inventory_shows_it": out_seen}));
    c.close().await;
    lab.record(
        "inventory-follows-a-member-toolstack-restart",
        json!({"connect_ms": connect_ms, "steps": steps, "restart": followed, "followed": ok}),
    );
    assert!(entered_ok, "the member did not enter maintenance");
    assert!(
        ok,
        "the inventory did not follow the member's toolstack restart: {followed}"
    );
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

/// P11-08: the halted guest is exported as an OVA package, the package is inspected and
/// imported into the standalone pool with its network mapped by name, the import is
/// deleted with its disks, the file removed and the guest started again.
#[tokio::test]
#[ignore]
async fn lab_ovf_export_and_import_across_pools() {
    use xenadmin_core::{
        ExportSource, OvfPackageFormat, PowerAction, PowerState, ResourceTarget, TransferAction,
        TransferFile, TransferRequest, VmCommand, VmLifecycleOperation, VmSettingEdit,
        VmSettingsPatch,
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

    // The OVF writer represents the dc boot order only; set it through the reviewed boot
    // order editor and restore the guest's own order afterwards.
    async fn boot_order(
        c: &Connection,
        vm: xenadmin_core::VmId,
        previous: &str,
        value: &str,
    ) -> std::sync::Arc<xenadmin_core::OperationRecord> {
        let patch = VmSettingsPatch::new(vec![VmSettingEdit::BootOrder {
            previous: previous.into(),
            value: value.into(),
        }])
        .unwrap();
        let op = c
            .submit(
                OperationId::new(),
                VmCommand::Configure {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    patch,
                },
            )
            .expect("boot order edit accepted");
        wait_vm_outcome(&op, 120).await
    }
    let original_order = home.latest().inventory().unwrap().details.vms[&guest.id]
        .boot_order
        .clone()
        .unwrap_or_default();
    if original_order != "dc" {
        let outcome = boot_order(&home, guest.id, &original_order, "dc").await;
        let check = settle(60, || {
            home.latest()
                .inventory()
                .is_some_and(|i| i.details.vms[&guest.id].boot_order.as_deref() == Some("dc"))
        })
        .await;
        steps.push(json!({"step": "set boot order dc for the export", "from": original_order, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(check);
    }

    // Export as an OVA package.
    let directory = lab.evidence.parent().unwrap().join("transfers");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("lab-guest-{stamp}.ova"));
    let started = std::time::Instant::now();
    let export = home
        .transfer(
            OperationId::new(),
            home.latest()
                .observe_resource(ResourceTarget::Pool(home_inventory.pool_id))
                .unwrap(),
            TransferRequest {
                action: TransferAction::ExportOvf {
                    source: ExportSource::Vm {
                        vm: guest.id,
                        expected_template: false,
                    },
                    format: OvfPackageFormat::Ova,
                },
                file: TransferFile::destination(&path).expect("export destination"),
            },
        )
        .expect("export accepted");
    let outcome = wait_outcome(&export, 1800).await;
    let size = std::fs::metadata(&path).map(|m| m.len()).ok();
    steps.push(json!({"step": "export OVA", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "bytes": size}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );

    // Inspect and import with the package's networks mapped by name.
    let file = TransferFile::ovf_source(&path).expect("inspect package");
    let summary = file.ovf().cloned().expect("OVF summary");
    let destination_host = away_inventory
        .coordinator
        .or_else(|| away_inventory.hosts.keys().next().copied())
        .expect("destination host");
    let sr = local_sr_of(&away_inventory, destination_host).expect("destination repository");
    let fallback = away_inventory
        .networks
        .first()
        .map(|n| xenadmin_core::NetworkId::from(n.uuid))
        .expect("a destination network");
    let networks = summary
        .networks
        .iter()
        .map(|name| {
            let target = away_inventory
                .networks
                .iter()
                .find(|n| &n.name == name)
                .map(|n| xenadmin_core::NetworkId::from(n.uuid))
                .unwrap_or(fallback);
            (name.clone(), target)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let inspected = json!({"guests": summary.guests.iter().map(|g| json!({"name": g.name, "cpus": g.cpus, "memory": g.memory_bytes, "disks": g.disks.len()})).collect::<Vec<_>>(), "disks": summary.disks.len(), "networks": summary.networks, "manifest_verified": summary.manifest_verified});
    let started = std::time::Instant::now();
    let import = away
        .transfer(
            OperationId::new(),
            away.latest()
                .observe_resource(ResourceTarget::Pool(away_inventory.pool_id))
                .unwrap(),
            TransferRequest {
                action: TransferAction::ImportOvf { sr, networks },
                file,
            },
        )
        .expect("import accepted");
    let outcome = wait_outcome(&import, 1800).await;
    let imported = outcome.transfer_created.vms.first().copied();
    let visible = settle(120, || {
        imported.is_some_and(|id| {
            away.latest()
                .inventory()
                .is_some_and(|i| i.vms.contains_key(&id))
        })
    })
    .await;
    let imported_disks = imported.map(|id| disks_of(away.latest().inventory().unwrap(), id).len());
    steps.push(json!({"step": "import OVA", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "created_vms": outcome.transfer_created.vms.len(), "created_disks": outcome.transfer_created.disks.len(), "visible": visible, "disks": imported_disks}));
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
    if original_order != "dc" {
        let outcome = boot_order(&home, guest.id, "dc", &original_order).await;
        let check = settle(60, || {
            home.latest().inventory().is_some_and(|i| {
                i.details.vms[&guest.id].boot_order.as_deref() == Some(original_order.as_str())
            })
        })
        .await;
        steps.push(json!({"step": "restore boot order", "to": original_order, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }
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
        "ovf-export-and-import-across-pools",
        json!({"guest": guest.name, "package": inspected, "file_removed": removed, "steps": steps}),
    );
    assert!(gone && running);
    home.close().await;
    away.close().await;
}

/// P09-02, P09-03, P09-06, P09-08 and P09-09: the standalone host's untouched data disk is
/// probed without formatting, a local EXT repository created on it after the explicit data
/// loss acknowledgement, rescanned, detached and reattached through its attachment, and
/// finally destroyed with no disks reviewed, leaving the host as found.
#[tokio::test]
#[ignore]
async fn lab_local_storage_lifecycle_on_standalone_host() {
    use xenadmin_core::{
        LocalDeviceDeclaration, LocalStorageKind, ResourceCommand, ResourceTarget,
        StorageDiscoveryResult, StorageLocation, StorageOperation, StorageQuery,
    };
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 3, "three hosts are required");
    let device = std::env::var("XENADMIN_LAB_DATA_DEVICE")
        .unwrap_or_else(|_| "/dev/disk/by-id/ata-QEMU_HARDDISK_lab-data".into());
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 2, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let host = inventory
        .coordinator
        .or_else(|| inventory.hosts.keys().next().copied())
        .expect("host");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    async fn storage(
        c: &Connection,
        target: ResourceTarget,
        operation: StorageOperation,
        seconds: u64,
    ) -> std::sync::Arc<xenadmin_core::ResourceOperationRecord> {
        let op = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Storage {
                    target: c.latest().observe_resource(target).unwrap(),
                    operation,
                },
            )
            .expect("storage command accepted");
        wait_outcome(&op, seconds).await
    }
    let find_sr = |c: &Connection, name: &str| {
        c.latest()
            .inventory()
            .and_then(|i| i.storage.iter().find(|s| s.name == name).cloned())
    };

    // Leftovers of an interrupted run are detached and destroyed first through the same
    // reviewed operations; a repository must be detached everywhere before it is destroyed.
    let attachment_of = |c: &Connection, sr: xenadmin_core::SrId| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pbds
                .values()
                .find(|p| p.sr == Some(sr) && p.host == Some(host))
                .map(|p| (p.id, p.currently_attached))
        })
    };
    for leftover in inventory
        .storage
        .iter()
        .filter(|s| s.name.starts_with("lab-sr-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        let leftover_id: xenadmin_core::SrId = leftover.uuid.into();
        if let Some((pbd, Some(true))) = attachment_of(&c, leftover_id) {
            let outcome = storage(
                &c,
                ResourceTarget::Pbd(pbd),
                StorageOperation::SetAttached {
                    sr: leftover_id,
                    host,
                    expected_attached: true,
                    attached: false,
                },
                300,
            )
            .await;
            steps.push(json!({"step": "detach a leftover repository", "name": leftover.name, "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
            settle(60, || {
                attachment_of(&c, leftover_id).is_some_and(|(_, now)| now == Some(false))
            })
            .await;
        }
        let outcome = storage(
            &c,
            ResourceTarget::Storage(leftover_id),
            StorageOperation::Destroy {
                expected_name: leftover.name.clone(),
                expected_disks: Vec::new(),
            },
            300,
        )
        .await;
        steps.push(json!({"step": "destroy a leftover repository", "name": leftover.name, "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
        settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| !i.storage.iter().any(|s| s.uuid == leftover.uuid))
        })
        .await;
    }

    // Probe the device without formatting it.
    let probe = c
        .discover_storage(
            c.latest()
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .unwrap(),
            StorageQuery::LocalDevice {
                host,
                device: device.clone(),
            },
        )
        .expect("probe accepted");
    let result = tokio::time::timeout(deadline(180), async {
        loop {
            if let Some(result) = probe.latest() {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("probe answered");
    let existing = match &result {
        Ok(StorageDiscoveryResult::LocalRepositories(rows)) => Some(rows.iter().map(|r| json!({"kind": format!("{:?}", r.kind), "uuid": r.repository.uuid.to_string()})).collect::<Vec<_>>()),
        _ => None,
    };
    steps.push(json!({"step": "probe the data disk", "device": device, "result": match &result { Ok(_) => "ok".to_string(), Err(e) => e.clone() }, "existing_repositories": existing}));
    assert!(
        existing.as_ref().is_some_and(|e| e.is_empty()),
        "the data disk must be untouched: {result:?}"
    );

    // Create a local EXT repository on it.
    let name = format!("lab-sr-{stamp}");
    let outcome = storage(
        &c,
        ResourceTarget::Pool(inventory.pool_id),
        StorageOperation::Create {
            host,
            name: name.clone(),
            description: "Local repository created by the live storage scenario".into(),
            location: StorageLocation::Local {
                kind: LocalStorageKind::Ext,
                device: device.clone(),
            },
            hba_device: None,
            local_device: Some(LocalDeviceDeclaration {
                identity_statement: "Operator declares the lab data disk for VM storage".into(),
                acknowledge_data_loss: true,
            }),
        },
        900,
    )
    .await;
    let created = settle(120, || find_sr(&c, &name).is_some()).await;
    let sr = find_sr(&c, &name);
    steps.push(json!({"step": "create local EXT repository", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "visible": created, "kind": sr.as_ref().map(|s| s.kind.clone()), "capacity_bytes": sr.as_ref().and_then(|s| s.capacity_bytes)}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let sr = sr.expect("repository visible");
    let sr_id: xenadmin_core::SrId = sr.uuid.into();

    // Rescan, detach and reattach.
    let outcome = {
        let op = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::ScanStorage {
                    sr: c
                        .latest()
                        .observe_resource(ResourceTarget::Storage(sr_id))
                        .unwrap(),
                },
            )
            .expect("scan accepted");
        wait_outcome(&op, 300).await
    };
    steps.push(json!({"step": "rescan", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    // Detach and reattach through the host's attachment record.
    let attachment = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pbds
                .values()
                .find(|p| p.sr == Some(sr_id) && p.host == Some(host))
                .map(|p| (p.id, p.currently_attached))
        })
    };
    let (pbd, _) = attachment(&c).expect("the repository's attachment on the host");
    for attached in [false, true] {
        let outcome = storage(
            &c,
            ResourceTarget::Pbd(pbd),
            StorageOperation::SetAttached {
                sr: sr_id,
                host,
                expected_attached: !attached,
                attached,
            },
            300,
        )
        .await;
        let check = settle(60, || {
            attachment(&c).is_some_and(|(_, now)| now == Some(attached))
        })
        .await;
        steps.push(json!({"step": if attached { "reattach" } else { "detach" }, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
        assert!(check);
    }

    // Detach again, then destroy it, reviewing that it holds no disks.
    let outcome = storage(
        &c,
        ResourceTarget::Pbd(pbd),
        StorageOperation::SetAttached {
            sr: sr_id,
            host,
            expected_attached: true,
            attached: false,
        },
        300,
    )
    .await;
    let check = settle(60, || {
        attachment(&c).is_some_and(|(_, now)| now == Some(false))
    })
    .await;
    steps.push(json!({"step": "detach before destroying", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let outcome = storage(
        &c,
        ResourceTarget::Storage(sr_id),
        StorageOperation::Destroy {
            expected_name: name.clone(),
            expected_disks: Vec::new(),
        },
        300,
    )
    .await;
    let gone = settle(120, || find_sr(&c, &name).is_none()).await;
    steps.push(json!({"step": "destroy", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "gone": gone}));
    lab.record(
        "local-storage-lifecycle-on-standalone-host",
        json!({"host": lab.hosts[2].name, "device": device, "steps": steps}),
    );
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(gone);
    c.close().await;
}

/// P09-05 and P09-09: a local repository on the standalone host's data disk is forgotten
/// (its records dropped, its data kept), rediscovered on the disk by the non-formatting
/// probe, introduced again by its exact UUID, given a host attachment and attached, then
/// detached and destroyed, leaving the disk as found.
#[tokio::test]
#[ignore]
async fn lab_local_storage_forget_and_introduce() {
    use xenadmin_core::{
        LocalDeviceDeclaration, LocalStorageKind, ResourceCommand, ResourceTarget,
        StorageDiscoveryResult, StorageLocation, StorageOperation, StorageQuery,
    };
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 3, "three hosts are required");
    let device = std::env::var("XENADMIN_LAB_DATA_DEVICE")
        .unwrap_or_else(|_| "/dev/disk/by-id/ata-QEMU_HARDDISK_lab-data".into());
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 2, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let host = inventory
        .coordinator
        .or_else(|| inventory.hosts.keys().next().copied())
        .expect("host");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    async fn storage(
        c: &Connection,
        target: ResourceTarget,
        operation: StorageOperation,
        seconds: u64,
    ) -> std::sync::Arc<xenadmin_core::ResourceOperationRecord> {
        let op = c
            .submit_resource(
                OperationId::new(),
                ResourceCommand::Storage {
                    target: c.latest().observe_resource(target).unwrap(),
                    operation,
                },
            )
            .expect("storage command accepted");
        wait_outcome(&op, seconds).await
    }
    let attachment_of = |c: &Connection, sr: xenadmin_core::SrId| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pbds
                .values()
                .find(|p| p.sr == Some(sr) && p.host == Some(host))
                .map(|p| (p.id, p.currently_attached))
        })
    };
    let find_sr = |c: &Connection, id: xenadmin_core::SrId| {
        c.latest().inventory().and_then(|i| {
            i.storage
                .iter()
                .find(|s| xenadmin_core::SrId::from(s.uuid) == id)
                .cloned()
        })
    };
    async fn probe(
        c: &Connection,
        pool: xenadmin_core::PoolId,
        host: xenadmin_core::HostId,
        device: &str,
    ) -> Result<StorageDiscoveryResult, String> {
        let probe = c
            .discover_storage(
                c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                StorageQuery::LocalDevice {
                    host,
                    device: device.into(),
                },
            )
            .expect("probe accepted");
        tokio::time::timeout(deadline(180), async {
            loop {
                if let Some(result) = probe.latest() {
                    break result;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .expect("probe answered")
    }

    // Any repository already on the disk is destroyed first (detached, then destroyed).
    for leftover in inventory
        .storage
        .iter()
        .filter(|s| s.name.starts_with("lab-sr-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        let id: xenadmin_core::SrId = leftover.uuid.into();
        if let Some((pbd, Some(true))) = attachment_of(&c, id) {
            let outcome = storage(
                &c,
                ResourceTarget::Pbd(pbd),
                StorageOperation::SetAttached {
                    sr: id,
                    host,
                    expected_attached: true,
                    attached: false,
                },
                300,
            )
            .await;
            steps.push(json!({"step": "detach a leftover repository", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
            settle(60, || {
                attachment_of(&c, id).is_some_and(|(_, now)| now == Some(false))
            })
            .await;
        }
        let outcome = storage(
            &c,
            ResourceTarget::Storage(id),
            StorageOperation::Destroy {
                expected_name: leftover.name.clone(),
                expected_disks: Vec::new(),
            },
            300,
        )
        .await;
        steps.push(json!({"step": "destroy a leftover repository", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
        settle(60, || find_sr(&c, id).is_none()).await;
    }

    // Create, detach, forget.
    let name = format!("lab-sr-{stamp}");
    let location = StorageLocation::Local {
        kind: LocalStorageKind::Ext,
        device: device.clone(),
    };
    let outcome = storage(
        &c,
        ResourceTarget::Pool(inventory.pool_id),
        StorageOperation::Create {
            host,
            name: name.clone(),
            description: "Local repository for the forget and introduce scenario".into(),
            location: location.clone(),
            hba_device: None,
            local_device: Some(LocalDeviceDeclaration {
                identity_statement: "Operator declares the lab data disk for VM storage".into(),
                acknowledge_data_loss: true,
            }),
        },
        900,
    )
    .await;
    let sr = tokio::time::timeout(deadline(120), async {
        loop {
            if let Some(s) = c
                .latest()
                .inventory()
                .and_then(|i| i.storage.iter().find(|s| s.name == name).cloned())
            {
                break s;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("repository visible");
    let sr_id: xenadmin_core::SrId = sr.uuid.into();
    steps.push(json!({"step": "create", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "uuid": sr_id.to_string()}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let (pbd, _) = attachment_of(&c, sr_id).expect("attachment");
    let outcome = storage(
        &c,
        ResourceTarget::Pbd(pbd),
        StorageOperation::SetAttached {
            sr: sr_id,
            host,
            expected_attached: true,
            attached: false,
        },
        300,
    )
    .await;
    settle(60, || {
        attachment_of(&c, sr_id).is_some_and(|(_, now)| now == Some(false))
    })
    .await;
    steps.push(json!({"step": "detach", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let outcome = storage(
        &c,
        ResourceTarget::Storage(sr_id),
        StorageOperation::Forget {
            expected_name: name.clone(),
            expected_disks: Vec::new(),
        },
        300,
    )
    .await;
    let forgotten = settle(60, || find_sr(&c, sr_id).is_none()).await;
    steps.push(json!({"step": "forget", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "record_gone": forgotten}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(forgotten);

    // The data is still on the disk: the probe finds the repository by its UUID.
    let result = probe(&c, inventory.pool_id, host, &device).await;
    let found = match &result {
        Ok(StorageDiscoveryResult::LocalRepositories(rows)) => {
            rows.iter().find(|r| r.repository.uuid == sr_id).map(
                |r| json!({"kind": format!("{:?}", r.kind), "size_bytes": r.repository.size_bytes}),
            )
        }
        _ => None,
    };
    steps.push(json!({"step": "probe after forgetting", "found": found, "result": match &result { Ok(_) => "ok".to_string(), Err(e) => e.clone() }}));
    assert!(
        found.is_some(),
        "the forgotten repository was not found on the disk: {result:?}"
    );

    // Introduce it by exact UUID, give it a host attachment and attach it.
    let outcome = storage(
        &c,
        ResourceTarget::Pool(inventory.pool_id),
        StorageOperation::Introduce {
            uuid: sr_id,
            name: name.clone(),
            description: "Introduced again by the live scenario".into(),
            location: location.clone(),
        },
        300,
    )
    .await;
    let back = settle(120, || find_sr(&c, sr_id).is_some()).await;
    steps.push(json!({"step": "introduce by UUID", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "visible": back}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(back);
    let outcome = storage(
        &c,
        ResourceTarget::Storage(sr_id),
        StorageOperation::CreateAttachment {
            host,
            location: location.clone(),
        },
        300,
    )
    .await;
    let attachment = settle(120, || attachment_of(&c, sr_id).is_some()).await;
    steps.push(json!({"step": "create host attachment", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "visible": attachment}));
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    let (pbd, attached) = attachment_of(&c, sr_id).expect("attachment");
    if attached != Some(true) {
        let outcome = storage(
            &c,
            ResourceTarget::Pbd(pbd),
            StorageOperation::SetAttached {
                sr: sr_id,
                host,
                expected_attached: false,
                attached: true,
            },
            300,
        )
        .await;
        let check = settle(60, || {
            attachment_of(&c, sr_id).is_some_and(|(_, now)| now == Some(true))
        })
        .await;
        steps.push(json!({"step": "attach", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "verified": check}));
        assert_eq!(
            outcome.state,
            OperationState::Succeeded,
            "{}",
            outcome.detail
        );
    }

    // Leave the disk as found.
    let outcome = storage(
        &c,
        ResourceTarget::Pbd(pbd),
        StorageOperation::SetAttached {
            sr: sr_id,
            host,
            expected_attached: true,
            attached: false,
        },
        300,
    )
    .await;
    settle(60, || {
        attachment_of(&c, sr_id).is_some_and(|(_, now)| now == Some(false))
    })
    .await;
    steps.push(json!({"step": "detach before destroying", "state": format!("{:?}", outcome.state), "detail": outcome.detail}));
    let outcome = storage(
        &c,
        ResourceTarget::Storage(sr_id),
        StorageOperation::Destroy {
            expected_name: name.clone(),
            expected_disks: Vec::new(),
        },
        300,
    )
    .await;
    let gone = settle(120, || find_sr(&c, sr_id).is_none()).await;
    steps.push(json!({"step": "destroy", "state": format!("{:?}", outcome.state), "detail": outcome.detail, "gone": gone}));
    lab.record(
        "local-storage-forget-and-introduce",
        json!({"host": lab.hosts[2].name, "device": device, "steps": steps}),
    );
    assert_eq!(
        outcome.state,
        OperationState::Succeeded,
        "{}",
        outcome.detail
    );
    assert!(gone);
    c.close().await;
}

/// VM settings, failure column: an edit reviewed against a description that another client
/// has changed meanwhile is refused before any write, and a fresh review succeeds.
#[tokio::test]
#[ignore]
async fn lab_concurrent_description_edit_is_refused() {
    use xenadmin_core::{VmCommand, VmSettingEdit, VmSettingsPatch};
    let lab = Lab::load();
    let first_state = tempfile::tempdir().unwrap();
    let second_state = tempfile::tempdir().unwrap();
    let first_backend = Backend::with_state_directory(first_state.path().into()).unwrap();
    let second_backend = Backend::with_state_directory(second_state.path().into()).unwrap();
    let first = connect(&lab, &first_backend, 0, false).await;
    let second = connect(&lab, &second_backend, 0, false).await;
    let guest = first
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-"))
        .expect("lab guest")
        .clone();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    async fn edit(
        c: &Connection,
        vm: xenadmin_core::VmId,
        previous: &str,
        value: &str,
    ) -> Result<std::sync::Arc<xenadmin_core::OperationRecord>, String> {
        let patch = VmSettingsPatch::new(vec![VmSettingEdit::Description {
            previous: previous.into(),
            value: value.into(),
        }])
        .unwrap();
        let op = c
            .submit(
                OperationId::new(),
                VmCommand::Configure {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    patch,
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(wait_vm_outcome(&op, 120).await)
    }

    // The first client reviews the current description; the second changes it meanwhile.
    let reviewed = guest.description.clone();
    let meanwhile = format!("Changed by another client at {stamp}");
    let other = edit(&second, guest.id, &reviewed, &meanwhile)
        .await
        .expect("second client's edit accepted");
    assert_eq!(other.state, OperationState::Succeeded, "{}", other.detail);
    let seen = settle(60, || {
        first
            .latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].description == meanwhile)
    })
    .await;

    // The stale review is refused before any write; a fresh one succeeds.
    let stale = edit(
        &first,
        guest.id,
        &reviewed,
        &format!("Stale review at {stamp}"),
    )
    .await;
    let stale_outcome = match &stale {
        Ok(record) => {
            json!({"accepted": true, "state": format!("{:?}", record.state), "detail": record.detail})
        }
        Err(error) => json!({"accepted": false, "error": error}),
    };
    let unchanged = first
        .latest()
        .inventory()
        .is_some_and(|i| i.vms[&guest.id].description == meanwhile);
    let fresh_value = format!("Fresh review at {stamp}");
    let fresh = edit(&first, guest.id, &meanwhile, &fresh_value)
        .await
        .expect("fresh edit accepted");
    let applied = settle(60, || {
        first
            .latest()
            .inventory()
            .is_some_and(|i| i.vms[&guest.id].description == fresh_value)
    })
    .await;
    lab.record(
        "concurrent-description-edit-refused",
        json!({
            "guest": guest.name,
            "other_client_edit": {"state": format!("{:?}", other.state), "seen_by_first_client": seen},
            "stale_review": stale_outcome,
            "description_unchanged_by_stale_review": unchanged,
            "fresh_review": {"state": format!("{:?}", fresh.state), "detail": fresh.detail, "applied": applied},
        }),
    );
    assert!(
        seen,
        "the first client did not observe the other client's change"
    );
    assert!(
        !stale
            .as_ref()
            .is_ok_and(|r| r.state == OperationState::Succeeded),
        "a stale review was applied"
    );
    assert!(unchanged);
    assert_eq!(fresh.state, OperationState::Succeeded, "{}", fresh.detail);
    assert!(applied);
    first.close().await;
    second.close().await;
}

/// VM settings, failure column: a description edit whose reply is lost because the client
/// closes right after submitting it is recovered on restart, resolved by a recheck against
/// the host's current value, and never repeated.
#[tokio::test]
#[ignore]
async fn lab_interrupted_description_edit_resolves_on_restart() {
    use xenadmin_core::{OperationControl, VmCommand, VmSettingEdit, VmSettingsPatch};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let value = format!("Interrupted edit at {stamp}");
    let id = OperationId::new();
    let (guest_id, previous, state_at_close) = {
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
        let patch = VmSettingsPatch::new(vec![VmSettingEdit::Description {
            previous: guest.description.clone(),
            value: value.clone(),
        }])
        .unwrap();
        let operation = c
            .submit(
                id,
                VmCommand::Configure {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    patch,
                },
            )
            .expect("edit accepted");
        // Close at once: a single setter call completes within milliseconds.
        tokio::task::yield_now().await;
        let state_at_close = format!("{:?}", operation.latest().state);
        c.close().await;
        (guest.id, guest.description.clone(), state_at_close)
    };

    // Same state directory, new process.
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let (record, timeline) = wait_history_outcome(&c, id, 300).await;
    let record = record.expect("interrupted record recovered");
    let mut recheck = None;
    let record = if record.state == OperationState::OutcomeUnknown {
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
        recheck = Some(json!({"before": before.1, "moved": moved, "timeline": more}));
        after.expect("record after recheck")
    } else {
        record
    };
    tokio::time::sleep(Duration::from_secs(3)).await;
    let current = c.latest().inventory().unwrap().vms[&guest_id]
        .description
        .clone();
    let consistent = match record.state {
        OperationState::Succeeded => current == value,
        OperationState::Failed | OperationState::Rejected => current == previous,
        _ => current == value || current == previous,
    };
    lab.record(
        "interrupted-description-edit",
        json!({
            "state_at_close": state_at_close,
            "recovered": {"state": format!("{:?}", record.state), "detail": record.detail, "timeline": timeline},
            "recheck": recheck,
            "description_now": if current == value { "new" } else if current == previous { "previous" } else { "other" },
            "consistent_with_record": consistent,
        }),
    );
    assert!(
        consistent,
        "record {:?} does not match the host's description",
        record.state
    );
    c.close().await;
}

/// Snapshot marker, failure column: a reviewed restore (shutdown, revert, start) is cut
/// off by closing the client while it is under way; the restarted client recovers the
/// workflow record without submitting its remaining steps, a recheck reports the saved
/// step evidence, and the operator finishes by starting the guest.
#[tokio::test]
#[ignore]
async fn lab_interrupted_snapshot_restore_recovers_without_replay() {
    use xenadmin_core::{
        OperationControl, PowerAction, PowerState, SnapshotName, SnapshotWorkflow, VmCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let name = format!("lab-restore-interrupt-{stamp}");
    let id = OperationId::new();
    let (guest_id, snapshot_id, state_at_close) = {
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
        for leftover in c
            .latest()
            .inventory()
            .unwrap()
            .snapshots
            .for_vm(guest.id)
            .filter(|s| s.name.starts_with("lab-restore-interrupt-"))
            .map(|s| s.id)
            .collect::<Vec<_>>()
        {
            let disks = c
                .latest()
                .inventory()
                .map(|i| {
                    i.details
                        .vbds
                        .values()
                        .filter(|b| b.snapshot == Some(leftover) && b.kind == "Disk")
                        .filter_map(|b| b.vdi)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let deletion = c
                .submit(
                    OperationId::new(),
                    VmCommand::DeleteSnapshot {
                        snapshot: c.latest().observe_snapshot(guest.id, leftover).unwrap(),
                        disks,
                    },
                )
                .expect("leftover deletion accepted");
            wait_vm_outcome(&deletion, 300).await;
        }
        if guest.power_state != PowerState::Running {
            let start = c
                .submit(
                    OperationId::new(),
                    VmCommand::Power {
                        vm: c.latest().observe_vm(guest.id).unwrap(),
                        action: PowerAction::Start,
                    },
                )
                .expect("start accepted");
            wait_vm_outcome(&start, 300).await;
            assert!(
                settle(120, || c
                    .latest()
                    .inventory()
                    .is_some_and(
                        |i| i.vms[&guest.id].power_state == PowerState::Running
                    ))
                .await
            );
        }
        let snapshot = c
            .submit(
                OperationId::new(),
                VmCommand::CreateSnapshot {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    name: SnapshotName::new(&name).unwrap(),
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
        let snapshot_id = tokio::time::timeout(deadline(60), async {
            loop {
                if let Some(id) = c.latest().inventory().and_then(|i| {
                    i.snapshots
                        .for_vm(guest.id)
                        .find(|s| s.name == name)
                        .map(|s| s.id)
                }) {
                    return id;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .expect("snapshot visible");
        let restore = c
            .submit(
                id,
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
        // Let the first step (the clean shutdown) get under way, then stop like a crash.
        // Only a record that is tracking a host task has been admitted to the journal: a
        // restore refused before admission (the guest or snapshot not yet advertising the
        // action, another operation holding the guest) ends Rejected on the handle alone and
        // leaves nothing to recover. The earlier wait accepted that state, and the still
        // unadmitted one, as "under way"; one attempt in four then found no record.
        let mut seen = Vec::new();
        let tracking = tokio::time::timeout(deadline(60), async {
            loop {
                let latest = restore.latest();
                let entry = format!("{:?}: {}", latest.state, latest.detail);
                if seen.last() != Some(&entry) {
                    seen.push(entry);
                }
                if latest.state == OperationState::Tracking || latest.state.terminal() {
                    return latest.state == OperationState::Tracking;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap_or(false);
        if !tracking {
            lab.record(
                "interrupted-snapshot-restore-not-admitted",
                json!({"operation": id.to_string(), "states": seen}),
            );
            panic!("the restore never reached a tracked host task: {seen:?}");
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state_at_close = format!("{:?}", restore.latest().state);
        c.close().await;
        (guest.id, snapshot_id, state_at_close)
    };

    // Same state directory, new process: the workflow is recovered, never resumed.
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let (record, timeline) = wait_history_outcome(&c, id, 300).await;
    let record = record.unwrap_or_else(|| {
        let history = c
            .operation_history()
            .iter()
            .map(|r| format!("{} {:?} {}", r.id, r.state, r.detail))
            .collect::<Vec<_>>();
        // One early attempt ended here and left nothing to diagnose it with. Keep what the
        // interrupted client last saw and what the guest is doing before failing.
        lab.record(
            "interrupted-snapshot-restore-not-recovered",
            json!({"operation": id.to_string(), "state_at_close": &state_at_close,
                   "timeline": &timeline, "history_after_restart": &history,
                   "guest_power": c.latest().inventory().and_then(|i| i.vms.get(&guest_id).map(|vm| format!("{:?}", vm.power_state)))}),
        );
        panic!("interrupted workflow {id} not recovered; history after restart: {history:?}")
    });
    let mut recheck = None;
    let record = if record.state == OperationState::OutcomeUnknown {
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
        recheck = Some(
            json!({"before": before.1, "moved": moved, "timeline": more, "steps": after.as_ref().map(|r| r.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>())}),
        );
        after.expect("record after recheck")
    } else {
        record
    };
    tokio::time::sleep(Duration::from_secs(5)).await;
    let power_after_recovery = format!(
        "{:?}",
        c.latest().inventory().unwrap().vms[&guest_id].power_state
    );
    // An unresolved workflow holds the guest; the operator acknowledges it to go on.
    let mut acknowledged = json!(null);
    if record.state == OperationState::OutcomeUnknown {
        c.control_operation(id, OperationControl::AcknowledgeUnknown)
            .expect("acknowledgement accepted");
        let done = settle(120, || {
            c.operation_history()
                .iter()
                .find(|r| r.id == id)
                .is_some_and(|r| r.state == OperationState::AcknowledgedUnknown)
        })
        .await;
        acknowledged = json!({"acknowledged": done});
        assert!(done, "the unknown workflow was not acknowledged");
    }

    // The operator finishes: start the guest if the interrupted workflow left it halted.
    let mut finish = json!(null);
    if c.latest().inventory().unwrap().vms[&guest_id].power_state != PowerState::Running {
        let start = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(guest_id).unwrap(),
                    action: PowerAction::Start,
                },
            )
            .expect("start accepted");
        let outcome = wait_vm_outcome(&start, 300).await;
        let running = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&guest_id].power_state == PowerState::Running)
        })
        .await;
        finish = json!({"state": format!("{:?}", outcome.state), "detail": outcome.detail, "running": running});
    }
    let running = c
        .latest()
        .inventory()
        .is_some_and(|i| i.vms[&guest_id].power_state == PowerState::Running);

    // Remove the snapshot.
    let disks = c
        .latest()
        .inventory()
        .map(|i| {
            i.details
                .vbds
                .values()
                .filter(|b| b.snapshot == Some(snapshot_id) && b.kind == "Disk")
                .filter_map(|b| b.vdi)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(guest_id, snapshot_id).unwrap(),
                disks,
            },
        )
        .expect("snapshot deletion accepted");
    let cleanup = wait_vm_outcome(&deletion, 300).await;
    lab.record(
        "interrupted-snapshot-restore",
        json!({
            "state_at_close": state_at_close,
            "recovered": {"state": format!("{:?}", record.state), "detail": record.detail, "timeline": timeline, "steps": record.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()},
            "recheck": recheck,
            "guest_power_after_recovery": power_after_recovery,
            "acknowledgement": acknowledged,
            "operator_finish": finish,
            "guest_running_at_end": running,
            "cleanup": {"state": format!("{:?}", cleanup.state), "detail": cleanup.detail},
        }),
    );
    assert!(running, "the guest is not running at the end");
    c.close().await;
}

/// P01-05 and P01-08: two saved profiles for the two pools are persisted in a profile
/// store, both are connected from their saved options at once with session-only passwords,
/// both are disconnected, one profile is removed and the store reloads with the other.
#[tokio::test]
#[ignore]
async fn lab_saved_profiles_connect_all_and_remove() {
    use xenadmin_core::{ConnectionProfile, ProfileStore};
    let lab = Lab::load();
    assert!(lab.hosts.len() >= 3, "three hosts are required");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let mut store = ProfileStore::open(state.path().join("profiles")).expect("profile store");
    let mut ids = Vec::new();
    for (index, label) in [(0usize, "Lab pool"), (2usize, "Standalone host")] {
        let mut profile = ConnectionProfile::new(label.into(), lab.endpoint(index), "root".into());
        profile
            .accept_certificate(&inspect(&lab.endpoint(index)).await)
            .expect("own certificate accepted");
        for other in 0..lab.hosts.len() {
            if other != index
                && let Some(observation) = inspect_result(&lab.endpoint(other)).await
            {
                profile
                    .accept_certificate(&observation)
                    .expect("member certificate accepted");
            }
        }
        ids.push(profile.id);
        store.save(profile).expect("profile saved");
    }
    drop(store);

    // A fresh store sees both; connect all from their saved options.
    let store = ProfileStore::open(state.path().join("profiles")).expect("profile store");
    let saved = store
        .profiles()
        .map(|p| json!({"name": p.name, "endpoint": p.endpoint.to_string()}))
        .collect::<Vec<_>>();
    assert_eq!(store.profiles().count(), 2);
    let mut connections = Vec::new();
    for profile in store.profiles() {
        let options = profile.options().expect("profile options");
        let connection = backend.connect(options, lab.credentials());
        connection
            .wait_ready(deadline(90))
            .await
            .unwrap_or_else(|e| panic!("{} did not become ready: {e}", profile.name));
        connections.push((profile.name.clone(), connection));
    }
    let pools = connections.iter().map(|(name, c)| json!({"profile": name, "pool": c.latest().inventory().unwrap().pool_id.to_string(), "hosts": c.latest().inventory().unwrap().hosts.len()})).collect::<Vec<_>>();
    let distinct = connections[0].1.latest().inventory().unwrap().pool_id
        != connections[1].1.latest().inventory().unwrap().pool_id;
    for (_, c) in &connections {
        c.close().await;
    }
    let disconnected = connections.iter().all(|(_, c)| !c.latest().is_current());
    drop(store);

    // Remove one profile; the store reloads with the other.
    let mut store = ProfileStore::open(state.path().join("profiles")).expect("profile store");
    let removed = store.remove(ids[1]).expect("remove");
    drop(store);
    let store = ProfileStore::open(state.path().join("profiles")).expect("profile store");
    let remaining = store.profiles().map(|p| p.name.clone()).collect::<Vec<_>>();
    lab.record(
        "saved-profiles-connect-all-and-remove",
        json!({"saved": saved, "connected": pools, "distinct_pools": distinct, "disconnected": disconnected, "removed": removed, "remaining": remaining}),
    );
    assert!(distinct && disconnected && removed);
    assert_eq!(remaining, vec!["Lab pool".to_string()]);
}

async fn diagnosis(handle: &xenadmin_core::ActionDiagnostic, seconds: u64) -> Value {
    tokio::time::timeout(deadline(seconds), async {
        loop {
            if let Some(result) = handle.latest() {
                return match result {
                    Ok(availability) => json!({"availability": format!("{availability:?}")}),
                    Err(error) => json!({"error": error}),
                };
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or_else(|_| json!({"error": "no diagnosis within deadline"}))
}

async fn diagnose_all(c: &Connection, vm: xenadmin_core::VmId) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for action in xenadmin_core::PowerAction::ALL {
        let entry = match c
            .latest()
            .observe_vm(vm)
            .and_then(|target| c.diagnose_power(target, action))
        {
            Ok(handle) => diagnosis(&handle, 60).await,
            Err(error) => json!({"error": error.to_string()}),
        };
        out.insert(format!("{action:?}"), entry);
    }
    out
}

fn available(diagnosis: &serde_json::Map<String, Value>, action: &str) -> bool {
    diagnosis
        .get(action)
        .and_then(|d| d["availability"].as_str())
        == Some("Available")
}

/// P03-04, P03-05, P03-06: explain every power action on the running, shutting-down and
/// halted guest; refuse a chosen-host start on the member that cannot reach the guest's
/// local disks; start the guest on the chosen resident host; set and restore the home server.
#[tokio::test]
#[ignore]
async fn lab_chosen_host_start_and_action_diagnosis() {
    use xenadmin_core::{PowerAction, PowerState, VmCommand, VmSettingEdit, VmSettingsPatch};
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
    let resident = guest.host.expect("resident host");
    let other = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != resident && inventory.hosts[h].enabled)
        .expect("another enabled member");
    let host_name = |id: HostId| {
        inventory
            .hosts
            .get(&id)
            .map(|h| h.name.clone())
            .unwrap_or_default()
    };

    // A guest that another scenario started a moment ago does not advertise a clean
    // shutdown yet, and the client rightly refuses one until the host does.
    settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.allowed_actions.contains(&PowerAction::CleanShutdown))
        })
    })
    .await;
    let while_running = diagnose_all(&c, guest.id).await;
    let shutdown = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::CleanShutdown,
            },
        )
        .expect("shutdown accepted");
    let during_shutdown = match c
        .latest()
        .observe_vm(guest.id)
        .and_then(|target| c.diagnose_power(target, PowerAction::Start))
    {
        Ok(handle) => diagnosis(&handle, 60).await,
        Err(error) => json!({"error": error.to_string()}),
    };
    let shutdown = wait_vm_outcome(&shutdown, 300).await;
    let halted = settle(180, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms.get(&guest.id).is_some_and(|v| {
                v.power_state == PowerState::Halted
                    && v.allowed_actions.contains(&PowerAction::Start)
            })
        })
    })
    .await;
    assert_eq!(
        shutdown.state,
        OperationState::Succeeded,
        "{}",
        shutdown.detail
    );
    assert!(halted, "guest did not halt");
    let while_halted = diagnose_all(&c, guest.id).await;

    // A chosen-host start on the member that cannot reach the guest's local disks.
    let refused = match c.submit(
        OperationId::new(),
        VmCommand::PowerOnHost {
            vm: c.latest().observe_vm(guest.id).unwrap(),
            action: PowerAction::Start,
            host: other,
        },
    ) {
        Ok(operation) => {
            let row = wait_vm_outcome(&operation, 300).await;
            json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
        }
        Err(error) => json!({"accepted": false, "error": error.to_string()}),
    };
    let still_halted = settle(30, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.power_state == PowerState::Halted)
        })
    })
    .await;

    // A chosen-host start on the resident host.
    let started = c
        .submit(
            OperationId::new(),
            VmCommand::PowerOnHost {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Start,
                host: resident,
            },
        )
        .expect("chosen-host start accepted");
    let started = wait_vm_outcome(&started, 300).await;
    let on_resident = settle(180, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.power_state == PowerState::Running && v.host == Some(resident))
        })
    })
    .await;

    // Home server: pin to the resident host, then restore the previous value.
    let previous_affinity = c.latest().inventory().unwrap().details.vms[&guest.id].affinity;
    let mut home = Vec::new();
    for value in [Some(resident), previous_affinity] {
        let current = c.latest().inventory().unwrap().details.vms[&guest.id].affinity;
        if current == value {
            home.push(json!({"value": value.map(|h| h.to_string()), "skipped": "already set"}));
            continue;
        }
        let patch = VmSettingsPatch::new(vec![VmSettingEdit::HomeHost {
            previous: current,
            value,
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
            .expect("home host edit accepted");
        let row = wait_vm_outcome(&operation, 120).await;
        let applied = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .vms
                    .get(&guest.id)
                    .is_some_and(|v| v.affinity == value && v.affinity_known)
            })
        })
        .await;
        home.push(json!({
            "value": value.map(|h| h.to_string()), "state": format!("{:?}", row.state),
            "detail": row.detail, "read_back": applied,
        }));
    }
    c.close().await;
    lab.record(
        "chosen-host-start-and-action-diagnosis",
        json!({
            "guest": guest.name,
            "resident": {"id": resident.to_string(), "name": host_name(resident)},
            "other": {"id": other.to_string(), "name": host_name(other)},
            "diagnosis_while_running": while_running,
            "diagnosis_during_shutdown": during_shutdown,
            "shutdown": {"state": format!("{:?}", shutdown.state), "detail": shutdown.detail},
            "diagnosis_while_halted": while_halted,
            "start_on_other_host": refused,
            "still_halted_after_refusal": still_halted,
            "start_on_resident_host": {"state": format!("{:?}", started.state), "detail": started.detail},
            "running_on_resident": on_resident,
            "home_host": home,
        }),
    );
    assert!(
        !available(&while_running, "Start"),
        "Start offered on a running guest"
    );
    assert!(available(&while_running, "CleanShutdown"));
    assert!(available(&while_halted, "Start"));
    assert!(
        !available(&while_halted, "CleanShutdown"),
        "shutdown offered on a halted guest"
    );
    assert_ne!(
        refused["state"], "Succeeded",
        "start on the other host succeeded: {refused}"
    );
    assert!(still_halted, "guest left Halted by the refused placement");
    assert_eq!(
        started.state,
        OperationState::Succeeded,
        "{}",
        started.detail
    );
    assert!(on_resident, "guest is not running on the chosen host");
    for entry in &home {
        if entry.get("skipped").is_none() {
            assert_eq!(entry["state"], "Succeeded", "{entry}");
            assert_eq!(entry["read_back"], true, "{entry}");
        }
    }
}

/// P03-03, P03-04, P04-01: suspend the running guest, refuse a chosen-host resume on the
/// member that cannot reach its local disks, resume it on the chosen resident host and
/// receive a console frame afterwards.
#[tokio::test]
#[ignore]
async fn lab_suspend_and_chosen_host_resume() {
    use xenadmin_core::{PowerAction, PowerState, VmCommand};
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
    let resident = guest.host.expect("resident host");
    let other = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != resident && inventory.hosts[h].enabled)
        .expect("another enabled member");
    let advertised = guest.allowed_actions.contains(&PowerAction::Suspend);
    let diagnosed = match c
        .latest()
        .observe_vm(guest.id)
        .and_then(|target| c.diagnose_power(target, PowerAction::Suspend))
    {
        Ok(handle) => diagnosis(&handle, 60).await,
        Err(error) => json!({"error": error.to_string()}),
    };
    let suspend = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Suspend,
            },
        )
        .expect("suspend accepted");
    let suspend = wait_vm_outcome(&suspend, 300).await;
    let suspended = settle(180, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms.get(&guest.id).is_some_and(|v| {
                v.power_state == PowerState::Suspended
                    && v.allowed_actions.contains(&PowerAction::Resume)
            })
        })
    })
    .await;
    let mut body = json!({
        "guest": guest.name,
        "resident": resident.to_string(), "other": other.to_string(),
        "suspend_advertised": advertised, "suspend_diagnosis": diagnosed,
        "suspend": {"state": format!("{:?}", suspend.state), "detail": suspend.detail},
        "suspended": suspended,
    });
    if suspend.state == OperationState::Succeeded && suspended {
        let refused = match c.submit(
            OperationId::new(),
            VmCommand::PowerOnHost {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                action: PowerAction::Resume,
                host: other,
            },
        ) {
            Ok(operation) => {
                let row = wait_vm_outcome(&operation, 300).await;
                json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
            }
            Err(error) => json!({"accepted": false, "error": error.to_string()}),
        };
        let still_suspended = settle(30, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms
                    .get(&guest.id)
                    .is_some_and(|v| v.power_state == PowerState::Suspended)
            })
        })
        .await;
        let resumed = c
            .submit(
                OperationId::new(),
                VmCommand::PowerOnHost {
                    vm: c.latest().observe_vm(guest.id).unwrap(),
                    action: PowerAction::Resume,
                    host: resident,
                },
            )
            .expect("chosen-host resume accepted");
        let resumed = wait_vm_outcome(&resumed, 300).await;
        let on_resident = settle(180, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms.get(&guest.id).is_some_and(|v| {
                    v.power_state == PowerState::Running && v.host == Some(resident)
                })
            })
        })
        .await;
        let frame = match c.latest().observe_vm(guest.id) {
            Ok(target) => match c.open_console(target) {
                Ok(console) => {
                    let got = stable_frame(&console, 60).await;
                    console.close();
                    got
                }
                Err(error) => format!("console refused: {error}"),
            },
            Err(error) => format!("stale: {error}"),
        };
        body["resume_on_other_host"] = refused.clone();
        body["still_suspended_after_refusal"] = json!(still_suspended);
        body["resume_on_resident_host"] =
            json!({"state": format!("{:?}", resumed.state), "detail": resumed.detail});
        body["running_on_resident"] = json!(on_resident);
        body["console_after_resume"] = json!(frame);
        c.close().await;
        lab.record("suspend-and-chosen-host-resume", body);
        assert_ne!(
            refused["state"], "Succeeded",
            "resume on the other host succeeded: {refused}"
        );
        assert!(
            still_suspended,
            "guest left Suspended by the refused placement"
        );
        assert_eq!(
            resumed.state,
            OperationState::Succeeded,
            "{}",
            resumed.detail
        );
        assert!(on_resident, "guest is not running on the chosen host");
        assert!(!frame.is_empty(), "no console frame after resume");
    } else {
        // Record the refusal and leave the guest as it was; a failed suspend keeps it running.
        let running = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms
                    .get(&guest.id)
                    .is_some_and(|v| v.power_state == PowerState::Running)
            })
        })
        .await;
        body["running_after_refused_suspend"] = json!(running);
        c.close().await;
        lab.record("suspend-and-chosen-host-resume", body);
        assert!(running, "guest neither suspended nor kept running");
    }
}

async fn apply_vm_edit<F: Fn(&xenadmin_core::Inventory) -> bool>(
    c: &Connection,
    vm: xenadmin_core::VmId,
    label: &str,
    edits: Vec<xenadmin_core::VmSettingEdit>,
    applied: F,
) -> Value {
    let patch = match xenadmin_core::VmSettingsPatch::new(edits) {
        Ok(patch) => patch,
        Err(error) => {
            return json!({"edit": label, "accepted": false, "error": error.to_string()});
        }
    };
    let operation = match c.latest().observe_vm(vm).and_then(|target| {
        c.submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Configure { vm: target, patch },
        )
    }) {
        Ok(operation) => operation,
        Err(error) => {
            return json!({"edit": label, "accepted": false, "error": error.to_string()});
        }
    };
    let row = wait_vm_outcome(&operation, 180).await;
    let read_back = settle(60, || c.latest().inventory().is_some_and(|i| applied(i))).await;
    json!({
        "edit": label, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
        "read_back": read_back,
        "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>(),
    })
}

/// Runs one power action on a VM and waits for the state it should reach. A guest that
/// another scenario started moments ago is not yet offered a clean shutdown or reboot by
/// its host, and the client refuses an action the host does not offer, so the action is
/// first given time to be advertised; one that never is gets submitted and refused.
async fn power_to(
    c: &Connection,
    vm: xenadmin_core::VmId,
    action: xenadmin_core::PowerAction,
    expected: xenadmin_core::PowerState,
) -> Value {
    settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&vm)
                .is_some_and(|v| v.allowed_actions.contains(&action))
        })
    })
    .await;
    if matches!(
        action,
        xenadmin_core::PowerAction::CleanShutdown | xenadmin_core::PowerAction::Reboot
    ) {
        guest_booted(c, vm).await;
    }
    let operation = c
        .submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Power {
                vm: c.latest().observe_vm(vm).unwrap(),
                action,
            },
        )
        .expect("power action accepted");
    let row = wait_vm_outcome(&operation, 300).await;
    let reached = settle(180, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&vm)
                .is_some_and(|v| v.power_state == expected && !v.allowed_actions.is_empty())
        })
    })
    .await;
    assert_eq!(
        row.state,
        OperationState::Succeeded,
        "{action:?}: {}",
        row.detail
    );
    assert!(reached, "{action:?}: guest did not reach {expected:?}");
    json!({"action": format!("{action:?}"), "state": format!("{:?}", row.state), "reached": reached})
}

/// P05-01, P05-07, P06-02, P06-03, P06-04, P06-09: on the running guest edit the CPU
/// scheduling weight and the dynamic memory range and submit a live shadow multiplier edit
/// (refused by name when the host does not advertise it); while halted raise the vCPU
/// maximum, set the CPU topology and the shadow multiplier; increase vCPUs live once the
/// host advertises it; create a snapshot with a description, edit its name and description
/// and delete it; finally restore the vCPU counts and the shadow multiplier.
#[tokio::test]
#[ignore]
async fn lab_live_property_editors_and_snapshot_metadata() {
    use xenadmin_core::{
        MemoryLimits, PowerAction, PowerState, SnapshotWorkflow, VmCommand, VmSettingEdit,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let guest = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let id = guest.id;
    let details = || c.latest().inventory().unwrap().details.vms[&id].clone();
    let mut results = Vec::new();
    let mut power = Vec::new();
    // A halted guest left by an interrupted run may carry a topology its vCPU count cannot
    // satisfy; set a single core per socket through the editor before starting it.
    if guest.power_state != PowerState::Running {
        let cores = details().cores_per_socket;
        if cores.as_deref() != Some("1") {
            results.push(
                apply_vm_edit(
                    &c,
                    id,
                    "repair: cpu topology reset while halted",
                    vec![VmSettingEdit::CpuTopology {
                        previous: cores,
                        cores_per_socket: 1,
                    }],
                    |i| i.details.vms[&id].cores_per_socket.as_deref() == Some("1"),
                )
                .await,
            );
        }
        power.push(power_to(&c, id, PowerAction::Start, PowerState::Running).await);
    }
    let advertises = |operation: &str| {
        details()
            .allowed_operations
            .is_some_and(|ops| ops.iter().any(|op| op == operation))
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Running: scheduling weight, dynamic memory range, live shadow multiplier.
    let before = details();
    let weight: u16 = if before.cpu_weight.as_deref() == Some("512") {
        256
    } else {
        512
    };
    results.push(
        apply_vm_edit(
            &c,
            id,
            "cpu weight",
            vec![VmSettingEdit::CpuWeight {
                previous: before.cpu_weight.clone(),
                weight,
            }],
            |i| i.details.vms[&id].cpu_weight.as_deref() == Some(weight.to_string().as_str()),
        )
        .await,
    );
    let restored_weight: u16 = before
        .cpu_weight
        .as_deref()
        .and_then(|w| w.parse().ok())
        .unwrap_or(256);
    results.push(
        apply_vm_edit(
            &c,
            id,
            "cpu weight restored",
            vec![VmSettingEdit::CpuWeight {
                previous: Some(weight.to_string()),
                weight: restored_weight,
            }],
            |i| {
                i.details.vms[&id].cpu_weight.as_deref()
                    == Some(restored_weight.to_string().as_str())
            },
        )
        .await,
    );
    let limits = MemoryLimits {
        static_min: before.memory_static_min.unwrap_or(0),
        dynamic_min: before.memory_dynamic_min.unwrap_or(0),
        dynamic_max: before.memory_dynamic_max.unwrap_or(0),
        static_max: before.memory_static_max.unwrap_or(0),
    };
    if limits.static_min < limits.static_max && limits.static_min > 0 {
        let mib = 1u64 << 20;
        let midpoint =
            (limits.static_min + (limits.static_max - limits.static_min) / 2) / mib * mib;
        let target_min = if limits.dynamic_min == limits.static_min {
            midpoint.max(limits.static_min)
        } else {
            limits.static_min
        };
        let target_max = limits.static_max;
        results.push(
            apply_vm_edit(
                &c,
                id,
                "dynamic memory live",
                vec![VmSettingEdit::DynamicMemory {
                    previous: limits.clone(),
                    min: target_min,
                    max: target_max,
                }],
                move |i| {
                    let v = &i.details.vms[&id];
                    v.memory_dynamic_min == Some(target_min)
                        && v.memory_dynamic_max == Some(target_max)
                },
            )
            .await,
        );
        let now = details();
        let current = MemoryLimits {
            static_min: limits.static_min,
            dynamic_min: now.memory_dynamic_min.unwrap_or(target_min),
            dynamic_max: now.memory_dynamic_max.unwrap_or(target_max),
            static_max: limits.static_max,
        };
        let (back_min, back_max) = (limits.dynamic_min, limits.dynamic_max);
        results.push(
            apply_vm_edit(
                &c,
                id,
                "dynamic memory restored",
                vec![VmSettingEdit::DynamicMemory {
                    previous: current,
                    min: back_min,
                    max: back_max,
                }],
                move |i| {
                    let v = &i.details.vms[&id];
                    v.memory_dynamic_min == Some(back_min) && v.memory_dynamic_max == Some(back_max)
                },
            )
            .await,
        );
    } else {
        results.push(json!({"edit": "dynamic memory live", "skipped": format!("static limits {}..{} leave no dynamic range", limits.static_min, limits.static_max)}));
    }
    let shadow_before = before
        .shadow_multiplier
        .clone()
        .unwrap_or_else(|| "1".into());
    let shadow_live_advertised = advertises("changing_shadow_memory_live");
    let mut live_shadow = apply_vm_edit(
        &c,
        id,
        "shadow multiplier live",
        vec![VmSettingEdit::ShadowMultiplier {
            previous: shadow_before.clone(),
            value: "2.0".into(),
            live: true,
        }],
        |i| {
            i.details.vms[&id]
                .shadow_multiplier
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok())
                == Some(2.0)
        },
    )
    .await;
    live_shadow["advertised"] = json!(shadow_live_advertised);
    live_shadow["expected"] = json!(if shadow_live_advertised {
        "Succeeded"
    } else {
        "Rejected"
    });
    results.push(live_shadow);
    if shadow_live_advertised {
        results.push(
            apply_vm_edit(
                &c,
                id,
                "shadow multiplier live restored",
                vec![VmSettingEdit::ShadowMultiplier {
                    previous: "2.0".into(),
                    value: shadow_before.clone(),
                    live: true,
                }],
                |i| {
                    i.details.vms[&id]
                        .shadow_multiplier
                        .as_deref()
                        .and_then(|v| v.parse::<f64>().ok())
                        == shadow_before.parse::<f64>().ok()
                },
            )
            .await,
        );
    }

    // Halted: raise the vCPU maximum, set the topology and the shadow multiplier.
    power.push(power_to(&c, id, PowerAction::CleanShutdown, PowerState::Halted).await);
    let halted = details();
    let (previous_max, previous_startup) = (
        halted.vcpus_max.unwrap_or(1),
        halted.vcpus_at_startup.unwrap_or(1),
    );
    let new_max = previous_max + 1;
    results.push(
        apply_vm_edit(
            &c,
            id,
            "vcpu maximum raised while halted",
            vec![VmSettingEdit::Cpus {
                previous_max,
                previous_startup,
                max: new_max,
                startup: previous_startup,
            }],
            move |i| {
                let v = &i.details.vms[&id];
                v.vcpus_max == Some(new_max) && v.vcpus_at_startup == Some(previous_startup)
            },
        )
        .await,
    );
    let cores = if halted.cores_per_socket.as_deref() == Some("1") {
        new_max
    } else {
        1
    };
    results.push(
        apply_vm_edit(
            &c,
            id,
            "cpu topology while halted",
            vec![VmSettingEdit::CpuTopology {
                previous: halted.cores_per_socket.clone(),
                cores_per_socket: cores,
            }],
            move |i| {
                i.details.vms[&id].cores_per_socket.as_deref() == Some(cores.to_string().as_str())
            },
        )
        .await,
    );
    let halted_shadow_advertised = settle(30, || advertises("changing_shadow_memory")).await;
    let mut halted_shadow = apply_vm_edit(
        &c,
        id,
        "shadow multiplier while halted",
        vec![VmSettingEdit::ShadowMultiplier {
            previous: details()
                .shadow_multiplier
                .unwrap_or_else(|| shadow_before.clone()),
            value: "2.0".into(),
            live: false,
        }],
        |i| {
            i.details.vms[&id]
                .shadow_multiplier
                .as_deref()
                .and_then(|v| v.parse::<f64>().ok())
                == Some(2.0)
        },
    )
    .await;
    halted_shadow["advertised"] = json!(halted_shadow_advertised);
    halted_shadow["expected"] = json!(if halted_shadow_advertised {
        "Succeeded"
    } else {
        "Rejected"
    });
    results.push(halted_shadow);
    power.push(power_to(&c, id, PowerAction::Start, PowerState::Running).await);
    let live_cpus_wait = std::time::Instant::now();
    let live_cpus_advertised = settle(180, || advertises("changing_VCPUs_live")).await;
    let mut live_cpus = apply_vm_edit(
        &c,
        id,
        "vcpus increased live",
        vec![VmSettingEdit::LiveCpus {
            previous: previous_startup,
            max: new_max,
            value: new_max,
        }],
        move |i| i.details.vms[&id].vcpus_at_startup == Some(new_max),
    )
    .await;
    live_cpus["advertised"] = json!(live_cpus_advertised);
    live_cpus["advertised_after_seconds"] = json!(live_cpus_wait.elapsed().as_secs());
    live_cpus["expected"] = json!(if live_cpus_advertised {
        "Succeeded"
    } else {
        "Rejected"
    });
    results.push(live_cpus);

    // Snapshot with a description, metadata edit, deletion.
    let name = format!("lab-meta-{stamp}");
    let description = "Created by the live harness with a description".to_string();
    let created = c
        .submit(
            OperationId::new(),
            VmCommand::SnapshotWorkflow {
                vm: c.latest().observe_vm(id).unwrap(),
                operation: SnapshotWorkflow::Create {
                    name: name.clone(),
                    description: description.clone(),
                    memory: false,
                },
            },
        )
        .expect("snapshot creation accepted");
    let created = wait_vm_outcome(&created, 300).await;
    let snapshot = tokio::time::timeout(deadline(60), async {
        loop {
            if let Some(s) = c.latest().inventory().and_then(|i| {
                i.snapshots
                    .for_vm(id)
                    .find(|s| s.name == name && s.description == description)
                    .cloned()
            }) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("snapshot with its description listed");
    let renamed = format!("{name}-renamed");
    let edited_description = "Edited by the live harness".to_string();
    let edit = c
        .submit(
            OperationId::new(),
            VmCommand::SnapshotWorkflow {
                vm: c.latest().observe_vm(id).unwrap(),
                operation: SnapshotWorkflow::Edit {
                    snapshot: snapshot.id,
                    expected_name: name.clone(),
                    expected_description: description.clone(),
                    name: renamed.clone(),
                    description: edited_description.clone(),
                },
            },
        )
        .expect("snapshot edit accepted");
    let edit = wait_vm_outcome(&edit, 180).await;
    let edit_read_back = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.snapshots
                .get(&snapshot.id)
                .is_some_and(|s| s.name == renamed && s.description == edited_description)
        })
    })
    .await;
    let disks = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vbds
        .values()
        .filter(|b| b.snapshot == Some(snapshot.id) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect::<Vec<_>>();
    let deleted = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(id, snapshot.id).unwrap(),
                disks: disks.clone(),
            },
        )
        .expect("snapshot deletion accepted");
    let deleted = wait_vm_outcome(&deleted, 300).await;
    let gone = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            i.snapshots.get(&snapshot.id).is_none()
                && disks.iter().all(|d| !i.details.vdis.contains_key(d))
        })
    })
    .await;

    // Restore the vCPU counts (maximum back to the startup count) and the shadow
    // multiplier from the current values, then leave the guest running.
    power.push(power_to(&c, id, PowerAction::CleanShutdown, PowerState::Halted).await);
    let current = details();
    let (current_max, current_startup) = (
        current.vcpus_max.unwrap_or(1),
        current.vcpus_at_startup.unwrap_or(1),
    );
    let current_cores = current
        .cores_per_socket
        .as_deref()
        .and_then(|c| c.parse::<u64>().ok());
    if current_cores.is_some_and(|c| previous_startup % c != 0) {
        // The editor must refuse a vCPU maximum the current topology cannot satisfy.
        let mut refused = apply_vm_edit(
            &c,
            id,
            "vcpus restored against an incompatible topology",
            vec![VmSettingEdit::Cpus {
                previous_max: current_max,
                previous_startup: current_startup,
                max: previous_startup,
                startup: previous_startup,
            }],
            move |i| i.details.vms[&id].vcpus_max == Some(previous_startup),
        )
        .await;
        refused["expected"] = json!("Rejected");
        refused["expected_detail"] = json!("cores per socket");
        results.push(refused);
    }
    if current_cores != Some(1) {
        results.push(
            apply_vm_edit(
                &c,
                id,
                "cpu topology restored while halted",
                vec![VmSettingEdit::CpuTopology {
                    previous: current.cores_per_socket.clone(),
                    cores_per_socket: 1,
                }],
                |i| i.details.vms[&id].cores_per_socket.as_deref() == Some("1"),
            )
            .await,
        );
    }
    results.push(
        apply_vm_edit(
            &c,
            id,
            "vcpus restored while halted",
            vec![VmSettingEdit::Cpus {
                previous_max: current_max,
                previous_startup: current_startup,
                max: previous_startup,
                startup: previous_startup,
            }],
            move |i| {
                let v = &i.details.vms[&id];
                v.vcpus_max == Some(previous_startup)
                    && v.vcpus_at_startup == Some(previous_startup)
            },
        )
        .await,
    );
    if let Some(current_shadow) = current
        .shadow_multiplier
        .clone()
        .filter(|s| s.parse::<f64>().ok() != shadow_before.parse::<f64>().ok())
    {
        {
            let target = shadow_before.parse::<f64>().ok();
            results.push(
                apply_vm_edit(
                    &c,
                    id,
                    "shadow multiplier restored while halted",
                    vec![VmSettingEdit::ShadowMultiplier {
                        previous: current_shadow,
                        value: shadow_before.clone(),
                        live: false,
                    }],
                    move |i| {
                        i.details.vms[&id]
                            .shadow_multiplier
                            .as_deref()
                            .and_then(|v| v.parse::<f64>().ok())
                            == target
                    },
                )
                .await,
            );
        }
    }
    power.push(power_to(&c, id, PowerAction::Start, PowerState::Running).await);
    c.close().await;
    lab.record(
        "live-property-editors-and-snapshot-metadata",
        json!({
            "guest": guest.name,
            "before": {"cpu_weight": before.cpu_weight, "shadow_multiplier": before.shadow_multiplier, "memory": limits, "vcpus_max": previous_max, "vcpus_at_startup": previous_startup, "cores_per_socket": halted.cores_per_socket, "allowed_operations": before.allowed_operations},
            "edits": results, "power": power,
            "snapshot": {
                "created": {"state": format!("{:?}", created.state), "detail": created.detail, "id": snapshot.id.to_string(), "listed_with_description": true},
                "edited": {"state": format!("{:?}", edit.state), "detail": edit.detail, "read_back": edit_read_back},
                "deleted": {"state": format!("{:?}", deleted.state), "detail": deleted.detail, "disks": disks.len(), "gone_with_disks": gone},
            },
        }),
    );
    for entry in &results {
        if entry.get("skipped").is_some() {
            continue;
        }
        assert_eq!(entry["accepted"], true, "{entry}");
        let expected = entry["expected"].as_str().unwrap_or("Succeeded");
        assert_eq!(entry["state"], expected, "{entry}");
        if expected == "Succeeded" {
            assert_eq!(entry["read_back"], true, "{entry}");
        } else {
            let needle = entry["expected_detail"]
                .as_str()
                .unwrap_or("does not advertise");
            assert!(
                entry["detail"].as_str().is_some_and(|d| d.contains(needle)),
                "refusal does not name its reason ({needle}): {entry}"
            );
        }
    }
    assert_eq!(
        created.state,
        OperationState::Succeeded,
        "{}",
        created.detail
    );
    assert_eq!(edit.state, OperationState::Succeeded, "{}", edit.detail);
    assert!(edit_read_back, "snapshot metadata edit not read back");
    assert_eq!(
        deleted.state,
        OperationState::Succeeded,
        "{}",
        deleted.detail
    );
    assert!(gone, "snapshot or its disks remain");
}

/// P03-09, P05-10: clone the guest's snapshot into a second VM, then run reviewed batch
/// power actions over both VMs: a start of both halted VMs, a clean shutdown of both running
/// VMs, a batch that names a wrong expected state (refused before any write) and a batch
/// start of the guest alone; finally delete the clone with its disks and the source snapshot.
#[tokio::test]
#[ignore]
async fn lab_batch_power_actions_over_two_vms() {
    use xenadmin_core::{
        BatchVmPower, PowerAction, PowerState, ResourceCommand, SnapshotName, VmCommand,
        VmLifecycleOperation, VmPowerBatch,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let pool = inventory.pool_id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut power = Vec::new();
    if guest.power_state != PowerState::Running {
        power.push(power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await);
    }

    // Leftovers of an earlier interrupted run go first, through the same reviewed workflows.
    let mut leftovers = Vec::new();
    for vm in inventory
        .vms
        .values()
        .filter(|v| v.name.starts_with("lab-batch-clone-") && v.id != guest.id)
        .cloned()
        .collect::<Vec<_>>()
    {
        if vm.power_state == PowerState::Running {
            power_to(&c, vm.id, PowerAction::CleanShutdown, PowerState::Halted).await;
        }
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
        .filter(|s| s.name.starts_with("lab-batch-src-"))
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

    // A disk-only snapshot cloned into a halted second VM.
    let source_name = format!("lab-batch-src-{stamp}");
    let snapshot = c
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                name: SnapshotName::new(&source_name).unwrap(),
            },
        )
        .expect("snapshot accepted");
    let snapshot = wait_vm_outcome(&snapshot, 300).await;
    assert_eq!(
        snapshot.state,
        OperationState::Succeeded,
        "{}",
        snapshot.detail
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
    .expect("source snapshot listed");
    let clone_name = format!("lab-batch-clone-{stamp}");
    let clone = c
        .submit(
            OperationId::new(),
            VmCommand::CloneSnapshot {
                snapshot: c.latest().observe_snapshot(guest.id, source).unwrap(),
                name: SnapshotName::new(&clone_name).unwrap(),
                as_template: false,
            },
        )
        .expect("clone accepted");
    let clone = wait_vm_outcome(&clone, 600).await;
    assert_eq!(clone.state, OperationState::Succeeded, "{}", clone.detail);
    let clone_vm = tokio::time::timeout(deadline(120), async {
        loop {
            if let Some(vm) = c.latest().inventory().and_then(|i| {
                i.vms
                    .values()
                    .find(|v| {
                        v.name == clone_name
                            && v.id != guest.id
                            && v.allowed_actions.contains(&PowerAction::Start)
                    })
                    .cloned()
            }) {
                return vm;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("clone visible with Start offered");
    power.push(power_to(&c, guest.id, PowerAction::CleanShutdown, PowerState::Halted).await);

    let states = |ids: &[xenadmin_core::VmId]| -> Vec<String> {
        let snapshot = c.latest();
        let i = snapshot.inventory().unwrap();
        ids.iter()
            .map(|id| {
                i.vms
                    .get(id)
                    .map(|v| format!("{:?}", v.power_state))
                    .unwrap_or_else(|| "missing".into())
            })
            .collect()
    };
    let both = [guest.id, clone_vm.id];
    let mut batches = Vec::new();
    let run_batch = |action: PowerAction,
                     items: Vec<(xenadmin_core::VmId, PowerState)>,
                     expected_after: Option<PowerState>| {
        let c = &c;
        async move {
            let ids = items.iter().map(|(id, _)| *id).collect::<Vec<_>>();
            // XAPI publishes a freshly started VM's allowed operations a few seconds after
            // its power state; wait for every VM to advertise the action, as the UI would.
            let waited = std::time::Instant::now();
            let advertised = settle(120, || {
                c.latest().inventory().is_some_and(|i| {
                    ids.iter().all(|id| {
                        i.vms
                            .get(id)
                            .is_some_and(|v| v.allowed_actions.contains(&action))
                    })
                })
            })
            .await;
            let advertised_after_seconds = waited.elapsed().as_secs();
            let before = states(&ids);
            let command = ResourceCommand::PowerBatch {
                pool: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                selection: VmPowerBatch {
                    action,
                    vms: items
                        .iter()
                        .map(|(vm, expected_state)| BatchVmPower {
                            vm: *vm,
                            expected_state: expected_state.clone(),
                        })
                        .collect(),
                },
            };
            match c.submit_resource(OperationId::new(), command) {
                Ok(operation) => {
                    let row = wait_outcome(&operation, 600).await;
                    let reached = match expected_after {
                        Some(expected) => {
                            settle(180, || {
                                c.latest().inventory().is_some_and(|i| {
                                    ids.iter().all(|id| {
                                        i.vms.get(id).is_some_and(|v| v.power_state == expected)
                                    })
                                })
                            })
                            .await
                        }
                        None => true,
                    };
                    json!({
                        "action": format!("{action:?}"), "vms": ids.len(), "accepted": true, "before": before,
                        "advertised": advertised, "advertised_after_seconds": advertised_after_seconds,
                        "state": format!("{:?}", row.state), "detail": row.detail, "reached": reached,
                        "after": states(&ids),
                        "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>(),
                    })
                }
                Err(error) => {
                    json!({"action": format!("{action:?}"), "vms": ids.len(), "accepted": false, "before": before, "error": error.to_string(), "after": states(&ids)})
                }
            }
        }
    };
    batches.push(
        run_batch(
            PowerAction::Start,
            both.iter().map(|id| (*id, PowerState::Halted)).collect(),
            Some(PowerState::Running),
        )
        .await,
    );
    batches.push(
        run_batch(
            PowerAction::CleanShutdown,
            both.iter().map(|id| (*id, PowerState::Running)).collect(),
            Some(PowerState::Halted),
        )
        .await,
    );
    // Wrong expected state for the clone: the whole selection must be refused before any write.
    let mut wrong = run_batch(
        PowerAction::Start,
        vec![
            (guest.id, PowerState::Halted),
            (clone_vm.id, PowerState::Running),
        ],
        None,
    )
    .await;
    wrong["expected"] = json!("refused");
    batches.push(wrong);
    batches.push(
        run_batch(
            PowerAction::Start,
            vec![(guest.id, PowerState::Halted)],
            Some(PowerState::Running),
        )
        .await,
    );

    // Delete the clone with its disks and the source snapshot with its disks.
    let clone_disks = disks_of(c.latest().inventory().unwrap(), clone_vm.id);
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(clone_vm.id).unwrap(),
                operation: VmLifecycleOperation::Delete {
                    expected_template: false,
                    disks: clone_disks.clone(),
                    snapshots: Vec::new(),
                },
            },
        )
        .expect("clone deletion accepted");
    let deletion = wait_vm_outcome(&deletion, 300).await;
    let clone_gone = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            !i.vms.contains_key(&clone_vm.id)
                && clone_disks.iter().all(|d| !i.details.vdis.contains_key(d))
        })
    })
    .await;
    let snapshot_disks = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vbds
        .values()
        .filter(|b| b.snapshot == Some(source) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect::<Vec<_>>();
    let snapshot_deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(guest.id, source).unwrap(),
                disks: snapshot_disks.clone(),
            },
        )
        .expect("snapshot deletion accepted");
    let snapshot_deletion = wait_vm_outcome(&snapshot_deletion, 300).await;
    let snapshot_gone = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            i.snapshots.get(&source).is_none()
                && snapshot_disks
                    .iter()
                    .all(|d| !i.details.vdis.contains_key(d))
        })
    })
    .await;
    let history = c.resource_history().len();
    c.close().await;
    lab.record(
        "batch-power-actions-over-two-vms",
        json!({
            "guest": guest.name, "clone": {"name": clone_name, "id": clone_vm.id.to_string(), "disks": clone_disks.len()},
            "leftovers": leftovers, "power": power, "batches": batches, "resource_history_records": history,
            "clone_deletion": {"state": format!("{:?}", deletion.state), "detail": deletion.detail, "gone_with_disks": clone_gone},
            "snapshot_deletion": {"state": format!("{:?}", snapshot_deletion.state), "detail": snapshot_deletion.detail, "disks": snapshot_disks.len(), "gone_with_disks": snapshot_gone},
        }),
    );
    for batch in &batches {
        if batch["expected"] == "refused" {
            assert!(
                batch["accepted"] == false || batch["state"] == "Rejected",
                "wrong expected state was not refused: {batch}"
            );
            assert_eq!(
                batch["after"], batch["before"],
                "a refused batch changed a power state: {batch}"
            );
        } else {
            assert_eq!(batch["accepted"], true, "{batch}");
            assert_eq!(batch["state"], "Succeeded", "{batch}");
            assert_eq!(batch["reached"], true, "{batch}");
        }
    }
    assert_eq!(
        deletion.state,
        OperationState::Succeeded,
        "{}",
        deletion.detail
    );
    assert!(clone_gone, "clone or its disks remain");
    assert_eq!(
        snapshot_deletion.state,
        OperationState::Succeeded,
        "{}",
        snapshot_deletion.detail
    );
    assert!(snapshot_gone, "source snapshot or its disks remain");
}

/// P14-04, P14-05, P14-02, P11-02: a second client sees the storage live migration another
/// client started in the pool-wide task inventory with its typed details and requests its
/// cancellation through the reviewed task command; both clients record the outcome and the
/// guest is verified running on one host with its disks beside it (migrated back if the
/// task completed first).
#[tokio::test]
#[ignore]
async fn lab_other_client_task_inspection_and_cancellation() {
    use xenadmin_core::{OperationControl, PowerAction, PowerState, ResourceCommand, VmCommand};
    let lab = Lab::load();
    let first_state = tempfile::tempdir().unwrap();
    let second_state = tempfile::tempdir().unwrap();
    let first_backend = Backend::with_state_directory(first_state.path().into()).unwrap();
    let second_backend = Backend::with_state_directory(second_state.path().into()).unwrap();
    let first = connect(&lab, &first_backend, 0, false).await;
    let second = connect(&lab, &second_backend, 0, false).await;
    let guest = first
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    if guest.power_state != PowerState::Running {
        power_to(&first, guest.id, PowerAction::Start, PowerState::Running).await;
    }
    // A storage move is refused while the guest has snapshots; delete the checkpoint an
    // earlier interrupted attempt left (named lab-cancel-*) through the reviewed deletion.
    let mut leftovers = Vec::new();
    for snapshot in first
        .latest()
        .inventory()
        .unwrap()
        .snapshots
        .for_vm(guest.id)
        .filter(|s| s.name.starts_with("lab-cancel-"))
        .map(|s| s.id)
        .collect::<Vec<_>>()
    {
        let inventory = first.latest().inventory().unwrap().clone();
        let disks = inventory
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(snapshot) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let deletion = first
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: first.latest().observe_snapshot(guest.id, snapshot).unwrap(),
                    disks,
                },
            )
            .expect("leftover snapshot deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        let gone = settle(60, || {
            first
                .latest()
                .inventory()
                .is_some_and(|i| i.snapshots.get(&snapshot).is_none())
        })
        .await;
        leftovers.push(json!({"snapshot": snapshot.to_string(), "state": format!("{:?}", outcome.state), "detail": outcome.detail, "gone": gone}));
    }
    let inventory = first.latest().inventory().unwrap().clone();
    let source_host = inventory.vms[&guest.id].host.expect("resident host");
    let destination = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != source_host && inventory.hosts[h].enabled)
        .expect("another enabled member");
    let plan = migration_plan(&inventory, guest.id, destination);
    let source_sr = plan.disks[0].expected_sr;
    // A freshly started guest advertises migration a few seconds after its power state.
    let migration_advertised = settle(120, || {
        first.latest().inventory().is_some_and(|i| {
            i.details.vms.get(&guest.id).is_some_and(|v| {
                v.allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "migrate_send"))
            })
        })
    })
    .await;
    let tasks_before = second
        .latest()
        .inventory()
        .unwrap()
        .details
        .tasks
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let migration = first
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: first.latest().observe_vm(guest.id).unwrap(),
                operation: plan,
            },
        )
        .expect("migration accepted");
    let started = std::time::Instant::now();
    let mut names_seen = std::collections::BTreeSet::new();
    // The other client sees the new task in the pool-wide inventory.
    let task = tokio::time::timeout(deadline(180), async {
        loop {
            if let Some(i) = second.latest().inventory() {
                for t in i
                    .details
                    .tasks
                    .values()
                    .filter(|t| !tasks_before.contains(&t.id))
                {
                    names_seen.insert(format!("{} ({})", t.name, t.status));
                }
                if let Some(task) = i
                    .details
                    .tasks
                    .values()
                    .find(|t| {
                        !tasks_before.contains(&t.id)
                            && t.name.contains("migrate_send")
                            && t.status == "pending"
                    })
                    .cloned()
                {
                    return Some(task);
                }
            }
            if migration.latest().state.terminal() {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("task wait deadline");
    let seen_after_ms = started.elapsed().as_millis() as u64;
    let typed = task.as_ref().map(|task| {
        json!({
            "id": task.id.to_string(), "name": task.name, "description": task.description, "status": task.status,
            "progress": task.progress, "created": task.created, "resident_host": task.resident_host.map(|h| h.to_string()),
            "allowed_operations": task.allowed_operations, "parent": task.parent.map(|t| t.to_string()),
            "subtasks": task.subtasks.len(), "result_type": task.result_type,
        })
    });
    let cancel = match &task {
        Some(task) => match second
            .latest()
            .observe_resource(ResourceTarget::Task(task.id))
            .and_then(|observed| {
                second.submit_resource(
                    OperationId::new(),
                    ResourceCommand::CancelTask { task: observed },
                )
            }) {
            Ok(operation) => {
                let row = wait_outcome(&operation, 300).await;
                let mut entry = json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                       "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()});
                if row.state == OperationState::OutcomeUnknown {
                    // The owning client destroys its task once it has read the result, so the
                    // canceller's verification can find nothing: recheck, then acknowledge.
                    let rechecked = second
                        .control_resource(row.id, OperationControl::Recheck)
                        .map(|_| true)
                        .unwrap_or(false);
                    let after_recheck = tokio::time::timeout(deadline(30), async {
                        loop {
                            let now = operation.latest();
                            if now.state != OperationState::OutcomeUnknown
                                && now.state != OperationState::Verifying
                            {
                                return format!("{:?}", now.state);
                            }
                            if now.state == OperationState::OutcomeUnknown
                                && rechecked
                                && now.detail != row.detail
                            {
                                return format!("{:?}", now.state);
                            }
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    })
                    .await
                    .unwrap_or_else(|_| format!("{:?}", operation.latest().state));
                    // Acknowledgement is read from the durable history, as the activity view does.
                    let history_state = || {
                        second
                            .resource_history()
                            .iter()
                            .find(|r| r.id == row.id)
                            .map(|r| r.state.clone())
                    };
                    let mut ack_errors = Vec::new();
                    let acknowledged = tokio::time::timeout(deadline(30), async {
                        loop {
                            match history_state() {
                                Some(OperationState::AcknowledgedUnknown) => return true,
                                Some(OperationState::OutcomeUnknown) => {
                                    if let Err(error) = second.control_resource(
                                        row.id,
                                        OperationControl::AcknowledgeUnknown,
                                    ) {
                                        ack_errors.push(error.to_string());
                                    }
                                }
                                Some(state) if state.terminal() => return false,
                                _ => {}
                            }
                            tokio::time::sleep(Duration::from_millis(300)).await;
                        }
                    })
                    .await
                    .unwrap_or(false);
                    let last = second
                        .resource_history()
                        .iter()
                        .find(|r| r.id == row.id)
                        .cloned()
                        .map(std::sync::Arc::new)
                        .unwrap_or_else(|| operation.latest());
                    entry["acknowledge_errors"] = json!(ack_errors);
                    entry["recheck_requested"] = json!(rechecked);
                    entry["state_after_recheck"] = json!(after_recheck);
                    entry["detail_after_recheck"] = json!(last.detail);
                    entry["acknowledged"] = json!(acknowledged);
                    entry["final_state"] = json!(format!("{:?}", last.state));
                }
                entry
            }
            Err(error) => json!({"accepted": false, "error": error.to_string()}),
        },
        None => json!({"accepted": false, "error": "task not seen before the migration finished"}),
    };
    let migration = wait_vm_outcome(&migration, 1200).await;
    let migration_elapsed_ms = started.elapsed().as_millis() as u64;
    let task_after = match &task {
        Some(task) => {
            tokio::time::timeout(deadline(120), async {
                loop {
                    let snapshot = second.latest();
                    if let Some(i) = snapshot.inventory() {
                        match i.details.tasks.get(&task.id) {
                            None => return json!({"present": false}),
                            Some(t) if t.status != "pending" => {
                                return json!({"present": true, "status": t.status, "error_code": t.error_code, "finished": t.finished, "progress": t.progress});
                            }
                            _ => {}
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            })
            .await
            .unwrap_or_else(|_| json!({"present": true, "status": "still pending after the deadline"}))
        }
        None => json!({"present": false}),
    };
    // Where is the guest now, with which disks?
    let placement = |i: &xenadmin_core::Inventory| -> Option<(HostId, Vec<xenadmin_core::SrId>)> {
        let host = i.vms.get(&guest.id)?.host?;
        let srs = i
            .details
            .vm_disks
            .get(&guest.id)?
            .iter()
            .filter_map(|b| i.details.vbds.get(b))
            .filter(|b| b.kind == "Disk")
            .filter_map(|b| i.details.vdis.get(&b.vdi?)?.sr)
            .collect();
        Some((host, srs))
    };
    let consistent = settle(300, || {
        first.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.power_state == PowerState::Running)
                && placement(i).is_some_and(|(host, srs)| {
                    srs.len() == 2
                        && ((host == source_host && srs.iter().all(|s| *s == source_sr))
                            || (host == destination && srs.iter().all(|s| *s != source_sr)))
                })
        })
    })
    .await;
    let after = placement(first.latest().inventory().unwrap());
    let mut returned = None;
    if after.as_ref().is_some_and(|(host, _)| *host == destination) {
        // The migration completed before the cancellation took effect; bring the guest back.
        let inventory = first.latest().inventory().unwrap().clone();
        let back = first
            .submit(
                OperationId::new(),
                VmCommand::Migrate {
                    vm: first.latest().observe_vm(guest.id).unwrap(),
                    operation: migration_plan(&inventory, guest.id, source_host),
                },
            )
            .expect("return migration accepted");
        let row = wait_vm_outcome(&back, 1200).await;
        let home = settle(300, || {
            first.latest().inventory().is_some_and(|i| {
                placement(i).is_some_and(|(host, srs)| {
                    host == source_host && srs.iter().all(|s| *s == source_sr)
                })
            })
        })
        .await;
        returned =
            Some(json!({"state": format!("{:?}", row.state), "detail": row.detail, "home": home}));
    }
    let second_history = second.resource_history().len();
    first.close().await;
    second.close().await;
    lab.record(
        "other-client-task-inspection-and-cancellation",
        json!({
            "guest": guest.name, "source_host": source_host.to_string(), "destination": destination.to_string(),
            "leftover_snapshots": leftovers, "migration_advertised_before_submission": migration_advertised,
            "task_seen_by_other_client": typed, "seen_after_ms": seen_after_ms, "task_names_seen": names_seen,
            "cancellation": cancel, "task_after": task_after,
            "migration": {"state": format!("{:?}", migration.state), "detail": migration.detail, "elapsed_ms": migration_elapsed_ms,
                "steps": migration.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()},
            "placement_after": after.map(|(h, srs)| json!({"host": h.to_string(), "repositories": srs.iter().map(|s| s.to_string()).collect::<Vec<_>>()})),
            "consistent": consistent, "returned": returned, "second_client_resource_records": second_history,
        }),
    );
    assert!(
        task.is_some(),
        "the other client never saw the migration task: {names_seen:?}"
    );
    assert_eq!(cancel["accepted"], true, "{cancel}");
    let settled = cancel["final_state"]
        .as_str()
        .or(cancel["state"].as_str())
        .unwrap_or("");
    assert!(
        ["Succeeded", "Rejected", "Failed", "AcknowledgedUnknown"].contains(&settled),
        "cancellation did not settle: {cancel}"
    );
    assert!(
        migration.state == OperationState::Cancelled
            || migration.state == OperationState::Succeeded,
        "{}",
        migration.detail
    );
    assert!(consistent, "guest and disks are not together on one host");
}

/// Cancels the pool's pending host tasks of one name through the client's reviewed task
/// command and reports each outcome.
async fn cancel_pending_task(c: &Connection, name: &str) -> Vec<String> {
    use xenadmin_core::ResourceCommand;
    let tasks: Vec<_> = c
        .latest()
        .inventory()
        .map(|i| {
            i.details
                .tasks
                .values()
                .filter(|t| t.name == name && t.status == "pending")
                .map(|t| t.id)
                .collect()
        })
        .unwrap_or_default();
    let mut outcomes = Vec::new();
    for task in tasks {
        let submitted = c
            .latest()
            .observe_resource(ResourceTarget::Task(task))
            .and_then(|observed| {
                c.submit_resource(
                    OperationId::new(),
                    ResourceCommand::CancelTask { task: observed },
                )
            });
        outcomes.push(match submitted {
            Ok(operation) => {
                let row = wait_outcome(&operation, 120).await;
                format!("{:?}: {}", row.state, row.detail)
            }
            Err(error) => format!("refused: {error:?}"),
        });
    }
    outcomes
}

/// Halt a running guest: a clean shutdown first, and a hard shutdown when the guest does
/// not acknowledge it (a guest sitting in its firmware after a recovery boot ignores ACPI).
async fn halt_guest(c: &Connection, vm: xenadmin_core::VmId) -> Value {
    use xenadmin_core::{PowerAction, PowerState, VmCommand};
    let advertised = settle(30, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&vm)
                .is_some_and(|v| v.allowed_actions.contains(&PowerAction::CleanShutdown))
        })
    })
    .await;
    if !advertised {
        // A clean shutdown that an earlier session left pending on the host, because the
        // guest never acted on it, is one reason for none being offered. It is cancelled
        // first, through the reviewed task command, as an operator would.
        let cancelled = cancel_pending_task(c, "Async.VM.clean_shutdown").await;
        if !cancelled.is_empty() {
            eprintln!("cancelled a pending clean shutdown before forcing: {cancelled:?}");
            settle(60, || {
                c.latest().inventory().is_some_and(|i| {
                    !i.details
                        .tasks
                        .values()
                        .any(|t| t.name == "Async.VM.clean_shutdown" && t.status == "pending")
                })
            })
            .await;
        }
        let forced = power_to(c, vm, PowerAction::HardShutdown, PowerState::Halted).await;
        return json!({"action": "HardShutdown (clean shutdown not advertised)", "cancelled_pending_shutdown": cancelled, "forced": forced, "reached": true});
    }
    guest_booted(c, vm).await;
    let shutdown = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(vm).unwrap(),
                action: PowerAction::CleanShutdown,
            },
        )
        .expect("clean shutdown accepted");
    let clean = tokio::time::timeout(deadline(150), async {
        loop {
            let row = shutdown.latest();
            if row.state.terminal() || row.state == OperationState::OutcomeUnknown {
                return Some(row);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap_or(None);
    let halted = settle(30, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&vm)
                .is_some_and(|v| v.power_state == PowerState::Halted)
        })
    })
    .await;
    if halted {
        return json!({"action": "CleanShutdown", "state": clean.map(|r| format!("{:?}", r.state)), "reached": true});
    }
    // The guest did not act on the request: one that is still starting loses it. The host
    // keeps its task open for many minutes, and the client refuses another action on the
    // guest while that one is unresolved. Cancel the host task as an operator would,
    // through the reviewed task command, and let the shutdown end before forcing.
    let cancelled = cancel_pending_task(c, "Async.VM.clean_shutdown").await;
    let ended = settle(120, || {
        let row = shutdown.latest();
        row.state.terminal() || row.state == OperationState::OutcomeUnknown
    })
    .await;
    eprintln!(
        "clean shutdown not acted on: task cancellation {cancelled:?}, operation ended {ended}"
    );
    let forced = power_to(c, vm, PowerAction::HardShutdown, PowerState::Halted).await;
    json!({"action": "CleanShutdown then HardShutdown", "clean": clean.map(|r| json!({"state": format!("{:?}", r.state), "detail": r.detail})), "forced": forced, "reached": true})
}

/// P03-08, P04-01: boot the halted guest from recovery media (CD, then network) through the
/// reviewed override, which saves the exact boot order, starts the guest and restores the
/// order after the start; capture the console while the firmware looks for media, then
/// return the guest to its normal boot.
#[tokio::test]
#[ignore]
async fn lab_recovery_boot_override_and_restore() {
    use xenadmin_core::{
        PowerAction, PowerState, RecoveryBootConfiguration, VmCommand, VmRecoveryOperation,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let guest = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let mut power = Vec::new();
    if guest.power_state == PowerState::Running {
        power.push(halt_guest(&c, guest.id).await);
    }
    let inventory = c.latest().inventory().unwrap().clone();
    let original_order = inventory.details.vms[&guest.id].boot_order.clone();
    let configuration = RecoveryBootConfiguration::from_inventory(&inventory, guest.id)
        .expect("recovery boot available on the halted HVM guest");
    let operation = c
        .submit(
            OperationId::new(),
            VmCommand::Recovery {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: VmRecoveryOperation::Boot {
                    configuration: configuration.clone(),
                },
            },
        )
        .expect("recovery boot accepted");
    let row = wait_vm_outcome(&operation, 600).await;
    let restored = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vms
                .get(&guest.id)
                .is_some_and(|v| v.boot_order == original_order)
                && i.vms
                    .get(&guest.id)
                    .is_some_and(|v| v.power_state == PowerState::Running)
        })
    })
    .await;
    let (frame, saved) = match c
        .latest()
        .observe_vm(guest.id)
        .and_then(|target| c.open_console(target))
    {
        Ok(console) => {
            // The firmware finds no bootable medium and the guest may halt within seconds;
            // take the first connected frame rather than waiting for a stable one.
            let got = tokio::time::timeout(deadline(30), async {
                loop {
                    let snapshot = console.latest();
                    if snapshot.state == xenadmin_core::ConsoleState::Connected
                        && snapshot.frame.is_some()
                    {
                        return true;
                    }
                    if matches!(snapshot.state, xenadmin_core::ConsoleState::Failed(_)) {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap_or(false);
            let digest = if got {
                console
                    .latest()
                    .frame
                    .as_ref()
                    .map(|frame| frame_digest(frame))
                    .unwrap_or_default()
            } else {
                format!("no frame: {:?}", console.latest().state)
            };
            let saved = got
                .then(|| save_frame(&lab, &console, "recovery-boot"))
                .flatten();
            console.close();
            (digest, saved)
        }
        Err(error) => (format!("console refused: {error}"), None),
    };
    // The firmware found no bootable medium, so return the guest to its normal boot.
    let halted_after =
        c.latest().inventory().unwrap().vms[&guest.id].power_state == PowerState::Halted;
    if !halted_after {
        power.push(power_to(&c, guest.id, PowerAction::HardShutdown, PowerState::Halted).await);
    }
    power.push(power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await);
    let history = c.operation_history().len();
    c.close().await;
    lab.record(
        "recovery-boot-override-and-restore",
        json!({
            "guest": guest.name, "original_order": original_order, "configuration": configuration,
            "operation": {"state": format!("{:?}", row.state), "detail": row.detail,
                "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>(),
                "restoration_offered_afterwards": row.recovery_boot_restoration().is_some()},
            "order_restored_while_running": restored, "console_frame": frame, "console_frame_file": saved,
            "guest_halted_by_itself": halted_after, "power": power, "vm_history_records": history,
        }),
    );
    assert_eq!(row.state, OperationState::Succeeded, "{}", row.detail);
    assert!(
        restored,
        "boot order was not restored to {original_order:?} on the running guest"
    );
    assert!(
        !frame.starts_with("console refused"),
        "console refused during the recovery boot: {frame}"
    );
}

async fn device_op(
    c: &Connection,
    vm: xenadmin_core::VmId,
    operation: xenadmin_core::DeviceOperation,
    seconds: u64,
) -> Value {
    match c.latest().observe_vm(vm).and_then(|target| {
        c.submit(
            OperationId::new(),
            xenadmin_core::VmCommand::Device(xenadmin_core::DeviceCommand {
                vm: target,
                operation,
            }),
        )
    }) {
        Ok(operation) => {
            let row = wait_vm_outcome(&operation, seconds).await;
            json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                   "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
        }
        Err(error) => json!({"accepted": false, "error": error.to_string()}),
    }
}

/// P07-05, P06-07, P07-11, P06-10: on the running guest request an unplug and plug of the
/// data disk attachment and set and restore the HA restart priority; while halted disable
/// changed-block tracking on the data disk (enabled beforehand outside the client) and
/// prepare, attach, verify and remove a NoCloud configuration drive; then start the guest.
#[tokio::test]
#[ignore]
async fn lab_disk_plug_cbt_ha_priority_and_cloud_init_drive() {
    use xenadmin_core::{
        CloudInitInputs, CloudInitMedia, DeviceOperation, DeviceProperty, DiskOperation,
        HaRestartPriority, OwnedDiskSource, PoolOperation, PowerAction, PowerState,
        ResourceCommand, ReviewedDisk,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let guest = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let id = guest.id;
    let mut power = Vec::new();
    if guest.power_state != PowerState::Running {
        power.push(power_to(&c, id, PowerAction::Start, PowerState::Running).await);
    }
    let data_attachment = |i: &xenadmin_core::Inventory| {
        i.details
            .vbds
            .values()
            .find(|b| {
                b.vm == Some(id)
                    && b.kind == "Disk"
                    && b.vdi
                        .and_then(|v| i.details.vdis.get(&v))
                        .is_some_and(|v| v.name.starts_with("lab-guest-data-"))
            })
            .cloned()
    };
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let resident = inventory.vms[&id].host.expect("resident host");
    let data = data_attachment(&inventory).expect("data disk attachment");
    let data_vdi = data.vdi.expect("data disk");
    let mut results = Vec::new();

    // Running: unplug and plug the data disk attachment.
    let before = json!({"currently_attached": data.currently_attached, "allowed_operations": data.allowed_operations});
    let unplug = device_op(&c, id, DeviceOperation::UnplugDisk { vbd: data.id }, 180).await;
    let detached = settle(30, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vbds
                .get(&data.id)
                .is_some_and(|b| b.currently_attached == Some(false))
        })
    })
    .await;
    results.push(json!({"step": "unplug data disk while running", "attachment_before": before, "outcome": unplug, "detached": detached}));
    if detached {
        let plug = device_op(&c, id, DeviceOperation::PlugDisk { vbd: data.id }, 180).await;
        let attached = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .vbds
                    .get(&data.id)
                    .is_some_and(|b| b.currently_attached == Some(true))
            })
        })
        .await;
        results.push(
            json!({"step": "plug data disk while running", "outcome": plug, "attached": attached}),
        );
    }

    // Running: HA restart priority set and restored through the pool workflow.
    let current = c.latest().inventory().unwrap().details.vms[&id]
        .ha_restart_priority
        .clone()
        .unwrap_or_default();
    let expected = HaRestartPriority::parse(&current);
    let mut ha = Vec::new();
    if let Some(expected) = expected {
        let target = if expected == HaRestartPriority::BestEffort {
            HaRestartPriority::Restart
        } else {
            HaRestartPriority::BestEffort
        };
        for (label, from, to) in [("set", expected, target), ("restore", target, expected)] {
            let entry = match c
                .latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .and_then(|observed| {
                    c.submit_resource(
                        OperationId::new(),
                        ResourceCommand::Pool {
                            target: observed,
                            operation: PoolOperation::SetVmRestartPriority {
                                vm: id,
                                expected: from,
                                priority: to,
                            },
                        },
                    )
                }) {
                Ok(operation) => {
                    let row = wait_outcome(&operation, 180).await;
                    let wire = match to {
                        HaRestartPriority::Disabled => "",
                        HaRestartPriority::BestEffort => "best-effort",
                        HaRestartPriority::Restart => "restart",
                    };
                    let read_back = settle(60, || {
                        c.latest().inventory().is_some_and(|i| {
                            i.details.vms.get(&id).is_some_and(|v| {
                                v.ha_restart_priority.as_deref().unwrap_or_default() == wire
                            })
                        })
                    })
                    .await;
                    json!({"step": label, "to": format!("{to:?}"), "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": read_back})
                }
                Err(error) => {
                    json!({"step": label, "to": format!("{to:?}"), "accepted": false, "error": error.to_string()})
                }
            };
            let stop = entry["accepted"] == false || entry["state"] != "Succeeded";
            ha.push(entry);
            if stop {
                break;
            }
        }
    } else {
        ha.push(json!({"skipped": format!("unrecognised priority {current:?}")}));
    }

    // Halted: disable changed-block tracking, then the cloud-init drive.
    power.push(halt_guest(&c, id).await);
    let cbt_before = c.latest().inventory().unwrap().details.vdis[&data_vdi].cbt_enabled;
    let cbt = if cbt_before == Some(true) {
        let outcome = device_op(
            &c,
            id,
            DeviceOperation::Property {
                change: DeviceProperty::DisableCbt {
                    vbd: data.id,
                    vdi: data_vdi,
                },
            },
            180,
        )
        .await;
        let disabled = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .vdis
                    .get(&data_vdi)
                    .is_some_and(|v| v.cbt_enabled == Some(false))
            })
        })
        .await;
        json!({"enabled_before": true, "outcome": outcome, "disabled": disabled})
    } else {
        json!({"enabled_before": cbt_before, "skipped": "changed-block tracking was not enabled on the data disk"})
    };
    let sr =
        local_sr_of(c.latest().inventory().unwrap(), resident).expect("resident local repository");
    let media = CloudInitMedia::prepare(
        id,
        CloudInitInputs::new(
            "lab-guest".into(),
            "#cloud-config\nhostname: lab-guest\n".into(),
            None,
        )
        .expect("cloud-init inputs"),
    )
    .expect("cloud-init media");
    let cloud_id = OperationId::new();
    let cloud = match c
        .latest()
        .observe_vm(id)
        .and_then(|target| c.prepare_cloud_init(cloud_id, target, sr, "2".into(), media))
    {
        Ok(operation) => {
            let row = wait_outcome(&operation, 600).await;
            json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                   "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
        }
        Err(error) => json!({"accepted": false, "error": error.to_string()}),
    };
    let drive = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vbds
                .values()
                .any(|b| b.vm == Some(id) && b.userdevice == "2" && b.vdi.is_some())
        })
    })
    .await
    .then(|| {
        let i = c.latest();
        let i = i.inventory().unwrap();
        i.details
            .vbds
            .values()
            .find(|b| b.vm == Some(id) && b.userdevice == "2" && b.vdi.is_some())
            .map(|b| {
                let vdi = i.details.vdis.get(&b.vdi.unwrap());
                (b.clone(), vdi.cloned())
            })
    })
    .flatten();
    let drive_json = drive.as_ref().map(|(b, v)| {
        json!({"vbd": b.id.to_string(), "mode": b.mode, "kind": b.kind, "owned": b.owned, "bootable": b.bootable,
               "vdi": v.as_ref().map(|v| json!({"id": v.id.to_string(), "name": v.name, "size": v.virtual_size, "read_only": v.read_only, "sr": v.sr.map(|s| s.to_string())}))})
    });
    let mut removal = Vec::new();
    if let Some((vbd, Some(vdi))) = &drive {
        let delete = device_op(
            &c,
            id,
            DeviceOperation::DeleteDisk {
                vbd: vbd.id,
                vdi: vdi.id,
            },
            180,
        )
        .await;
        let gone = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| !i.details.vdis.contains_key(&vdi.id))
        })
        .await;
        removal.push(json!({"step": "delete drive through the owning attachment", "outcome": delete, "gone": gone}));
        if !gone {
            let detach = device_op(
                &c,
                id,
                DeviceOperation::DetachDisk {
                    vbd: vbd.id,
                    vdi: vdi.id,
                },
                180,
            )
            .await;
            let detached = settle(60, || {
                c.latest()
                    .inventory()
                    .is_some_and(|i| !i.details.vbds.contains_key(&vbd.id))
            })
            .await;
            removal.push(json!({"step": "detach drive", "outcome": detach, "detached": detached}));
            let cleanup = match ReviewedDisk::from_inventory(
                c.latest().inventory().unwrap(),
                vdi.id,
            ) {
                Some(reviewed) => match c
                    .latest()
                    .observe_resource(ResourceTarget::Disk(vdi.id))
                    .and_then(|target| {
                        c.submit_resource(
                            OperationId::new(),
                            ResourceCommand::Disk {
                                target,
                                operation: DiskOperation::Cleanup {
                                    expected: reviewed,
                                    source: OwnedDiskSource::ResourceOperation(cloud_id),
                                },
                            },
                        )
                    }) {
                    Ok(operation) => {
                        let row = wait_outcome(&operation, 300).await;
                        json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
                    }
                    Err(error) => json!({"accepted": false, "error": error.to_string()}),
                },
                None => json!({"accepted": false, "error": "disk could not be reviewed"}),
            };
            let gone = settle(60, || {
                c.latest()
                    .inventory()
                    .is_some_and(|i| !i.details.vdis.contains_key(&vdi.id))
            })
            .await;
            removal.push(json!({"step": "owned-disk cleanup", "outcome": cleanup, "gone": gone}));
        }
    }
    power.push(power_to(&c, id, PowerAction::Start, PowerState::Running).await);
    let drive_remains = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vbds
        .values()
        .any(|b| b.vm == Some(id) && b.userdevice == "2");
    c.close().await;
    lab.record(
        "disk-plug-cbt-ha-priority-and-cloud-init-drive",
        json!({
            "guest": guest.name, "attachment": results, "ha_priority": {"before": current, "steps": ha},
            "changed_block_tracking": cbt, "cloud_init": {"operation": cloud, "drive": drive_json, "removal": removal, "drive_remains": drive_remains},
            "power": power,
        }),
    );
    for entry in &ha {
        if entry.get("skipped").is_none() {
            assert_eq!(entry["accepted"], true, "{entry}");
            assert_eq!(entry["state"], "Succeeded", "{entry}");
            assert_eq!(entry["read_back"], true, "{entry}");
        }
    }
    if cbt.get("skipped").is_none() {
        assert_eq!(cbt["outcome"]["state"], "Succeeded", "{cbt}");
        assert_eq!(cbt["disabled"], true, "{cbt}");
    }
    assert_eq!(cloud["accepted"], true, "{cloud}");
    assert_eq!(cloud["state"], "Succeeded", "{cloud}");
    assert!(
        drive_json.is_some(),
        "cloud-init drive not visible at slot 2"
    );
    assert!(
        !drive_remains,
        "cloud-init drive remains attached: {removal:?}"
    );
}

async fn resource_op(
    c: &Connection,
    label: &str,
    command: xenadmin_core::ResourceCommand,
    seconds: u64,
) -> Value {
    match c.submit_resource(OperationId::new(), command) {
        Ok(operation) => {
            let row = wait_outcome(&operation, seconds).await;
            json!({"step": label, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                   "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
        }
        Err(error) => json!({"step": label, "accepted": false, "error": error.to_string()}),
    }
}

/// P12-02, P12-13, P12-09, P13-07, P10-10, P04-12, P04-03: edit and restore a member's name
/// and description and its syslog destination, the pool's VM autostart, migration
/// compression and IGMP snooping settings, each verified on read-back; open the
/// coordinator's control-domain text console; send Ctrl+Alt+Delete to the guest console and
/// watch the guest reboot in place.
#[tokio::test]
#[ignore]
async fn lab_host_and_pool_settings_round_trip() {
    use xenadmin_core::{
        ConsoleInput, HostOperation, NetworkOperation, PoolOperation, PowerState, ResourceCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("an enabled member");
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let mut results = Vec::new();
    let host_target = |h: HostId| {
        c.latest()
            .observe_resource(ResourceTarget::Host(h))
            .unwrap()
    };
    let pool_target = || {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };

    // Member name and description, then restored exactly.
    let name = inventory.hosts[&member].name.clone();
    let description = inventory.details.hosts[&member].description.clone();
    let renamed = format!("{name}-renamed");
    let mut entry = resource_op(
        &c,
        "rename member",
        ResourceCommand::RenameMetadata {
            target: host_target(member),
            expected_name: name.clone(),
            expected_description: description.clone(),
            name: renamed.clone(),
            description: "Renamed by the live harness".into(),
        },
        180,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| i
            .hosts
            .get(&member)
            .is_some_and(|h| h.name == renamed)
            && i.details.hosts.get(&member).is_some_and(
                |h| h.description == "Renamed by the live harness"
            )))
        .await
    );
    results.push(entry);
    let mut entry = resource_op(
        &c,
        "restore member name",
        ResourceCommand::RenameMetadata {
            target: host_target(member),
            expected_name: renamed.clone(),
            expected_description: "Renamed by the live harness".into(),
            name: name.clone(),
            description: description.clone(),
        },
        180,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| i
            .hosts
            .get(&member)
            .is_some_and(|h| h.name == name)
            && i.details
                .hosts
                .get(&member)
                .is_some_and(|h| h.description == description)))
        .await
    );
    results.push(entry);

    // Member syslog destination, then restored.
    let syslog_before = inventory.details.hosts[&member].syslog_destination.clone();
    let destination = Some(lab.hosts[0].ip.clone());
    let mut entry = resource_op(
        &c,
        "set syslog destination",
        ResourceCommand::Host {
            target: host_target(member),
            operation: HostOperation::SetSyslog {
                expected_destination: syslog_before.clone(),
                destination: destination.clone(),
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| i
            .details
            .hosts
            .get(&member)
            .is_some_and(|h| h.syslog_destination == destination)))
        .await
    );
    results.push(entry);
    let mut entry = resource_op(
        &c,
        "restore syslog destination",
        ResourceCommand::Host {
            target: host_target(member),
            operation: HostOperation::SetSyslog {
                expected_destination: destination.clone(),
                destination: syslog_before.clone(),
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| i
            .details
            .hosts
            .get(&member)
            .is_some_and(|h| h.syslog_destination == syslog_before)))
        .await
    );
    results.push(entry);

    // Pool VM autostart, migration compression and IGMP snooping, each toggled and restored.
    let pool_details = |i: &xenadmin_core::Inventory| i.details.pool.clone();
    let autostart = pool_details(&inventory)
        .and_then(|p| p.auto_poweron)
        .unwrap_or(false);
    for (label, expected, enabled) in [
        ("enable pool autostart", autostart, !autostart),
        ("restore pool autostart", !autostart, autostart),
    ] {
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Host {
                target: pool_target(),
                operation: HostOperation::SetPoolAutostart { expected, enabled },
            },
            180,
        )
        .await;
        entry["read_back"] = json!(
            settle(60, || c.latest().inventory().is_some_and(|i| pool_details(
                i
            )
            .and_then(|p| p.auto_poweron)
                == Some(enabled)))
            .await
        );
        results.push(entry);
    }
    let compression = pool_details(&inventory)
        .and_then(|p| p.administration.migration_compression)
        .unwrap_or(false);
    for (label, expected, enabled) in [
        ("toggle migration compression", compression, !compression),
        ("restore migration compression", !compression, compression),
    ] {
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Pool {
                target: pool_target(),
                operation: PoolOperation::SetMigrationCompression { expected, enabled },
            },
            180,
        )
        .await;
        entry["read_back"] = json!(
            settle(60, || c.latest().inventory().is_some_and(|i| pool_details(
                i
            )
            .and_then(|p| p.administration.migration_compression)
                == Some(enabled)))
            .await
        );
        results.push(entry);
    }
    let snooping = pool_details(&inventory)
        .and_then(|p| p.igmp_snooping_enabled)
        .unwrap_or(false);
    for (label, expected, enabled) in [
        ("toggle igmp snooping", snooping, !snooping),
        ("restore igmp snooping", !snooping, snooping),
    ] {
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Network {
                target: pool_target(),
                operation: NetworkOperation::SetIgmpSnooping { expected, enabled },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(60, || c.latest().inventory().is_some_and(|i| pool_details(
                i
            )
            .and_then(|p| p.igmp_snooping_enabled)
                == Some(enabled)))
            .await
        );
        results.push(entry);
    }

    // The coordinator's control-domain text console.
    let control_domain = inventory.details.hosts[&coordinator]
        .control_domain
        .expect("control domain");
    let host_console = match c.open_host_console(host_target(coordinator), control_domain) {
        Ok(console) => {
            let got = tokio::time::timeout(deadline(60), async {
                loop {
                    let snapshot = console.latest();
                    if snapshot.state == xenadmin_core::ConsoleState::Connected
                        && snapshot.frame.is_some()
                    {
                        return Ok(());
                    }
                    if let xenadmin_core::ConsoleState::Failed(error) = &snapshot.state {
                        return Err(error.clone());
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap_or(Err("no frame within the deadline".into()));
            // RFB paints the first frame in rectangles; wait for a frame that repeats.
            let digest = stable_frame_or_last(&console, 20).await;
            let saved = got
                .is_ok()
                .then(|| save_frame(&lab, &console, "host-console"))
                .flatten();
            let size = console.latest().frame.as_ref().map(|f| (f.width, f.height));
            console.close();
            json!({"opened": true, "frame": got.is_ok(), "error": got.err(), "size": size, "digest": digest, "file": saved})
        }
        Err(error) => json!({"opened": false, "error": error.to_string()}),
    };

    // Ctrl+Alt+Delete through the guest console: the guest reboots in place.
    let metrics_start = |i: &xenadmin_core::Inventory| {
        i.details
            .vms
            .get(&guest.id)
            .and_then(|v| v.metrics)
            .and_then(|m| i.details.vm_metrics.get(&m))
            .and_then(|m| m.start_time.clone())
    };
    let start_before = metrics_start(&inventory);
    let chord = match c
        .latest()
        .observe_vm(guest.id)
        .and_then(|target| c.open_console(target))
    {
        Ok(console) => {
            let ready = tokio::time::timeout(deadline(60), async {
                loop {
                    let snapshot = console.latest();
                    if snapshot.state == xenadmin_core::ConsoleState::Connected
                        && snapshot.frame.is_some()
                    {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap_or(false);
            let sent = ready
                && console
                    .input(ConsoleInput::KeyChord(vec![0xffe3, 0xffe9, 0xffff]))
                    .is_ok();
            let at = std::time::Instant::now();
            let rebooted = sent
                && settle(180, || {
                    c.latest().inventory().is_some_and(|i| {
                        metrics_start(i).is_some() && metrics_start(i) != start_before
                    })
                })
                .await;
            let running_again = settle(180, || {
                c.latest().inventory().is_some_and(|i| {
                    i.vms.get(&guest.id).is_some_and(|v| {
                        v.power_state == PowerState::Running && !v.allowed_actions.is_empty()
                    })
                })
            })
            .await;
            let frame_after = stable_frame_or_last(&console, 30).await;
            let saved = save_frame(&lab, &console, "after-ctrl-alt-del");
            console.close();
            json!({"console_ready": ready, "chord_sent": sent, "start_time_before": start_before, "start_time_after": metrics_start(c.latest().inventory().unwrap()),
                   "rebooted_in_place": rebooted, "seconds": at.elapsed().as_secs(), "running_again": running_again, "frame_after": frame_after, "file": saved})
        }
        Err(error) => json!({"console_ready": false, "error": error.to_string()}),
    };
    c.close().await;
    lab.record(
        "host-and-pool-settings-round-trip",
        json!({
            "coordinator": coordinator.to_string(), "member": member.to_string(), "guest": guest.name,
            "settings": results, "host_console": host_console, "ctrl_alt_delete": chord,
        }),
    );
    for entry in &results {
        assert_eq!(entry["accepted"], true, "{entry}");
        assert_eq!(entry["state"], "Succeeded", "{entry}");
        assert_eq!(entry["read_back"], true, "{entry}");
    }
    assert_eq!(host_console["frame"], true, "{host_console}");
    assert_eq!(chord["chord_sent"], true, "{chord}");
    assert_eq!(chord["running_again"], true, "{chord}");
}

/// The digest of the first frame that repeats, or of the last frame seen within the bound.
async fn stable_frame_or_last(console: &xenadmin_core::Console, seconds: u64) -> String {
    let mut last = String::new();
    let _ = tokio::time::timeout(deadline(seconds), async {
        loop {
            if let Some(frame) = console.latest().frame.as_ref() {
                let digest = frame_digest(frame);
                if digest == last {
                    return;
                }
                last = digest;
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    })
    .await;
    last
}

/// P13-04, P01-02, P01-04: hand the coordinator role to the member through the reviewed
/// handover, watch the live session follow the host's coordinator redirect to the new
/// coordinator under a new generation, log in through the former coordinator's address as
/// a member alias, then hand the role back and follow it home; the guest is untouched.
#[tokio::test]
#[ignore]
async fn lab_coordinator_handover_round_trip() {
    use xenadmin_core::{
        ConnectionState, OperationControl, PoolOperation, PowerState, ResourceCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator_of = |i: &xenadmin_core::Inventory| {
        i.details
            .pool
            .as_ref()
            .and_then(|p| p.administration.coordinator)
    };
    let current = coordinator_of(&inventory).expect("coordinator");
    // The round trip starts and ends with the lab's first host as coordinator; an earlier
    // interrupted run may have left the role elsewhere, so return it first.
    let home = inventory
        .hosts
        .values()
        .find(|h| h.address == lab.hosts[0].ip)
        .map(|h| h.id)
        .expect("first lab host");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != home && inventory.hosts[h].enabled)
        .expect("an enabled member");
    let address_of = |h: HostId| inventory.hosts[&h].address.clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let guest_host = guest.host;
    let endpoint_now = || c.latest().active_endpoint().map(|e| e.to_string());
    let before = json!({"coordinator": home.to_string(), "member": member.to_string(), "endpoint": endpoint_now(), "generation": c.latest().generation()});

    let mut legs = Vec::new();
    let mut plan = Vec::new();
    if current != home {
        plan.push(("return the role to the first host", current, home));
    }
    plan.extend([
        ("handover to member", home, member),
        ("handover back", member, home),
    ]);
    for (label, from, to) in plan {
        let target_address = address_of(to);
        let operation = c
            .latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .and_then(|target| {
                c.submit_resource(
                    OperationId::new(),
                    ResourceCommand::Pool {
                        target,
                        operation: PoolOperation::DesignateCoordinator {
                            expected: from,
                            host: to,
                        },
                    },
                )
            });
        let started = std::time::Instant::now();
        let followed_now = |c: &Connection| {
            let snapshot = c.latest();
            snapshot.state() == ConnectionState::Ready
                && snapshot
                    .active_endpoint()
                    .is_some_and(|e| e.to_string().contains(&target_address))
                && snapshot
                    .inventory()
                    .is_some_and(|i| coordinator_of(i) == Some(to))
        };
        let rejected =
            |operation: &Result<xenadmin_core::ResourceOperation, xenadmin_core::Error>| {
                operation.as_ref().map_or(true, |o| {
                    matches!(
                        o.latest().state,
                        OperationState::Rejected | OperationState::Failed
                    )
                })
            };
        let (timeline, _) =
            state_timeline(&c, 420, |c| followed_now(c) || rejected(&operation)).await;
        let followed = followed_now(&c);
        let outcome = match &operation {
            Ok(operation) => {
                let row = wait_outcome(operation, 120).await;
                json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                       "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
            }
            Err(error) => json!({"accepted": false, "error": error.to_string()}),
        };
        // The session is lost to the redirect while the handover is verified, so the
        // record is left unknown; after reconnecting, the operator's recheck confirms it.
        let rechecked = match &operation {
            Ok(operation) if operation.latest().state == OperationState::OutcomeUnknown => {
                let id = operation.latest().id;
                let settled_first = settle(240, || {
                    c.latest()
                        .inventory()
                        .is_some_and(|i| i.hosts.values().all(|h| h.enabled))
                })
                .await;
                let requested = c
                    .control_resource(id, OperationControl::Recheck)
                    .map(|_| true)
                    .map_err(|e| e.to_string());
                let final_state = tokio::time::timeout(deadline(180), async {
                    loop {
                        if let Some(r) = c.resource_history().iter().find(|r| r.id == id)
                            && (r.state.terminal()
                                || r.state == OperationState::AcknowledgedUnknown)
                        {
                            return json!({"state": format!("{:?}", r.state), "detail": r.detail});
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| json!({"state": "still unresolved after the deadline"}));
                json!({"hosts_enabled_first": settled_first, "requested": requested.is_ok(), "error": requested.err(), "after": final_state})
            }
            _ => json!({"skipped": "record was not unknown"}),
        };
        // Both toolstacks restart around a handover; let the pool settle before judging it.
        let settled = settle(240, || {
            c.latest().inventory().is_some_and(|i| {
                i.hosts.values().all(|h| h.enabled)
                    && coordinator_of(i).is_some()
                    && i.vms
                        .get(&guest.id)
                        .is_some_and(|v| v.power_state == PowerState::Running)
            })
        })
        .await;
        tokio::time::sleep(Duration::from_secs(15)).await;
        let after = c.latest();
        let guest_untouched = after.inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.power_state == PowerState::Running && v.host == guest_host)
                && i.hosts.values().all(|h| h.enabled)
        });
        legs.push(json!({
            "leg": label, "from": from.to_string(), "to": to.to_string(), "operation": outcome,
            "followed_within_seconds": started.elapsed().as_secs(), "followed": followed, "settled": settled, "rechecked": rechecked, "timeline": timeline,
            "endpoint_after": endpoint_now(), "generation_after": after.generation(),
            "coordinator_after": after.inventory().and_then(|i| coordinator_of(i)).map(|h| h.to_string()),
            "guest_untouched_and_hosts_enabled": guest_untouched,
        }));
        if label == "handover to member" {
            // The former coordinator's address now answers as a member alias.
            let alias_state = tempfile::tempdir().unwrap();
            let alias_backend = Backend::with_state_directory(alias_state.path().into()).unwrap();
            let alias = connect(&lab, &alias_backend, 0, true).await;
            let snapshot = alias.latest();
            legs.push(json!({
                "leg": "member alias login through the former coordinator",
                "requested": lab.hosts[0].ip, "endpoint": snapshot.active_endpoint().map(|e| e.to_string()),
                "coordinator": snapshot.inventory().and_then(|i| coordinator_of(i)).map(|h| h.to_string()),
                "state": format!("{:?}", snapshot.state()),
            }));
            alias.close().await;
        }
    }
    c.close().await;
    lab.record(
        "coordinator-handover-round-trip",
        json!({"before": before, "legs": legs}),
    );
    for leg in &legs {
        if leg.get("followed").is_some() {
            assert_eq!(leg["followed"], true, "{}", leg["leg"]);
            assert_eq!(
                leg["guest_untouched_and_hosts_enabled"], true,
                "{}",
                leg["leg"]
            );
            let final_state = leg["rechecked"]["after"]["state"]
                .as_str()
                .or(leg["operation"]["state"].as_str());
            assert_eq!(
                final_state,
                Some("Succeeded"),
                "{}: {}",
                leg["leg"],
                leg["rechecked"]
            );
        }
    }
}

/// P13-12, P01-14, P01-01: reset the member's self-signed certificate through the reviewed
/// pool security operation, observe the new fingerprint, show that a login pinned to the
/// previous fingerprint is refused by TLS verification before any credential is sent, and
/// log in again once the new fingerprint is pinned.
#[tokio::test]
#[ignore]
async fn lab_member_certificate_reset_and_trust_replacement() {
    use xenadmin_core::{ConnectionState, PoolOperation, PoolSecurityOperation, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member_index = (0..lab.hosts.len())
        .find(|i| {
            inventory
                .hosts
                .values()
                .any(|h| h.id != coordinator && h.address == lab.hosts[*i].ip)
        })
        .expect("member endpoint");
    let member = inventory
        .hosts
        .values()
        .find(|h| h.address == lab.hosts[member_index].ip)
        .map(|h| h.id)
        .unwrap();
    let endpoint = lab.endpoint(member_index);
    let old = inspect(&endpoint).await.details.fingerprint.clone();
    let reset = match c
        .latest()
        .observe_resource(ResourceTarget::Pool(pool))
        .and_then(|target| {
            c.submit_resource(
                OperationId::new(),
                ResourceCommand::Pool {
                    target,
                    operation: PoolOperation::Security(PoolSecurityOperation::ResetCertificate {
                        host: member,
                        expected: old.clone(),
                    }),
                },
            )
        }) {
        Ok(operation) => {
            let row = wait_outcome(&operation, 300).await;
            json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                   "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
        }
        Err(error) => json!({"accepted": false, "error": error.to_string()}),
    };
    let new = tokio::time::timeout(deadline(120), async {
        loop {
            if let Some(observation) = inspect_result(&endpoint).await
                && observation.details.fingerprint != old
            {
                return observation.details.fingerprint;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .ok();

    // A login pinned to the previous fingerprint must fail TLS verification.
    let mut stale = ConnectionOptions::new(endpoint.clone());
    stale.certificate_fingerprint = Some(old.clone());
    stale.read_only = true;
    let stale_connection = backend.connect(stale, lab.credentials());
    let (stale_timeline, stale_failed) = state_timeline(&stale_connection, 90, |c| {
        let snapshot = c.latest();
        snapshot.state() == ConnectionState::Failed
            || snapshot
                .error()
                .is_some_and(|e| e.contains("TLS verification failed"))
    })
    .await;
    let stale_error = stale_connection.latest().error().map(str::to_string);
    stale_connection.close().await;

    // The same login succeeds once the new fingerprint is pinned (the member redirects to
    // the coordinator, whose pin the harness supplies as well).
    let replaced = match &new {
        Some(_) => {
            let fresh_state = tempfile::tempdir().unwrap();
            let fresh_backend = Backend::with_state_directory(fresh_state.path().into()).unwrap();
            let fresh = connect(&lab, &fresh_backend, member_index, true).await;
            let snapshot = fresh.latest();
            let entry = json!({"state": format!("{:?}", snapshot.state()), "endpoint": snapshot.active_endpoint().map(|e| e.to_string())});
            fresh.close().await;
            entry
        }
        None => json!({"skipped": "no new fingerprint observed"}),
    };
    c.close().await;
    lab.record(
        "member-certificate-reset-and-trust-replacement",
        json!({
            "member": member.to_string(), "endpoint": endpoint.to_string(),
            "fingerprint_before": old.to_string(), "fingerprint_after": new.as_ref().map(|f| f.to_string()),
            "reset": reset, "stale_pin": {"failed": stale_failed, "error": stale_error, "timeline": stale_timeline},
            "new_pin": replaced,
        }),
    );
    assert_eq!(reset["accepted"], true, "{reset}");
    assert_eq!(reset["state"], "Succeeded", "{reset}");
    assert!(new.is_some(), "the member's fingerprint did not change");
    assert!(
        stale_failed,
        "login with the previous pin did not fail: {stale_timeline:?}"
    );
    assert_eq!(replaced["state"], "Ready", "{replaced}");
}

/// P12-12, P12-14, P08-01, P08-04, P08-05, P08-07: download the member's private
/// configuration backup and a support bundle of its advertised categories (both removed
/// again once measured); with the guest halted, fast-clone it into a custom template, edit
/// the template's metadata, full-copy a VM from the template onto the resident host's local
/// repository, then delete the VM and the template with their disks and start the guest.
#[tokio::test]
#[ignore]
async fn lab_host_archives_and_template_lifecycle() {
    use xenadmin_core::{
        HostArchiveOperation, PowerAction, PowerState, TransferAction, TransferFile,
        TransferRequest, VmCommand, VmCopyMode, VmLifecycleOperation,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("member");
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let resident = guest.host.expect("resident host");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let directory = lab.evidence.parent().unwrap().join("transfers");
    std::fs::create_dir_all(&directory).unwrap();

    // Host support overview, then the private backup and a support bundle.
    let overview = match c
        .latest()
        .observe_resource(ResourceTarget::Host(member))
        .and_then(|o| c.host_support(o))
    {
        Ok(query) => tokio::time::timeout(deadline(120), async {
            loop {
                if let Some(result) = query.latest() {
                    return result.map_err(|e| e.to_string());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .unwrap_or(Err("no overview within the deadline".into())),
        Err(error) => Err(error.to_string()),
    };
    let overview_json = match &overview {
        Ok(o) => {
            json!({"version": o.version, "capabilities": o.capabilities.iter().map(|c| json!({"key": c.key, "privacy": c.privacy, "default": c.default_selected})).collect::<Vec<_>>(),
                        "capabilities_error": o.capabilities_error, "crashdumps": o.crashdumps.len(), "crashdumps_error": o.crashdumps_error})
        }
        Err(e) => json!({"error": e}),
    };
    let version = inventory.hosts[&member].version.clone();
    let mut archives = Vec::new();
    let entries = overview
        .as_ref()
        .ok()
        .map(|o| {
            let selected = o
                .capabilities
                .iter()
                .filter(|c| c.default_selected)
                .map(|c| c.key.clone())
                .collect::<Vec<_>>();
            if selected.is_empty() {
                o.capabilities
                    .iter()
                    .take(2)
                    .map(|c| c.key.clone())
                    .collect()
            } else {
                selected
            }
        })
        .unwrap_or_default();
    for (label, operation) in [
        ("host configuration backup", HostArchiveOperation::Backup),
        (
            "support bundle",
            HostArchiveOperation::Support {
                entries: entries.clone(),
            },
        ),
    ] {
        let path = directory.join(format!("lab-{}-{stamp}.archive", label.replace(' ', "-")));
        let started = std::time::Instant::now();
        let entry = match c
            .latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .and_then(|target| {
                c.transfer(
                    OperationId::new(),
                    target,
                    TransferRequest {
                        action: TransferAction::Host {
                            host: member,
                            expected_version: version.clone(),
                            operation,
                        },
                        file: TransferFile::destination(&path).expect("archive destination"),
                    },
                )
            }) {
            Ok(operation) => {
                let row = wait_outcome(&operation, 900).await;
                let size = std::fs::metadata(&path).map(|m| m.len()).ok();
                let removed = std::fs::remove_file(&path).is_ok();
                json!({"archive": label, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "bytes": size, "elapsed_ms": started.elapsed().as_millis() as u64, "removed_after_measuring": removed,
                       "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
            }
            Err(error) => json!({"archive": label, "accepted": false, "error": error.to_string()}),
        };
        archives.push(entry);
    }

    // An archive that did not succeed leaves an unresolved infrastructure operation that
    // blocks VM work by design; record what happened before touching the guest.
    if archives.iter().any(|a| a["state"] != "Succeeded") {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".xenadmin-transfer-")
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        c.close().await;
        lab.record(
            "host-archives-and-template-lifecycle",
            json!({"member": member.to_string(), "support_overview": overview_json, "support_entries": entries, "archives": archives, "template_lifecycle": "not attempted"}),
        );
        panic!("an archive transfer did not succeed: {archives:?}");
    }

    // Template lifecycle on the halted guest.
    let mut power = Vec::new();
    if c.latest().inventory().unwrap().vms[&guest.id].power_state == PowerState::Running {
        power.push(halt_guest(&c, guest.id).await);
    }
    let lifecycle = |source: xenadmin_core::VmId, operation: VmLifecycleOperation, seconds: u64| {
        let c = &c;
        async move {
            match c.latest().observe_vm(source).and_then(|target| {
                c.submit(
                    OperationId::new(),
                    VmCommand::Lifecycle {
                        source: target,
                        operation,
                    },
                )
            }) {
                Ok(operation) => {
                    let row = wait_vm_outcome(&operation, seconds).await;
                    json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                           "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
                }
                Err(error) => json!({"accepted": false, "error": error.to_string()}),
            }
        }
    };
    let find_named = |name: &str, template: bool| {
        let snapshot = c.latest();
        snapshot.inventory().and_then(|i| {
            i.vms
                .values()
                .find(|v| v.name == name && v.is_template == template && v.id != guest.id)
                .cloned()
        })
    };
    let mut steps = Vec::new();
    // Leftovers of an interrupted run go first, through the same reviewed deletions.
    for vm in inventory
        .vms
        .values()
        .filter(|v| {
            (v.name.starts_with("lab-template-") || v.name.starts_with("lab-from-template-"))
                && v.id != guest.id
        })
        .cloned()
        .collect::<Vec<_>>()
    {
        if vm.power_state == PowerState::Running {
            halt_guest(&c, vm.id).await;
        }
        let disks = disks_of(c.latest().inventory().unwrap(), vm.id);
        let outcome = lifecycle(
            vm.id,
            VmLifecycleOperation::Delete {
                expected_template: vm.is_template,
                disks,
                snapshots: Vec::new(),
            },
            300,
        )
        .await;
        steps.push(json!({"step": "delete leftover", "name": vm.name, "outcome": outcome}));
    }
    let template_name = format!("lab-template-{stamp}");
    let outcome = lifecycle(
        guest.id,
        VmLifecycleOperation::Clone {
            name: template_name.clone(),
            expected_template: false,
            mode: VmCopyMode::Fast,
            as_template: true,
        },
        600,
    )
    .await;
    let template = tokio::time::timeout(deadline(60), async {
        loop {
            if let Some(t) = find_named(&template_name, true) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .ok();
    let template_disks = template
        .as_ref()
        .map(|t| disks_of(c.latest().inventory().unwrap(), t.id))
        .unwrap_or_default();
    let guest_disks = disks_of(c.latest().inventory().unwrap(), guest.id);
    steps.push(json!({"step": "fast clone to template", "outcome": outcome, "template": template.as_ref().map(|t| t.id.to_string()), "disks": template_disks.len(),
                      "disks_shared_with_guest": template_disks.iter().filter(|d| guest_disks.contains(d)).count()}));
    let mut created_vm = None;
    if let Some(template) = &template {
        let renamed = format!("{template_name}-edited");
        let outcome = lifecycle(
            template.id,
            VmLifecycleOperation::EditTemplate {
                name: renamed.clone(),
                description: "Edited by the live harness".into(),
            },
            180,
        )
        .await;
        let read_back = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms.get(&template.id).is_some_and(|v| {
                    v.name == renamed && v.description == "Edited by the live harness"
                })
            })
        })
        .await;
        steps.push(
            json!({"step": "edit template metadata", "outcome": outcome, "read_back": read_back}),
        );
        let sr = local_sr_of(c.latest().inventory().unwrap(), resident)
            .expect("resident local repository");
        let vm_name = format!("lab-from-template-{stamp}");
        let started = std::time::Instant::now();
        let outcome = lifecycle(
            template.id,
            VmLifecycleOperation::Clone {
                name: vm_name.clone(),
                expected_template: true,
                mode: VmCopyMode::Full { sr },
                as_template: false,
            },
            900,
        )
        .await;
        let vm = tokio::time::timeout(deadline(60), async {
            loop {
                if let Some(v) = find_named(&vm_name, false) {
                    return v;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .ok();
        let vm_disks = vm
            .as_ref()
            .map(|v| disks_of(c.latest().inventory().unwrap(), v.id))
            .unwrap_or_default();
        let on_sr = {
            let snapshot = c.latest();
            let i = snapshot.inventory().unwrap();
            vm_disks
                .iter()
                .all(|d| i.details.vdis.get(d).and_then(|v| v.sr) == Some(sr))
        };
        steps.push(json!({"step": "full copy from template to VM", "outcome": outcome, "elapsed_ms": started.elapsed().as_millis() as u64, "vm": vm.as_ref().map(|v| v.id.to_string()),
                          "disks": vm_disks.len(), "disks_on_chosen_repository": on_sr, "disks_shared_with_template": vm_disks.iter().filter(|d| template_disks.contains(d)).count()}));
        created_vm = vm;
    }
    if let Some(vm) = &created_vm {
        let disks = disks_of(c.latest().inventory().unwrap(), vm.id);
        let outcome = lifecycle(
            vm.id,
            VmLifecycleOperation::Delete {
                expected_template: false,
                disks: disks.clone(),
                snapshots: Vec::new(),
            },
            300,
        )
        .await;
        let gone = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                !i.vms.contains_key(&vm.id) && disks.iter().all(|d| !i.details.vdis.contains_key(d))
            })
        })
        .await;
        steps.push(
            json!({"step": "delete the VM with its disks", "outcome": outcome, "gone": gone}),
        );
    }
    if let Some(template) = &template {
        let disks = disks_of(c.latest().inventory().unwrap(), template.id);
        let outcome = lifecycle(
            template.id,
            VmLifecycleOperation::Delete {
                expected_template: true,
                disks: disks.clone(),
                snapshots: Vec::new(),
            },
            300,
        )
        .await;
        let gone = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                !i.vms.contains_key(&template.id)
                    && disks.iter().all(|d| !i.details.vdis.contains_key(d))
            })
        })
        .await;
        steps.push(
            json!({"step": "delete the template with its disks", "outcome": outcome, "gone": gone}),
        );
    }
    let guest_disks_after = disks_of(c.latest().inventory().unwrap(), guest.id);
    power.push(power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await);
    c.close().await;
    lab.record(
        "host-archives-and-template-lifecycle",
        json!({"member": member.to_string(), "guest": guest.name, "support_overview": overview_json, "support_entries": entries, "archives": archives,
               "template_lifecycle": steps, "guest_disks_intact": guest_disks_after == guest_disks, "power": power}),
    );
    for archive in &archives {
        assert_eq!(archive["accepted"], true, "{archive}");
        assert_eq!(archive["state"], "Succeeded", "{archive}");
        assert!(
            archive["bytes"].as_u64().is_some_and(|b| b > 0),
            "{archive}"
        );
    }
    for step in &steps {
        assert_eq!(step["outcome"]["accepted"], true, "{step}");
        assert_eq!(step["outcome"]["state"], "Succeeded", "{step}");
        if step.get("gone").is_some() {
            assert_eq!(step["gone"], true, "{step}");
        }
    }
    assert!(
        template.is_some() && created_vm.is_some(),
        "template or VM not visible"
    );
    assert_eq!(guest_disks_after, guest_disks, "guest disks changed");
}

/// P10-02, P10-03, P10-06, P10-07: create an internal network and a tagged VLAN interface
/// on the coordinator's management NIC attached to it, rescan the host's physical
/// interfaces, show that unplugging the management interface is refused before submission,
/// unplug and plug the VLAN interface, delete the VLAN and then the unused network.
#[tokio::test]
#[ignore]
async fn lab_vlan_lifecycle_rescan_and_management_unplug_refusal() {
    use xenadmin_core::{NetworkOperation, PifIdentity, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let management = inventory
        .details
        .pifs
        .values()
        .find(|p| {
            p.host == Some(coordinator) && p.management == Some(true) && p.physical == Some(true)
        })
        .cloned()
        .expect("management interface");
    let identity = PifIdentity::from_inventory(&management).expect("interface identity");
    let tag: u16 = 100;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();

    // Leftovers of an interrupted run: VLANs on lab networks, then the networks.
    for network in inventory
        .networks
        .iter()
        .filter(|n| n.name.starts_with("lab-vlan-net-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        let snapshot = c.latest();
        let i = snapshot.inventory().unwrap();
        for (vlan, child) in i
            .details
            .vlans
            .values()
            .filter_map(|v| {
                let child = i.details.pifs.get(&v.untagged_pif?)?;
                (child.network == Some(xenadmin_core::NetworkId::from(network.uuid)))
                    .then(|| (v.clone(), child.clone()))
            })
            .collect::<Vec<_>>()
        {
            let outcome = resource_op(
                &c,
                "delete leftover VLAN",
                ResourceCommand::Network {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Vlan(vlan.id))
                        .unwrap(),
                    operation: NetworkOperation::DeleteVlan {
                        parent: vlan.tagged_pif.unwrap_or(management.id),
                        child: child.id,
                        network: xenadmin_core::NetworkId::from(network.uuid),
                        tag: vlan.tag.unwrap_or(0) as u16,
                    },
                },
                300,
            )
            .await;
            steps.push(json!({"step": "delete leftover VLAN", "outcome": outcome}));
        }
        let outcome = resource_op(
            &c,
            "delete leftover network",
            ResourceCommand::DeleteInternalNetwork {
                network: c
                    .latest()
                    .observe_resource(ResourceTarget::Network(xenadmin_core::NetworkId::from(
                        network.uuid,
                    )))
                    .unwrap(),
                expected_name: network.name.clone(),
            },
            300,
        )
        .await;
        steps.push(json!({"step": "delete leftover network", "outcome": outcome}));
    }

    let name = format!("lab-vlan-net-{stamp}");
    let created_network = resource_op(
        &c,
        "create network for the VLAN",
        ResourceCommand::CreateInternalNetwork {
            pool: c
                .latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .unwrap(),
            name: name.clone(),
            description: "Target network for the live VLAN scenario".into(),
            mtu: 1500,
        },
        300,
    )
    .await;
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
    .await
    .ok();
    steps.push(json!({"step": "create network for the VLAN", "outcome": created_network, "network": network.as_ref().map(|n| n.uuid.to_string())}));
    let Some(network) = network else {
        c.close().await;
        lab.record(
            "vlan-lifecycle-rescan-and-management-unplug-refusal",
            json!({"steps": steps}),
        );
        panic!("target network not visible");
    };
    let network_id: xenadmin_core::NetworkId = network.uuid.into();

    let created = resource_op(
        &c,
        "create VLAN",
        ResourceCommand::Network {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Network(network_id))
                .unwrap(),
            operation: NetworkOperation::CreateVlan {
                parent: management.id,
                expected: identity.clone(),
                tag,
            },
        },
        300,
    )
    .await;
    let vlan = tokio::time::timeout(deadline(60), async {
        loop {
            let snapshot = c.latest();
            if let Some(i) = snapshot.inventory()
                && let Some((vlan, child)) = i.details.vlans.values().find_map(|v| {
                    let child = i.details.pifs.get(&v.untagged_pif?)?;
                    (v.tag == Some(tag as u64)
                        && v.tagged_pif == Some(management.id)
                        && child.host == Some(coordinator))
                    .then(|| (v.clone(), child.clone()))
                })
            {
                return Some((vlan, child));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .ok()
    .flatten();
    steps.push(json!({"step": "create VLAN", "outcome": created, "vlan": vlan.as_ref().map(|(v, child)| json!({"id": v.id.to_string(), "tag": v.tag, "child_pif": child.id.to_string(), "child_device": child.device, "child_vlan": child.vlan, "attached": child.currently_attached, "network": child.network.map(|n| n.to_string()), "on_target_network": child.network == Some(network_id)}))}));

    let scanned = resource_op(
        &c,
        "rescan physical interfaces",
        ResourceCommand::Network {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Host(coordinator))
                .unwrap(),
            operation: NetworkOperation::ScanInterfaces,
        },
        300,
    )
    .await;
    steps.push(json!({"step": "rescan physical interfaces", "outcome": scanned}));

    // Unplugging the management interface must be refused before anything is sent.
    let refused = match c
        .latest()
        .observe_resource(ResourceTarget::Pif(management.id))
        .and_then(|target| {
            c.submit_resource(
                OperationId::new(),
                ResourceCommand::Network {
                    target,
                    operation: NetworkOperation::SetAttached {
                        expected: identity.clone(),
                        expected_attached: true,
                        attached: false,
                    },
                },
            )
        }) {
        Ok(operation) => {
            let row = wait_outcome(&operation, 120).await;
            json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
        }
        Err(error) => json!({"accepted": false, "error": error.to_string()}),
    };
    let still_attached =
        c.latest().inventory().unwrap().details.pifs[&management.id].currently_attached;
    steps.push(json!({"step": "unplug management interface", "outcome": refused, "expected": "refused", "still_attached": still_attached}));

    if let Some((vlan, child)) = &vlan {
        for (label, from, to) in [
            ("unplug VLAN interface", true, false),
            ("plug VLAN interface", false, true),
        ] {
            let current = c
                .latest()
                .inventory()
                .unwrap()
                .details
                .pifs
                .get(&child.id)
                .cloned();
            let identity_now = current.as_ref().and_then(PifIdentity::from_inventory);
            let outcome = match identity_now {
                Some(identity_now) => match c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(child.id))
                    .and_then(|target| {
                        c.submit_resource(
                            OperationId::new(),
                            ResourceCommand::Network {
                                target,
                                operation: NetworkOperation::SetAttached {
                                    expected: identity_now,
                                    expected_attached: from,
                                    attached: to,
                                },
                            },
                        )
                    }) {
                    Ok(operation) => {
                        let row = wait_outcome(&operation, 300).await;
                        json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
                    }
                    Err(error) => json!({"accepted": false, "error": error.to_string()}),
                },
                None => json!({"accepted": false, "error": "VLAN interface identity unavailable"}),
            };
            let read_back = settle(60, || {
                c.latest().inventory().is_some_and(|i| {
                    i.details
                        .pifs
                        .get(&child.id)
                        .is_some_and(|p| p.currently_attached == Some(to))
                })
            })
            .await;
            steps.push(json!({"step": label, "outcome": outcome, "read_back": read_back}));
        }
        let deleted = resource_op(
            &c,
            "delete VLAN",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Vlan(vlan.id))
                    .unwrap(),
                operation: NetworkOperation::DeleteVlan {
                    parent: management.id,
                    child: child.id,
                    network: network_id,
                    tag,
                },
            },
            300,
        )
        .await;
        let gone = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                !i.details.vlans.contains_key(&vlan.id) && !i.details.pifs.contains_key(&child.id)
            })
        })
        .await;
        steps.push(json!({"step": "delete VLAN", "outcome": deleted, "gone": gone}));
    }
    let deleted_network = resource_op(
        &c,
        "delete the unused network",
        ResourceCommand::DeleteInternalNetwork {
            network: c
                .latest()
                .observe_resource(ResourceTarget::Network(network_id))
                .unwrap(),
            expected_name: name.clone(),
        },
        300,
    )
    .await;
    let network_gone = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            !i.networks
                .iter()
                .any(|n| xenadmin_core::NetworkId::from(n.uuid) == network_id)
        })
    })
    .await;
    steps.push(json!({"step": "delete the unused network", "outcome": deleted_network, "gone": network_gone}));
    c.close().await;
    lab.record("vlan-lifecycle-rescan-and-management-unplug-refusal", json!({"coordinator": coordinator.to_string(), "management_interface": management.device, "tag": tag, "network": network_id.to_string(), "steps": steps}));
    for step in &steps {
        if step["expected"] == "refused" {
            assert!(
                step["outcome"]["accepted"] == false || step["outcome"]["state"] == "Rejected",
                "{step}"
            );
            assert_eq!(step["still_attached"], true, "{step}");
        } else {
            assert_eq!(step["outcome"]["accepted"], true, "{step}");
            assert_eq!(step["outcome"]["state"], "Succeeded", "{step}");
            if step.get("read_back").is_some() {
                assert_eq!(step["read_back"], true, "{step}");
            }
            if step.get("gone").is_some() {
                assert_eq!(step["gone"], true, "{step}");
            }
        }
    }
    assert!(vlan.is_some(), "VLAN interface not visible");
}

/// P05-13: read the pool's snapshot-schedule support (coordinator clock), create a disk
/// snapshot schedule (created disabled), add the guest, enable it, run it now through the
/// synchronous plugin call, disable it, remove the guest, delete the empty schedule and
/// delete the snapshot the run created.
#[tokio::test]
#[ignore]
async fn lab_snapshot_schedule_lifecycle() {
    use xenadmin_core::{
        ResourceCommand, ScheduledSnapshotKind, SnapshotPolicyConfig, SnapshotPolicyOperation,
        SnapshotPolicyState, SnapshotSchedule, VmCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let guest_id = guest.id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let state_of = |c: &Connection, id: xenadmin_core::SnapshotPolicyId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.snapshot_policies.get(&id).cloned())
            .and_then(|p| SnapshotPolicyState::from_policy(&p).ok())
    };
    let run = |label: &'static str,
               id: xenadmin_core::SnapshotPolicyId,
               make: Box<dyn Fn(SnapshotPolicyState) -> SnapshotPolicyOperation>,
               check: Box<dyn Fn(&xenadmin_core::Inventory) -> bool>| {
        let c = &c;
        let pool_target = &pool_target;
        let state_of = &state_of;
        async move {
            let outcome = match state_of(c, id) {
                Some(expected) => {
                    resource_op(
                        c,
                        label,
                        ResourceCommand::SnapshotPolicy {
                            pool: pool_target(c),
                            operation: make(expected),
                        },
                        300,
                    )
                    .await
                }
                None => {
                    json!({"step": label, "accepted": false, "error": "schedule not in the inventory"})
                }
            };
            let read_back = settle(60, || c.latest().inventory().is_some_and(|i| check(i))).await;
            json!({"step": label, "outcome": outcome, "read_back": read_back})
        }
    };
    let mut leftovers = Vec::new();
    for policy in inventory
        .details
        .snapshot_policies
        .values()
        .filter(|p| p.name.starts_with("lab-schedule-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        let id = policy.id;
        if policy.enabled == Some(true) {
            leftovers.push(
                run(
                    "disable leftover",
                    id,
                    Box::new(|expected| SnapshotPolicyOperation::SetEnabled {
                        expected,
                        enabled: false,
                        clock: None,
                    }),
                    Box::new(move |i| {
                        i.details
                            .snapshot_policies
                            .get(&id)
                            .is_some_and(|p| p.enabled == Some(false))
                    }),
                )
                .await,
            );
        }
        for vm in policy.vms.clone() {
            leftovers.push(
                run(
                    "remove leftover member",
                    id,
                    Box::new(move |expected| SnapshotPolicyOperation::Membership {
                        expected,
                        vm,
                        join: false,
                    }),
                    Box::new(move |i| {
                        i.details
                            .snapshot_policies
                            .get(&id)
                            .is_some_and(|p| !p.vms.contains(&vm))
                    }),
                )
                .await,
            );
        }
        leftovers.push(
            run(
                "delete leftover",
                id,
                Box::new(|expected| SnapshotPolicyOperation::DeleteEmpty { expected }),
                Box::new(move |i| !i.details.snapshot_policies.contains_key(&id)),
            )
            .await,
        );
    }
    let support = match c.snapshot_policy_support(pool_target(&c)) {
        Ok(query) => tokio::time::timeout(deadline(120), async {
            loop {
                if let Some(result) = query.latest() {
                    return result;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .unwrap_or(Err("no support answer within the deadline".into())),
        Err(error) => Err(error.to_string()),
    };
    let support_json = match &support {
        Ok(s) => {
            json!({"coordinator": s.clock.coordinator.to_string(), "coordinator_name": s.coordinator_name, "utc_offset_minutes": s.clock.utc_offset_minutes, "local_time": s.local_time, "utc_time": s.utc_time})
        }
        Err(e) => json!({"error": e}),
    };
    let snapshots_before = c
        .latest()
        .inventory()
        .unwrap()
        .snapshots
        .for_vm(guest_id)
        .map(|s| s.id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut steps = Vec::new();
    let mut policy_id = None;
    if let Ok(support) = &support {
        let name = format!("lab-schedule-{stamp}");
        let config = SnapshotPolicyConfig {
            name: name.clone(),
            description: "Created by the live harness".into(),
            kind: ScheduledSnapshotKind::Disk,
            retained: 2,
            schedule: SnapshotSchedule::Hourly { minute: 30 },
        };
        let created = resource_op(
            &c,
            "create schedule",
            ResourceCommand::SnapshotPolicy {
                pool: pool_target(&c),
                operation: SnapshotPolicyOperation::Create {
                    config,
                    clock: support.clock,
                },
            },
            300,
        )
        .await;
        policy_id = tokio::time::timeout(deadline(60), async {
            loop {
                if let Some(id) = c.latest().inventory().and_then(|i| {
                    i.details
                        .snapshot_policies
                        .values()
                        .find(|p| p.name == name)
                        .map(|p| p.id)
                }) {
                    return id;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .ok();
        let created_enabled = policy_id.and_then(|id| {
            c.latest()
                .inventory()
                .unwrap()
                .details
                .snapshot_policies
                .get(&id)
                .and_then(|p| p.enabled)
        });
        steps.push(json!({"step": "create schedule", "outcome": created, "policy": policy_id.map(|p| p.to_string()), "enabled_after_creation": created_enabled}));
        if let Some(id) = policy_id {
            steps.push(
                run(
                    "add the guest",
                    id,
                    Box::new(move |expected| SnapshotPolicyOperation::Membership {
                        expected,
                        vm: guest_id,
                        join: true,
                    }),
                    Box::new(move |i| {
                        i.details
                            .snapshot_policies
                            .get(&id)
                            .is_some_and(|p| p.vms.contains(&guest_id))
                    }),
                )
                .await,
            );
            let clock = support.clock;
            steps.push(
                run(
                    "enable",
                    id,
                    Box::new(move |expected| SnapshotPolicyOperation::SetEnabled {
                        expected,
                        enabled: true,
                        clock: Some(clock),
                    }),
                    Box::new(move |i| {
                        i.details
                            .snapshot_policies
                            .get(&id)
                            .is_some_and(|p| p.enabled == Some(true))
                    }),
                )
                .await,
            );
            let before = snapshots_before.clone();
            let started = std::time::Instant::now();
            let mut ran = run(
                "run now",
                id,
                Box::new(|expected| SnapshotPolicyOperation::RunNow { expected }),
                Box::new(move |i| {
                    i.snapshots
                        .for_vm(guest_id)
                        .any(|s| !before.contains(&s.id))
                }),
            )
            .await;
            ran["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
            ran["last_run_time"] = json!(
                c.latest()
                    .inventory()
                    .unwrap()
                    .details
                    .snapshot_policies
                    .get(&id)
                    .and_then(|p| p.last_run_time.clone())
            );
            steps.push(ran);
            steps.push(
                run(
                    "disable",
                    id,
                    Box::new(|expected| SnapshotPolicyOperation::SetEnabled {
                        expected,
                        enabled: false,
                        clock: None,
                    }),
                    Box::new(move |i| {
                        i.details
                            .snapshot_policies
                            .get(&id)
                            .is_some_and(|p| p.enabled == Some(false))
                    }),
                )
                .await,
            );
            steps.push(
                run(
                    "remove the guest",
                    id,
                    Box::new(move |expected| SnapshotPolicyOperation::Membership {
                        expected,
                        vm: guest_id,
                        join: false,
                    }),
                    Box::new(move |i| {
                        i.details
                            .snapshot_policies
                            .get(&id)
                            .is_some_and(|p| !p.vms.contains(&guest_id))
                    }),
                )
                .await,
            );
            steps.push(
                run(
                    "delete the empty schedule",
                    id,
                    Box::new(|expected| SnapshotPolicyOperation::DeleteEmpty { expected }),
                    Box::new(move |i| !i.details.snapshot_policies.contains_key(&id)),
                )
                .await,
            );
        }
    }
    // Delete whatever snapshot the run created, with its disks.
    let mut cleanup = Vec::new();
    for snapshot in c
        .latest()
        .inventory()
        .unwrap()
        .snapshots
        .for_vm(guest_id)
        .filter(|s| !snapshots_before.contains(&s.id))
        .map(|s| (s.id, s.name.clone()))
        .collect::<Vec<_>>()
    {
        let inventory = c.latest().inventory().unwrap().clone();
        let disks = inventory
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(snapshot.0) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let outcome = match c
            .latest()
            .observe_snapshot(guest_id, snapshot.0)
            .and_then(|target| {
                c.submit(
                    OperationId::new(),
                    VmCommand::DeleteSnapshot {
                        snapshot: target,
                        disks: disks.clone(),
                    },
                )
            }) {
            Ok(operation) => {
                let row = wait_vm_outcome(&operation, 300).await;
                json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
            }
            Err(error) => json!({"accepted": false, "error": error.to_string()}),
        };
        let gone = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.snapshots.get(&snapshot.0).is_none())
        })
        .await;
        cleanup.push(json!({"snapshot": snapshot.0.to_string(), "name": snapshot.1, "disks": disks.len(), "outcome": outcome, "gone": gone}));
    }
    c.close().await;
    lab.record("snapshot-schedule-lifecycle", json!({"guest": guest.name, "support": support_json, "leftovers": leftovers, "steps": steps, "snapshot_cleanup": cleanup}));
    assert!(
        support.is_ok(),
        "snapshot schedules unsupported: {support_json}"
    );
    assert!(policy_id.is_some(), "schedule not visible");
    for step in &steps {
        assert_eq!(step["outcome"]["accepted"], true, "{step}");
        assert_eq!(step["outcome"]["state"], "Succeeded", "{step}");
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
    for entry in &cleanup {
        assert_eq!(entry["gone"], true, "{entry}");
    }
}

/// P01-15, P02-02, P02-05, P02-06, P02-10, P09-01: log in read-only with the explicit
/// certificate-validation opt-out instead of a pin and record what the inventory presents:
/// the storage and network tables, each guest's general information, each host's and the
/// pool's properties, and which optional record classes the host negotiated.
#[tokio::test]
#[ignore]
async fn lab_inventory_tables_and_certificate_opt_out() {
    use xenadmin_core::{ConnectionState, NetworkId, SrId};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let mut options = ConnectionOptions::new(lab.endpoint(0));
    options.accept_invalid_certificates = true;
    options.read_only = true;
    let c = backend.connect(options, lab.credentials());
    let ready = c
        .wait_ready(deadline(90))
        .await
        .map(|_| true)
        .map_err(|e| e.to_string());
    let snapshot = c.latest();
    let Some(i) = snapshot.inventory() else {
        lab.record(
            "inventory-tables-and-certificate-opt-out",
            json!({"ready": ready, "state": format!("{:?}", snapshot.state())}),
        );
        panic!("no inventory: {ready:?}");
    };
    let now = SystemTime::now();
    let storage = i.storage.iter().map(|s| {
        let id = SrId::from(s.uuid);
        let d = i.details.storage.get(&id);
        json!({"name": s.name, "kind": s.kind, "capacity_bytes": s.capacity_bytes, "used_bytes": s.used_bytes,
               "shared": d.and_then(|d| d.shared), "content_type": d.map(|d| d.content_type.clone()),
               "attachments": i.details.pbds.values().filter(|p| p.sr == Some(id)).count(),
               "attached": i.details.pbds.values().filter(|p| p.sr == Some(id) && p.currently_attached == Some(true)).count(),
               "disks": i.details.vdis.values().filter(|v| v.sr == Some(id)).count()})
    }).collect::<Vec<_>>();
    let networks = i.networks.iter().map(|n| {
        let id = NetworkId::from(n.uuid);
        let d = i.details.networks.get(&id);
        json!({"name": n.name, "bridge": n.bridge, "mtu": d.and_then(|d| d.mtu), "managed": d.and_then(|d| d.managed),
               "pifs": i.details.pifs.values().filter(|p| p.network == Some(id)).count(),
               "vifs": i.details.vifs.values().filter(|v| v.network == Some(id)).count()})
    }).collect::<Vec<_>>();
    let guests = i.guests().map(|v| {
        let d = i.details.vms.get(&v.id);
        let metrics = d.and_then(|d| d.metrics).and_then(|m| i.details.vm_metrics.get(&m));
        let guest = d.and_then(|d| d.guest_metrics).and_then(|g| i.details.guest_metrics.get(&g));
        json!({"name": v.name, "power_state": format!("{:?}", v.power_state), "host": v.host.map(|h| h.to_string()),
               "vcpus_at_startup": d.and_then(|d| d.vcpus_at_startup), "vcpus_max": d.and_then(|d| d.vcpus_max),
               "memory_static_max": d.and_then(|d| d.memory_static_max), "memory_actual": metrics.and_then(|m| m.memory_actual),
               "boot_policy": d.and_then(|d| d.boot_policy.clone()), "firmware": d.and_then(|d| d.firmware.clone()), "domain_type": d.and_then(|d| d.domain_type.clone()),
               "start_time": metrics.and_then(|m| m.start_time.clone()), "elapsed_since_boot": metrics.and_then(|m| m.elapsed_since_boot(now)),
               "guest_os": guest.map(|g| g.os_version.get("name").cloned()), "tools": guest.map(|g| g.tools_version.len()), "guest_networks": guest.map(|g| g.networks.len()),
               "disks": i.details.vm_disks.get(&v.id).map(|b| b.len()), "nics": i.details.vifs.values().filter(|n| n.vm == Some(v.id)).count()})
    }).collect::<Vec<_>>();
    let hosts = i.hosts.values().map(|h| {
        let d = i.details.hosts.get(&h.id);
        let metrics = d.and_then(|d| d.metrics).and_then(|m| i.details.host_metrics.get(&m));
        json!({"name": h.name, "address": h.address, "enabled": h.enabled, "version": h.version,
               "product": d.map(|d| json!({"brand": d.software_version.get("product_brand"), "version": d.software_version.get("product_version"), "build": d.software_version.get("build_number"), "xen": d.software_version.get("xen")})),
               "cpu": d.map(|d| json!({"count": d.cpu_info.get("cpu_count"), "model": d.cpu_info.get("modelname")})),
               "memory_total": metrics.and_then(|m| m.memory_total), "memory_free": metrics.and_then(|m| m.memory_free),
               "boot_time": d.and_then(|d| d.boot_time.clone()), "agent_start_time": d.and_then(|d| d.agent_start_time.clone()),
               "manufacturer": d.and_then(|d| d.bios_strings.get("system-manufacturer").cloned()), "capabilities": d.map(|d| d.capabilities.len()),
               "external_auth": d.and_then(|d| d.external_auth_type.clone())})
    }).collect::<Vec<_>>();
    let pool = i.details.pool.as_ref().map(|p| json!({"name": p.name, "description": p.description, "coordinator": p.administration.coordinator.map(|h| h.to_string()),
        "ha_enabled": p.ha_enabled, "default_sr": p.default_sr.map(|s| s.to_string()), "igmp_snooping": p.igmp_snooping_enabled, "auto_poweron": p.auto_poweron,
        "migration_compression": p.administration.migration_compression, "https_only": p.administration.https_only, "tls_verification": p.administration.tls_verification_enabled}));
    let classes = i
        .details
        .classes
        .iter()
        .map(|(k, v)| (k.clone(), format!("{v:?}")))
        .collect::<std::collections::BTreeMap<_, _>>();
    let state = format!("{:?}", snapshot.state());
    let endpoint = snapshot.active_endpoint().map(|e| e.to_string());
    let pool_id = i.pool_id.to_string();
    c.close().await;
    lab.record("inventory-tables-and-certificate-opt-out", json!({
        "ready": ready, "state": state, "endpoint": endpoint, "pool_id": pool_id, "certificate_validation": "opted out explicitly, no pin",
        "storage": storage, "networks": networks, "guests": guests, "hosts": hosts, "pool": pool, "optional_classes": classes,
    }));
    assert_eq!(state, format!("{:?}", ConnectionState::Ready), "{ready:?}");
    assert!(
        storage
            .iter()
            .any(|s| s["capacity_bytes"].as_u64().is_some_and(|b| b > 0)),
        "no repository capacity"
    );
    assert_eq!(hosts.len(), 2, "pool hosts");
    assert!(
        !classes.is_empty(),
        "no optional class negotiation recorded"
    );
}

/// P03-07, P11-01, P14-02: while another client's storage live migration of the guest
/// runs, review a force shutdown with the captured running task. The client refuses it
/// without the operator's acceptance that the migration may be left unfinished (finding
/// 63), and that refusal is recorded with the guest still running; the other client then
/// cancels its own migration, and once the VM has no task the guest is forced off by the
/// ordinary power action and started again. With
/// `XENADMIN_LAB_FORCE_DURING_MIGRATION=accept` the acceptance is given and the host's
/// answer to the forced shutdown is recorded instead.
#[tokio::test]
#[ignore]
async fn lab_force_shutdown_review_with_a_running_task() {
    use xenadmin_core::{
        ForceRecoveryAction, OperationControl, PowerAction, PowerState, VmCommand,
        VmRecoveryOperation,
    };
    let lab = Lab::load();
    // Finding 63: a forced shutdown during a storage migration can leave the migration
    // stuck on the hosts. Without the operator's acceptance the client refuses it, and
    // that refusal is what this scenario records. `XENADMIN_LAB_FORCE_DURING_MIGRATION=accept`
    // gives the acceptance and sends the forced shutdown, to look at the host's behaviour;
    // budget a recovery of the lab for that.
    let accept_migration =
        std::env::var("XENADMIN_LAB_FORCE_DURING_MIGRATION").as_deref() == Ok("accept");
    let first_state = tempfile::tempdir().unwrap();
    let second_state = tempfile::tempdir().unwrap();
    let first_backend = Backend::with_state_directory(first_state.path().into()).unwrap();
    let second_backend = Backend::with_state_directory(second_state.path().into()).unwrap();
    let c = connect(&lab, &first_backend, 0, false).await;
    let other = connect(&lab, &second_backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    if guest.power_state != PowerState::Running {
        power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await;
    }
    let inventory = other.latest().inventory().unwrap().clone();
    let source_host = inventory.vms[&guest.id].host.expect("resident host");
    let destination = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != source_host && inventory.hosts[h].enabled)
        .expect("member");
    let plan = migration_plan(&inventory, guest.id, destination);
    let source_sr = plan.disks[0].expected_sr;
    let advertised = settle(120, || {
        other.latest().inventory().is_some_and(|i| {
            i.details.vms.get(&guest.id).is_some_and(|v| {
                v.allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "migrate_send"))
            })
        })
    })
    .await;
    let migration = other
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: other.latest().observe_vm(guest.id).unwrap(),
                operation: plan,
            },
        )
        .expect("migration accepted");
    let migration_id = migration.latest().id;
    let seen = settle(90, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vms
                .get(&guest.id)
                .is_some_and(|v| !v.current_operations.is_empty())
        })
    })
    .await;
    let now = c.latest().inventory().unwrap().clone();
    let running = now.details.vms[&guest.id]
        .current_operations
        .iter()
        .map(|o| json!({"operation": o.operation, "task": o.task.map(|t| t.to_string())}))
        .collect::<Vec<_>>();
    let captured = VmRecoveryOperation::tasks(&now, guest.id);
    let review = match &captured {
        Ok(tasks) => {
            let operation = VmRecoveryOperation::Force {
                action: ForceRecoveryAction::Shutdown,
                tasks: tasks.clone(),
                interrupts_migration: accept_migration,
            };
            let reason = operation.unavailable_reason(&now, guest.id);
            match c.latest().observe_vm(guest.id).and_then(|target| {
                c.submit(
                    OperationId::new(),
                    VmCommand::Recovery {
                        vm: target,
                        operation,
                    },
                )
            }) {
                Ok(op) => {
                    let row = wait_vm_outcome(&op, 300).await;
                    json!({"captured_tasks": tasks.len(), "unavailable_reason": reason, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                           "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
                }
                Err(error) => {
                    json!({"captured_tasks": tasks.len(), "unavailable_reason": reason, "accepted": false, "error": error.to_string()})
                }
            }
        }
        Err(error) => {
            json!({"captured_tasks": 0, "accepted": false, "error": format!("capture: {error}")})
        }
    };
    let migration_state_at_review = format!("{:?}", migration.latest().state);
    let task_still_present = c.latest().inventory().unwrap().details.vms[&guest.id]
        .current_operations
        .len();
    // The other client cancels its own migration; this client never touches that task.
    let cancel_requested = other
        .control_operation(migration_id, OperationControl::RequestCancellation)
        .map(|_| true)
        .map_err(|e| e.to_string());
    let migration_row = wait_vm_outcome(&migration, 900).await;
    // A halted VM has no resident host; judge consistency by its disks alone.
    let disks_on = |i: &xenadmin_core::Inventory| -> Vec<xenadmin_core::SrId> {
        i.details
            .vm_disks
            .get(&guest.id)
            .map(|vbds| {
                vbds.iter()
                    .filter_map(|b| i.details.vbds.get(b))
                    .filter(|b| b.kind == "Disk")
                    .filter_map(|b| i.details.vdis.get(&b.vdi?)?.sr)
                    .collect()
            })
            .unwrap_or_default()
    };
    let together = |srs: &[xenadmin_core::SrId]| {
        srs.len() == 2
            && (srs.iter().all(|s| *s == source_sr) || srs.iter().all(|s| *s != source_sr))
    };
    let consistent = settle(300, || {
        c.latest()
            .inventory()
            .is_some_and(|i| together(&disks_on(i)))
    })
    .await;
    let capturable = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .vms
                .get(&guest.id)
                .is_some_and(|v| v.current_operations.is_empty())
                && VmRecoveryOperation::tasks(i, guest.id).is_ok_and(|t| t.is_empty())
        })
    })
    .await;
    // With no task left there is nothing to capture, and the forced shutdown is the
    // ordinary power action the host advertises again.
    let mut forced = None;
    if c.latest().inventory().unwrap().vms[&guest.id].power_state == PowerState::Running {
        let entry = power_to(&c, guest.id, PowerAction::HardShutdown, PowerState::Halted).await;
        forced = Some(
            json!({"state": entry["state"], "detail": entry["detail"], "halted": entry["reached"]}),
        );
    }
    // The owner rechecks its unknown migration once the VM has settled.
    let owner_recheck = if migration_row.state == OperationState::OutcomeUnknown {
        let requested = other
            .control_operation(migration_id, OperationControl::Recheck)
            .map(|_| true)
            .map_err(|e| e.to_string());
        let final_state = tokio::time::timeout(deadline(180), async {
            loop {
                let row = migration.latest();
                if row.state.terminal()
                    || row.state == OperationState::AcknowledgedUnknown
                    || (row.state == OperationState::OutcomeUnknown && row.detail != migration_row.detail)
                {
                    return json!({"state": format!("{:?}", row.state), "detail": row.detail});
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        })
        .await
        .unwrap_or_else(|_| json!({"state": format!("{:?}", migration.latest().state), "detail": "still unresolved after the deadline"}));
        json!({"requested": requested, "after": final_state})
    } else {
        json!({"skipped": "migration record was not unknown"})
    };
    let snapshot_after = c.latest();
    let inventory_after = snapshot_after.inventory().unwrap();
    let disks_after = disks_on(inventory_after);
    let placement_after = json!({"power_state": format!("{:?}", inventory_after.vms[&guest.id].power_state), "host": inventory_after.vms[&guest.id].host.map(|h| h.to_string()), "repositories": disks_after.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "together": together(&disks_after)});
    let after = inventory_after.vms[&guest.id]
        .host
        .map(|h| (h, disks_after.clone()));
    let mut returned = None;
    if after.as_ref().is_some_and(|(host, _)| *host == destination) {
        let inventory = c.latest().inventory().unwrap().clone();
        let back = c.submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: migration_plan(&inventory, guest.id, source_host),
            },
        );
        returned = Some(match back {
            Ok(op) => {
                let row = wait_vm_outcome(&op, 1200).await;
                json!({"state": format!("{:?}", row.state), "detail": row.detail})
            }
            Err(error) => json!({"error": error.to_string()}),
        });
    }
    let mut power = Vec::new();
    if c.latest().inventory().unwrap().vms[&guest.id].power_state == PowerState::Halted {
        power.push(power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await);
    }
    other.close().await;
    c.close().await;
    lab.record("force-shutdown-review-with-a-running-task", json!({
        "guest": guest.name, "accepting_an_unfinished_migration": accept_migration, "migration_advertised": advertised, "task_seen": seen, "running_operations_at_review": running,
        "force_review_during_task": review, "migration_state_at_review": migration_state_at_review, "task_present_after_review": task_still_present,
        "cancellation_requested_by_owner": cancel_requested, "migration": {"state": format!("{:?}", migration_row.state), "detail": migration_row.detail},
        "placement_consistent": consistent, "capturable_after": capturable, "owner_recheck": owner_recheck, "placement_after": placement_after,
        "force_shutdown_without_tasks": forced, "returned": returned, "power": power,
    }));
    assert!(seen, "the migration task never appeared on the VM");
    assert_eq!(review["accepted"], true, "{review}");
    if accept_migration {
        assert!(
            ["Succeeded", "Rejected", "Failed"].contains(&review["state"].as_str().unwrap_or("")),
            "{review}"
        );
    } else {
        // Refused by the client in its own words, with nothing sent: the guest ran on and
        // the migration's task was still there.
        assert_eq!(review["state"], "Rejected", "{review}");
        assert!(
            review["detail"]
                .as_str()
                .is_some_and(|d| d.contains("migration")),
            "{review}"
        );
        assert!(
            task_still_present > 0,
            "the migration's task was gone after the refusal"
        );
    }
    assert_eq!(placement_after["together"], true, "{placement_after}");
    if let Some(forced) = &forced {
        assert_eq!(forced["state"], "Succeeded", "{forced}");
        assert_eq!(forced["halted"], true, "{forced}");
    }
}

/// P08-02, P08-03, P06-08: create a UEFI VM from the generic template with the host's tools
/// image as the installation source and a new 2 GiB disk on the coordinator's local
/// repository, read its boot status, review a Secure Boot policy change on the halted VM,
/// and delete the VM with its disk.
#[tokio::test]
#[ignore]
async fn lab_vm_wizard_with_iso_uefi_and_secure_boot() {
    use xenadmin_core::{
        GuestFirmware, PowerState, VmCommand, VmLifecycleOperation, VmSettingEdit, VmSettingsPatch,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let template = inventory
        .vms
        .values()
        .find(|v| v.is_template && v.name == "Other install media")
        .expect("generic template")
        .clone();
    let network: xenadmin_core::NetworkId = inventory
        .networks
        .iter()
        .find(|n| n.bridge == "xenbr0")
        .expect("pool network")
        .uuid
        .into();
    let sr = local_sr_of(&inventory, coordinator).expect("coordinator local repository");
    let iso = inventory
        .details
        .tools_media
        .keys()
        .next()
        .copied()
        .expect("guest tools image");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lifecycle = |source: xenadmin_core::VmId, operation: VmLifecycleOperation, seconds: u64| {
        let c = &c;
        async move {
            match c.latest().observe_vm(source).and_then(|target| {
                c.submit(
                    OperationId::new(),
                    VmCommand::Lifecycle {
                        source: target,
                        operation,
                    },
                )
            }) {
                Ok(operation) => {
                    let row = wait_vm_outcome(&operation, seconds).await;
                    json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                           "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()})
                }
                Err(error) => json!({"accepted": false, "error": error.to_string()}),
            }
        }
    };
    let mut steps = Vec::new();
    for vm in inventory
        .vms
        .values()
        .filter(|v| v.name.starts_with("lab-wizard-"))
        .cloned()
        .collect::<Vec<_>>()
    {
        if vm.power_state == PowerState::Running {
            halt_guest(&c, vm.id).await;
        }
        let disks = disks_of(c.latest().inventory().unwrap(), vm.id);
        let outcome = lifecycle(
            vm.id,
            VmLifecycleOperation::Delete {
                expected_template: vm.is_template,
                disks,
                snapshots: Vec::new(),
            },
            300,
        )
        .await;
        steps.push(json!({"step": "delete leftover", "name": vm.name, "outcome": outcome}));
    }
    let name = format!("lab-wizard-{stamp}");
    let created = lifecycle(
        template.id,
        VmLifecycleOperation::Create {
            name: name.clone(),
            description:
                "UEFI guest created by the live harness with the tools image as install source"
                    .into(),
            sr,
            network,
            iso: Some(iso),
            vcpus: 1,
            memory: 512 * 1024 * 1024,
            disk_size: Some(2 * 1024 * 1024 * 1024),
            firmware: GuestFirmware::Uefi,
            bios_host: None,
        },
        600,
    )
    .await;
    let vm = tokio::time::timeout(deadline(60), async {
        loop {
            if let Some(v) = c.latest().inventory().and_then(|i| {
                i.vms
                    .values()
                    .find(|v| v.name == name && !v.is_template)
                    .cloned()
            }) {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .ok();
    let devices = vm.as_ref().map(|vm| {
        let snapshot = c.latest();
        let i = snapshot.inventory().unwrap();
        let d = i.details.vms.get(&vm.id);
        json!({"id": vm.id.to_string(), "power_state": format!("{:?}", vm.power_state),
               "vbds": i.details.vbds.values().filter(|b| b.vm == Some(vm.id)).map(|b| json!({"kind": b.kind, "device": b.userdevice, "empty": b.empty, "vdi": b.vdi.map(|v| v.to_string()), "is_tools_image": b.vdi == Some(iso), "bootable": b.bootable})).collect::<Vec<_>>(),
               "disk_sizes": disks_of(i, vm.id).iter().filter_map(|d| i.details.vdis.get(d)).map(|d| d.virtual_size).collect::<Vec<_>>(),
               "firmware": d.and_then(|d| d.firmware.clone()), "secure_boot": d.and_then(|d| d.secure_boot), "boot_policy": d.and_then(|d| d.boot_policy.clone()), "domain_type": d.and_then(|d| d.domain_type.clone()),
               "vcpus": d.and_then(|d| d.vcpus_at_startup), "memory_static_max": d.and_then(|d| d.memory_static_max), "nics": i.details.vifs.values().filter(|n| n.vm == Some(vm.id)).count()})
    });
    steps.push(
        json!({"step": "create UEFI VM with ISO and disk", "outcome": created, "vm": devices}),
    );
    let mut boot_status = None;
    let mut secure_boot = Vec::new();
    if let Some(vm) = &vm {
        boot_status = match c.latest().observe_vm(vm.id).and_then(|target| c.guest_boot_status(target)) {
            Ok(query) => tokio::time::timeout(deadline(60), async {
                loop {
                    if let Some(result) = query.latest() {
                        return match result {
                            Ok(s) => json!({"firmware": s.firmware, "policy": s.policy, "guest": format!("{:?}", s.guest), "pool": format!("{:?}", s.pool)}),
                            Err(e) => json!({"error": e}),
                        };
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            })
            .await
            .ok(),
            Err(error) => Some(json!({"error": error.to_string()})),
        };
        let current = c.latest().inventory().unwrap().details.vms[&vm.id]
            .secure_boot
            .unwrap_or(false);
        for value in [!current, current] {
            let previous = c.latest().inventory().unwrap().details.vms[&vm.id]
                .secure_boot
                .unwrap_or(false);
            if previous == value {
                secure_boot.push(json!({"value": value, "skipped": "already set"}));
                continue;
            }
            let entry = match VmSettingsPatch::new(vec![VmSettingEdit::SecureBoot {
                previous,
                value,
            }]) {
                Ok(patch) => match c.latest().observe_vm(vm.id).and_then(|target| {
                    c.submit(
                        OperationId::new(),
                        VmCommand::Configure { vm: target, patch },
                    )
                }) {
                    Ok(operation) => {
                        let row = wait_vm_outcome(&operation, 180).await;
                        let read_back = settle(60, || {
                            c.latest().inventory().is_some_and(|i| {
                                i.details
                                    .vms
                                    .get(&vm.id)
                                    .is_some_and(|d| d.secure_boot == Some(value))
                            })
                        })
                        .await;
                        json!({"value": value, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": read_back})
                    }
                    Err(error) => {
                        json!({"value": value, "accepted": false, "error": error.to_string()})
                    }
                },
                Err(error) => {
                    json!({"value": value, "accepted": false, "error": error.to_string()})
                }
            };
            let stop = entry["state"] != "Succeeded";
            secure_boot.push(entry);
            if stop {
                break;
            }
        }
        let disks = disks_of(c.latest().inventory().unwrap(), vm.id);
        let outcome = lifecycle(
            vm.id,
            VmLifecycleOperation::Delete {
                expected_template: false,
                disks: disks.clone(),
                snapshots: Vec::new(),
            },
            300,
        )
        .await;
        let gone = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                !i.vms.contains_key(&vm.id) && disks.iter().all(|d| !i.details.vdis.contains_key(d))
            })
        })
        .await;
        let iso_kept = c
            .latest()
            .inventory()
            .unwrap()
            .details
            .vdis
            .contains_key(&iso);
        steps.push(json!({"step": "delete the VM with its disk", "outcome": outcome, "disks": disks.len(), "gone": gone, "tools_image_kept": iso_kept}));
    }
    c.close().await;
    lab.record("vm-wizard-with-iso-uefi-and-secure-boot", json!({"template": template.name, "install_source": iso.to_string(), "steps": steps, "boot_status": boot_status, "secure_boot_policy": secure_boot}));
    for step in &steps {
        assert_eq!(step["outcome"]["accepted"], true, "{step}");
        assert_eq!(step["outcome"]["state"], "Succeeded", "{step}");
        if step.get("gone").is_some() {
            assert_eq!(step["gone"], true, "{step}");
            assert_eq!(step["tools_image_kept"], true, "{step}");
        }
    }
    let vm_json = steps
        .iter()
        .find(|s| s["step"] == "create UEFI VM with ISO and disk")
        .and_then(|s| s.get("vm"))
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(vm_json["firmware"], "uefi", "{vm_json}");
    assert!(
        vm_json["vbds"].as_array().is_some_and(|v| v
            .iter()
            .any(|b| b["kind"] == "CD" && b["is_tools_image"] == true)
            && v.iter().any(|b| b["kind"] == "Disk")),
        "{vm_json}"
    );
    for entry in &secure_boot {
        if entry.get("skipped").is_none() {
            assert_eq!(entry["accepted"], true, "{entry}");
        }
    }
}

/// P05-05, P05-03, P05-04, P04-02: a memory checkpoint of the running guest is taken while a
/// timed report is pending inside the guest; the report arrives once from the live guest,
/// the checkpoint is restored through the reviewed workflow (shut down, revert, resume), and
/// the same report arrives again from the resumed memory image; the checkpoint is deleted.
#[tokio::test]
#[ignore]
async fn lab_checkpoint_restore_resumes_memory_state() {
    use xenadmin_core::{PowerAction, PowerState, SnapshotName, SnapshotWorkflow, VmCommand};
    let lab = Lab::load();
    let gateway = std::env::var("XENADMIN_LAB_GATEWAY").unwrap_or_else(|_| "192.168.254.1".into());
    let port: u16 = 8000;
    let guest_ip =
        std::env::var("XENADMIN_LAB_GUEST_IP").unwrap_or_else(|_| "192.168.254.100".into());
    let reports = report_listener(&format!("{gateway}:{port}")).await;
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let guest = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    if guest.power_state != PowerState::Running {
        power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = format!("RESUMED-{stamp}");
    let count = |reports: &Reports| {
        reports
            .received
            .lock()
            .unwrap()
            .iter()
            .filter(|v| **v == token)
            .count()
    };

    let console = c
        .open_console(c.latest().observe_vm(guest.id).unwrap())
        .expect("console");
    guest_report_marker(&console, &gateway, port, &guest_ip, None).await;
    let marker_reported = wait_report(&reports, "A", 90).await;
    // A report that fires from inside the guest 45 s later, whatever the guest does meanwhile.
    type_line(
        &console,
        &format!("(sleep 45; wget -q -O- http://{gateway}:{port}/report/{token}) &"),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let armed_at = std::time::Instant::now();

    let name = format!("lab-ckpt-{stamp}");
    let checkpoint = c
        .submit(
            OperationId::new(),
            VmCommand::CreateCheckpoint {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                name: SnapshotName::new(&name).unwrap(),
            },
        )
        .expect("checkpoint accepted");
    let checkpoint = wait_vm_outcome(&checkpoint, 300).await;
    let checkpoint_seconds = armed_at.elapsed().as_secs();
    let snapshot = tokio::time::timeout(deadline(60), async {
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
    .await
    .expect("checkpoint listed");
    let first = wait_report(&reports, &token, 120).await;
    let first_count = count(&reports);
    let first_after_seconds = armed_at.elapsed().as_secs();
    console.close();

    let restore = c
        .submit(
            OperationId::new(),
            VmCommand::SnapshotWorkflow {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: SnapshotWorkflow::Restore {
                    snapshot: snapshot.id,
                    expected_state: PowerState::Running,
                    checkpoint: true,
                    shutdown: true,
                    start_after: true,
                },
            },
        )
        .expect("restore accepted");
    let restore = wait_vm_outcome(&restore, 600).await;
    let restored_at = std::time::Instant::now();
    let running = settle(180, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&guest.id)
                .is_some_and(|v| v.power_state == PowerState::Running)
        })
    })
    .await;
    let second = settle(150, || count(&reports) > first_count).await;
    let second_after_seconds = restored_at.elapsed().as_secs();
    let total = count(&reports);
    let frame = match c
        .latest()
        .observe_vm(guest.id)
        .and_then(|target| c.open_console(target))
    {
        Ok(console) => {
            let digest = stable_frame_or_last(&console, 20).await;
            let saved = save_frame(&lab, &console, "after-checkpoint-restore");
            console.close();
            json!({"digest": digest, "file": saved})
        }
        Err(error) => json!({"error": error.to_string()}),
    };
    let disks = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vbds
        .values()
        .filter(|b| b.snapshot == Some(snapshot.id) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect::<Vec<_>>();
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(guest.id, snapshot.id).unwrap(),
                disks: disks.clone(),
            },
        )
        .expect("checkpoint deletion accepted");
    let deletion = wait_vm_outcome(&deletion, 300).await;
    let gone = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            i.snapshots.get(&snapshot.id).is_none()
                && disks.iter().all(|d| !i.details.vdis.contains_key(d))
        })
    })
    .await;
    reports.task.abort();
    c.close().await;
    lab.record("checkpoint-restore-resumes-memory-state", json!({
        "guest": guest.name, "marker_reported": marker_reported, "token": token,
        "checkpoint": {"state": format!("{:?}", checkpoint.state), "detail": checkpoint.detail, "seconds_after_arming": checkpoint_seconds, "snapshot": snapshot.id.to_string(), "power_state": format!("{:?}", snapshot.power_state)},
        "report_from_live_guest": {"received": first, "count": first_count, "seconds_after_arming": first_after_seconds},
        "restore": {"state": format!("{:?}", restore.state), "detail": restore.detail, "steps": restore.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()},
        "running_after_restore": running, "report_from_resumed_memory": {"received": second, "seconds_after_restore": second_after_seconds, "total_reports": total},
        "console_after": frame, "checkpoint_deletion": {"state": format!("{:?}", deletion.state), "detail": deletion.detail, "disks": disks.len(), "gone": gone},
    }));
    assert_eq!(
        checkpoint.state,
        OperationState::Succeeded,
        "{}",
        checkpoint.detail
    );
    assert!(first, "the live guest never reported");
    assert_eq!(
        restore.state,
        OperationState::Succeeded,
        "{}",
        restore.detail
    );
    assert!(running, "guest not running after the restore");
    assert!(second, "the resumed memory image never repeated the report");
    assert_eq!(
        deletion.state,
        OperationState::Succeeded,
        "{}",
        deletion.detail
    );
    assert!(gone, "checkpoint or its disks remain");
}

/// P12-08, P12-03, P12-05: raise the member's control-domain memory through the reviewed
/// editor, reboot the member through maintenance so the new allocation takes effect, read
/// the control domain's actual memory after the boot, restore the previous value with a
/// second reboot and bring the member out of maintenance.
#[tokio::test]
#[ignore]
async fn lab_dom0_memory_change_with_member_reboot() {
    use xenadmin_core::{HostOperation, MemoryLimits, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("member");
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must not host guests"
    );
    let dom0 = inventory.details.hosts[&member]
        .control_domain
        .expect("control domain");
    let limits_of = |i: &xenadmin_core::Inventory| {
        let d = i.details.vms.get(&dom0)?;
        Some(MemoryLimits {
            static_min: d.memory_static_min?,
            dynamic_min: d.memory_dynamic_min?,
            dynamic_max: d.memory_dynamic_max?,
            static_max: d.memory_static_max?,
        })
    };
    let actual_of = |i: &xenadmin_core::Inventory| {
        i.details
            .vms
            .get(&dom0)
            .and_then(|d| d.metrics)
            .and_then(|m| i.details.vm_metrics.get(&m))
            .and_then(|m| m.memory_actual)
    };
    let original = limits_of(&inventory).expect("control-domain limits");
    let desired = original.static_max + 128 * 1024 * 1024;
    let mut steps = Vec::new();
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let details = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&member).cloned())
    };

    // An interrupted earlier run can leave the member enabled with the client's maintenance
    // marker still set (finding 12); exiting maintenance first keeps the scenario
    // self-contained and re-exercises the no-op enable path.
    if details(&c).and_then(|h| h.maintenance_mode) == Some(true) {
        let mut entry = resource_op(
            &c,
            "clear the leftover maintenance marker",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || details(&c).and_then(|h| h.maintenance_mode)
                == Some(false))
            .await
        );
        steps.push(entry);
    }

    let mut entry = resource_op(
        &c,
        "set control-domain memory",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::SetDom0Memory {
                control_domain: dom0,
                expected: original.clone(),
                memory: desired,
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| limits_of(i)
            .is_some_and(|l| l.static_max == desired
                && l.dynamic_max == desired
                && l.dynamic_min == desired)))
        .await
    );
    entry["actual_before_reboot"] = json!(actual_of(c.latest().inventory().unwrap()));
    steps.push(entry);

    let mut entry = resource_op(
        &c,
        "enter maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: true,
                resident_vms: Vec::new(),
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(|i| !i.hosts[&member].enabled))
        .await
    );
    steps.push(entry);

    for (label, back_to) in [
        ("reboot with the new allocation", None),
        (
            "reboot after restoring the allocation",
            Some(original.clone()),
        ),
    ] {
        if let Some(limits) = &back_to {
            let current = limits_of(c.latest().inventory().unwrap()).expect("current limits");
            let mut entry = resource_op(
                &c,
                "restore control-domain memory",
                ResourceCommand::Host {
                    target: host_target(&c),
                    operation: HostOperation::SetDom0Memory {
                        control_domain: dom0,
                        expected: current,
                        memory: limits.static_max,
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(60, || c.latest().inventory().is_some_and(
                    |i| limits_of(i).is_some_and(|l| l.static_max == limits.static_max)
                ))
                .await
            );
            steps.push(entry);
        }
        // A reboot re-enables the host on XCP-ng 8.3.0 while the client's maintenance
        // marker stays (finding 12); exit maintenance (which accepts that state and clears
        // the marker) and enter it again before each reviewed reboot.
        if c.latest()
            .inventory()
            .is_some_and(|i| i.hosts[&member].enabled)
        {
            let mut entry = resource_op(
                &c,
                "exit maintenance after the reboot re-enabled the host",
                ResourceCommand::Host {
                    target: host_target(&c),
                    operation: HostOperation::ExitMaintenance,
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || {
                    c.latest().inventory().is_some_and(|i| {
                        i.hosts[&member].enabled
                            && i.details
                                .hosts
                                .get(&member)
                                .is_some_and(|h| h.maintenance_mode != Some(true))
                    })
                })
                .await
            );
            steps.push(entry);
            let mut entry = resource_op(
                &c,
                "re-enter maintenance before the reboot",
                ResourceCommand::Host {
                    target: host_target(&c),
                    operation: HostOperation::EnterMaintenance {
                        expected_enabled: true,
                        resident_vms: Vec::new(),
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || {
                    c.latest()
                        .inventory()
                        .is_some_and(|i| !i.hosts[&member].enabled)
                })
                .await
            );
            steps.push(entry);
        }
        let boot_before = details(&c).and_then(|h| h.boot_time).expect("boot time");
        let started = std::time::Instant::now();
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::Reboot {
                    expected_boot_time: boot_before.clone(),
                },
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
        let live = settle(300, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .hosts
                    .get(&member)
                    .and_then(|h| h.metrics)
                    .and_then(|m| i.details.host_metrics.get(&m))
                    .and_then(|m| m.live)
                    == Some(true)
            })
        })
        .await;
        let expected_actual = back_to.as_ref().map(|l| l.static_max).unwrap_or(desired);
        let settled = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                actual_of(i).is_some_and(|a| a.abs_diff(expected_actual) <= expected_actual / 20)
            })
        })
        .await;
        entry["boot_time_changed"] = json!(rebooted);
        entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
        entry["host_live"] = json!(live);
        entry["expected_actual"] = json!(expected_actual);
        entry["actual_after_boot"] = json!(actual_of(c.latest().inventory().unwrap()));
        entry["actual_within_five_percent"] = json!(settled);
        entry["limits_after_boot"] = json!(limits_of(c.latest().inventory().unwrap()).map(|l| json!({"static_min": l.static_min, "dynamic_min": l.dynamic_min, "dynamic_max": l.dynamic_max, "static_max": l.static_max})));
        steps.push(entry);
    }

    let mut entry = resource_op(
        &c,
        "exit maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::ExitMaintenance,
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.hosts[&member].enabled))
        .await
    );
    steps.push(entry);
    c.close().await;
    lab.record("dom0-memory-change-with-member-reboot", json!({"member": inventory.hosts[&member].name, "control_domain": dom0.to_string(),
        "original": {"static_min": original.static_min, "dynamic_min": original.dynamic_min, "dynamic_max": original.dynamic_max, "static_max": original.static_max},
        "desired": desired, "steps": steps}));
    for step in &steps {
        assert_eq!(step["accepted"], true, "{step}");
        assert_eq!(step["state"], "Succeeded", "{step}");
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
        if step.get("boot_time_changed").is_some() {
            assert_eq!(step["boot_time_changed"], true, "{step}");
            assert_eq!(step["actual_within_five_percent"], true, "{step}");
        }
    }
}

/// P07-14, P07-15, P07-16, P02-07: add a NIC on a fresh internal network while the guest is
/// halted, start the guest and disconnect and reconnect that NIC while it runs, halt the
/// guest again, move the NIC to the management network with a replacement MAC (destroy and
/// re-create), set and clear a rate limit when the host advertises one, read every change
/// back from the inventory's NIC graph, then remove the NIC, delete the network and start
/// the guest.
#[tokio::test]
#[ignore]
async fn lab_nic_plug_edit_and_rate_limit() {
    use xenadmin_core::{
        DeviceOperation, DeviceProperty, DeviceQos, PowerAction, PowerState, ResourceCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;

    // Leftovers of an earlier interrupted run go first, through the same reviewed
    // workflows: the guest's NICs on a network this scenario made, then those networks.
    let mut leftovers = Vec::new();
    let made_here: Vec<(xenadmin_core::NetworkId, String)> = inventory
        .networks
        .iter()
        .filter(|n| n.name.starts_with("lab-nic-"))
        .map(|n| (n.uuid.into(), n.name.clone()))
        .collect();
    if !made_here.is_empty() {
        if guest.power_state != PowerState::Halted {
            let mut halt = halt_guest(&c, vm).await;
            halt["step"] = json!("halt the guest to remove a leftover NIC");
            leftovers.push(halt);
        }
        for (network, name) in &made_here {
            let nics: Vec<_> = c
                .latest()
                .inventory()
                .unwrap()
                .details
                .vifs
                .values()
                .filter(|v| v.vm == Some(vm) && v.network == Some(*network))
                .map(|v| v.id)
                .collect();
            for vif in nics {
                let mut entry = device_op(&c, vm, DeviceOperation::DeleteNic { vif }, 120).await;
                entry["step"] = json!("remove a leftover NIC");
                entry["read_back"] = json!(
                    settle(60, || c
                        .latest()
                        .inventory()
                        .is_some_and(|i| !i.details.vifs.contains_key(&vif)))
                    .await
                );
                leftovers.push(entry);
            }
            let mut entry = resource_op(
                &c,
                "delete a leftover network",
                ResourceCommand::DeleteInternalNetwork {
                    network: c
                        .latest()
                        .observe_resource(ResourceTarget::Network(*network))
                        .unwrap(),
                    expected_name: name.clone(),
                },
                120,
            )
            .await;
            entry["read_back"] = json!(
                settle(60, || c
                    .latest()
                    .inventory()
                    .is_some_and(|i| !i.networks.iter().any(|n| &n.name == name)))
                .await
            );
            leftovers.push(entry);
        }
    }
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory.vms[&vm].clone();
    let existing: Vec<_> = inventory
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(vm))
        .cloned()
        .collect();
    assert_eq!(
        existing.len(),
        1,
        "the lab guest should carry exactly one NIC: {leftovers:?}"
    );
    let management = existing[0].network.expect("the existing NIC's network");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let vif_view = |c: &Connection, vif: xenadmin_core::VifId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.vifs.get(&vif).cloned())
    };
    let nic_by_device = |c: &Connection, device: &str| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vifs
                .values()
                .find(|v| v.vm == Some(vm) && v.device == device)
                .cloned()
        })
    };

    if guest.power_state != PowerState::Halted {
        let mut halt = halt_guest(&c, vm).await;
        halt["step"] = json!("halt the guest");
        steps.push(halt);
    }

    // A fresh internal network for the new NIC.
    let name = format!("lab-nic-{stamp}");
    let mut entry = resource_op(
        &c,
        "create internal network",
        ResourceCommand::CreateInternalNetwork {
            pool: c
                .latest()
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .unwrap(),
            name: name.clone(),
            description: "Internal network for the NIC lifecycle scenario".into(),
            mtu: 1500,
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.networks.iter().any(|n| n.name == name)))
        .await
    );
    steps.push(entry);
    let network_id: xenadmin_core::NetworkId = c
        .latest()
        .inventory()
        .unwrap()
        .networks
        .iter()
        .find(|n| n.name == name)
        .expect("created network")
        .uuid
        .into();

    // Add the NIC while halted.
    let mut entry = device_op(
        &c,
        vm,
        DeviceOperation::AddNic {
            network: network_id,
            device: "1".into(),
            mac: None,
        },
        120,
    )
    .await;
    entry["step"] = json!("add NIC on the internal network");
    entry["read_back"] = json!(
        settle(60, || nic_by_device(&c, "1")
            .is_some_and(|v| v.network == Some(network_id)))
        .await
    );
    steps.push(entry);
    let nic = nic_by_device(&c, "1").expect("new NIC");
    let vif = nic.id;
    let original_mac = nic.mac.clone();
    steps.push(json!({"step": "nic", "device": nic.device, "mac": original_mac, "qos_known": nic.qos_known, "qos_supported_algorithms": nic.qos_supported_algorithms}));

    // Start, then disconnect and reconnect the NIC while the guest runs.
    let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
    start["step"] = json!("start the guest");
    steps.push(start);
    let attached = settle(60, || {
        vif_view(&c, vif).is_some_and(|v| v.currently_attached == Some(true))
    })
    .await;
    let nic = vif_view(&c, vif).expect("NIC after start");
    let hotplug = nic
        .allowed_operations
        .as_deref()
        .is_some_and(|ops| ops.iter().any(|o| o == "unplug"));
    steps.push(json!({"step": "NIC attached after start", "read_back": attached, "currently_attached": nic.currently_attached, "allowed_operations": nic.allowed_operations, "unplug_advertised": hotplug}));
    if hotplug {
        let mut entry = device_op(&c, vm, DeviceOperation::UnplugNic { vif }, 180).await;
        entry["step"] = json!("disconnect the NIC while running");
        entry["read_back"] = json!(
            settle(60, || vif_view(&c, vif)
                .is_some_and(|v| v.currently_attached == Some(false)))
            .await
        );
        steps.push(entry);
        let plug_advertised = settle(30, || {
            vif_view(&c, vif).is_some_and(|v| {
                v.allowed_operations
                    .as_deref()
                    .is_some_and(|ops| ops.iter().any(|o| o == "plug"))
            })
        })
        .await;
        let mut entry = device_op(&c, vm, DeviceOperation::PlugNic { vif }, 180).await;
        entry["step"] = json!("reconnect the NIC while running");
        entry["plug_advertised_first"] = json!(plug_advertised);
        entry["read_back"] = json!(
            settle(60, || vif_view(&c, vif)
                .is_some_and(|v| v.currently_attached == Some(true)))
            .await
        );
        steps.push(entry);
    } else {
        // XAPI advertises VIF plug and unplug on a running VM only when the guest's PV
        // drivers support them; the tool-less guest never does, so the client must refuse
        // and name the missing advertisement.
        let mut entry = device_op(&c, vm, DeviceOperation::UnplugNic { vif }, 60).await;
        entry["step"] = json!("disconnect the NIC while running (not advertised by the host)");
        entry["expected_refusal"] = json!(true);
        steps.push(entry);
    }

    // Halt, then move the NIC to the management network with a replacement MAC.
    let mut halt = halt_guest(&c, vm).await;
    halt["step"] = json!("halt the guest for the NIC edit");
    steps.push(halt);
    let new_mac = format!(
        "0a:4c:41:42:{:02x}:{:02x}",
        (stamp >> 8) & 0xff,
        stamp & 0xff
    );
    let mut entry = device_op(
        &c,
        vm,
        DeviceOperation::EditNic {
            vif,
            expected_network: network_id,
            expected_mac: original_mac.clone(),
            network: management,
            mac: new_mac.clone(),
        },
        120,
    )
    .await;
    entry["step"] = json!("move the NIC to the management network with a new MAC");
    entry["read_back"] = json!(
        settle(60, || nic_by_device(&c, "1").is_some_and(|v| v.network
            == Some(management)
            && v.mac == new_mac))
        .await
    );
    steps.push(entry);
    let nic = nic_by_device(&c, "1").expect("edited NIC");
    steps.push(json!({"step": "nic after edit", "recreated": nic.id != vif, "mac": nic.mac}));
    let vif = nic.id;

    // Rate limit when the host advertises the algorithm.
    if nic
        .qos_supported_algorithms
        .as_deref()
        .is_some_and(|a| a.iter().any(|s| s == "ratelimit"))
    {
        let mut entry = device_op(
            &c,
            vm,
            DeviceOperation::Property {
                change: DeviceProperty::NicRate {
                    vif,
                    previous: DeviceQos {
                        algorithm: nic.qos_algorithm.clone(),
                        parameters: nic.qos_parameters.clone(),
                    },
                    kilobytes_per_second: Some(10_240),
                },
            },
            120,
        )
        .await;
        entry["step"] = json!("set a 10240 KB/s rate limit");
        entry["read_back"] = json!(
            settle(60, || vif_view(&c, vif)
                .is_some_and(|v| v.qos_algorithm == "ratelimit"
                    && v.qos_parameters.get("kbps").map(String::as_str)
                        == Some("10240")))
            .await
        );
        steps.push(entry);
        let limited = vif_view(&c, vif).expect("limited NIC");
        let mut entry = device_op(
            &c,
            vm,
            DeviceOperation::Property {
                change: DeviceProperty::NicRate {
                    vif,
                    previous: DeviceQos {
                        algorithm: limited.qos_algorithm.clone(),
                        parameters: limited.qos_parameters.clone(),
                    },
                    kilobytes_per_second: None,
                },
            },
            120,
        )
        .await;
        entry["step"] = json!("clear the rate limit");
        entry["read_back"] = json!(
            settle(60, || vif_view(&c, vif)
                .is_some_and(|v| v.qos_algorithm.is_empty()))
            .await
        );
        entry["parameters_after_clear"] = json!(vif_view(&c, vif).map(|v| v.qos_parameters));
        steps.push(entry);
    } else {
        steps.push(json!({"step": "rate limit", "skipped": "the host does not advertise the ratelimit QoS algorithm for this NIC", "qos_supported_algorithms": nic.qos_supported_algorithms}));
    }

    // Remove the NIC, delete the network and start the guest.
    let mut entry = device_op(&c, vm, DeviceOperation::DeleteNic { vif }, 120).await;
    entry["step"] = json!("remove the NIC");
    entry["read_back"] = json!(
        settle(60, || c
            .latest()
            .inventory()
            .is_some_and(|i| !i.details.vifs.contains_key(&vif)))
        .await
    );
    steps.push(entry);
    let mut entry = resource_op(
        &c,
        "delete internal network",
        ResourceCommand::DeleteInternalNetwork {
            network: c
                .latest()
                .observe_resource(ResourceTarget::Network(network_id))
                .unwrap(),
            expected_name: name.clone(),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c
            .latest()
            .inventory()
            .is_some_and(|i| !i.networks.iter().any(|n| n.name == name)))
        .await
    );
    steps.push(entry);
    let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
    start["step"] = json!("start the guest");
    steps.push(start);
    c.close().await;
    lab.record(
        "nic-plug-edit-and-rate-limit",
        json!({"guest": guest.name, "network": name, "leftovers_removed_first": leftovers, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert_eq!(step["state"], "Rejected", "{step}");
            assert!(
                step["detail"]
                    .as_str()
                    .is_some_and(|d| d.contains("does not advertise this operation")),
                "{step}"
            );
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P06-06, P03-04, P08-02, P08-08: create a diskless VM from the generic template, pin its
/// home server to the member and start it without a chosen host so the pool places it there,
/// halt it, move the home server to the coordinator and start it again, halt it, clear the
/// home server and delete the record. A leftover VM from an interrupted run is deleted first.
#[tokio::test]
#[ignore]
async fn lab_home_server_placement_with_a_diskless_vm() {
    use xenadmin_core::{
        GuestFirmware, PowerAction, PowerState, VmCommand, VmLifecycleOperation, VmSettingEdit,
        VmSettingsPatch,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let sr = local_sr_of(&inventory, coordinator).expect("coordinator local repository");
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
        .expect("management network");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let delete_record = |c: &Connection, vm: xenadmin_core::VmId| {
        let operation = c
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: c.latest().observe_vm(vm).unwrap(),
                    operation: VmLifecycleOperation::Delete {
                        expected_template: false,
                        disks: Vec::new(),
                        snapshots: Vec::new(),
                    },
                },
            )
            .expect("deletion accepted");
        async move { wait_vm_outcome(&operation, 300).await }
    };

    for leftover in inventory
        .vms
        .values()
        .filter(|v| v.name.starts_with("lab-place-") && !v.is_template)
    {
        if leftover.power_state != PowerState::Halted {
            halt_guest(&c, leftover.id).await;
        }
        let outcome = delete_record(&c, leftover.id).await;
        let gone = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| !i.vms.contains_key(&leftover.id))
        })
        .await;
        steps.push(json!({"step": "delete leftover VM", "name": leftover.name, "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": gone}));
    }

    let name = format!("lab-place-{stamp}");
    let create = c
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(template.id).unwrap(),
                operation: VmLifecycleOperation::Create {
                    name: name.clone(),
                    description: "Disposable diskless VM for the home-server placement scenario"
                        .into(),
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
    let outcome = wait_vm_outcome(&create, 300).await;
    let vm = outcome.created_vm.expect("created VM identity");
    let visible = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms.contains_key(&vm))
    })
    .await;
    steps.push(json!({"step": "create diskless VM", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": visible}));
    let affinity_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vms
                .get(&vm)
                .map(|d| (d.affinity, d.affinity_known))
        })
    };
    let resident = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.vms.get(&vm).map(|v| (v.power_state.clone(), v.host)))
    };

    for (label, value) in [
        ("member", Some(member)),
        ("coordinator", Some(coordinator)),
        ("cleared", None),
    ] {
        let current = affinity_of(&c).map(|a| a.0).unwrap_or(None);
        if current != value {
            let patch = VmSettingsPatch::new(vec![VmSettingEdit::HomeHost {
                previous: current,
                value,
            }])
            .unwrap();
            let operation = c
                .submit(
                    OperationId::new(),
                    VmCommand::Configure {
                        vm: c.latest().observe_vm(vm).unwrap(),
                        patch,
                    },
                )
                .expect("home server edit accepted");
            let row = wait_vm_outcome(&operation, 120).await;
            let applied = settle(60, || affinity_of(&c) == Some((value, true))).await;
            steps.push(json!({"step": format!("set home server to the {label}"), "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": applied}));
        } else {
            steps.push(json!({"step": format!("home server already {label}"), "skipped": true}));
        }
        let Some(expected) = value else {
            continue;
        };
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!(format!("start without a chosen host, home server {label}"));
        let placed = settle(60, || {
            resident(&c)
                .is_some_and(|(state, host)| state == PowerState::Running && host == Some(expected))
        })
        .await;
        start["placed_on_home_server"] = json!(placed);
        start["resident_host"] = json!(
            resident(&c)
                .and_then(|(_, h)| h)
                .map(|h| inventory.hosts[&h].name.clone())
        );
        start["read_back"] = json!(placed);
        steps.push(start);
        let mut halt = halt_guest(&c, vm).await;
        halt["step"] = json!(format!("halt on the {label}"));
        steps.push(halt);
    }

    let outcome = delete_record(&c, vm).await;
    let gone = settle(120, || {
        c.latest()
            .inventory()
            .is_some_and(|i| !i.vms.contains_key(&vm))
    })
    .await;
    steps.push(json!({"step": "delete the record", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": gone}));
    c.close().await;
    lab.record(
        "home-server-placement-with-a-diskless-vm",
        json!({"vm": name, "member": inventory.hosts[&member].name, "coordinator": inventory.hosts[&coordinator].name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P01-04, P01-03: a connection whose profile endpoint no longer answers but which carries
/// the pool identity and the verified members a Ready connection reports (as the desktop
/// profile editor persists them) reaches Ready through a remembered member after the dead
/// endpoint's failure is reported; without the pool identity the client keeps retrying the
/// dead endpoint and never tries the members.
#[tokio::test]
#[ignore]
async fn lab_remembered_member_fallback_after_endpoint_failure() {
    use std::collections::BTreeSet;
    use xenadmin_core::ConnectionState;
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let pool = c.latest().inventory().unwrap().pool_id;
    let remembered: Vec<Endpoint> = c.latest().verified_members().to_vec();
    let learned_through = c.latest().active_endpoint().map(|e| e.to_string());
    c.close().await;
    assert!(
        !remembered.is_empty(),
        "a Ready connection reports its verified members"
    );
    let dead = lab.unused_endpoint();
    let mut pins = Vec::new();
    for endpoint in &remembered {
        let observation = inspect_result(endpoint)
            .await
            .expect("remembered member certificate");
        pins.push((endpoint.clone(), observation.details.fingerprint.clone()));
    }
    let options_for = |expected_pool: Option<xenadmin_core::PoolId>| {
        let mut options = ConnectionOptions::new(dead.clone());
        options.request_timeout = Duration::from_secs(10);
        options.expected_pool = expected_pool;
        options.fallback_endpoints = remembered.clone();
        for (endpoint, fingerprint) in &pins {
            options.add_host_certificate(endpoint.clone(), fingerprint.clone());
        }
        options
    };

    // With the pool identity: the dead endpoint fails, a remembered member is tried.
    let started = std::time::Instant::now();
    let fallback = backend.connect(options_for(Some(pool)), lab.credentials());
    let mut transitions: Vec<Value> = Vec::new();
    let ready = tokio::time::timeout(deadline(240), async {
        loop {
            let s = fallback.latest();
            let entry = json!({"state": format!("{:?}", s.state()), "endpoint": s.active_endpoint().map(|e| e.to_string()), "error": s.error(), "elapsed_ms": started.elapsed().as_millis() as u64});
            let same = transitions.last().is_some_and(|t| {
                t["state"] == entry["state"]
                    && t["endpoint"] == entry["endpoint"]
                    && t["error"] == entry["error"]
            });
            if !same {
                transitions.push(entry);
            }
            if s.is_current() {
                return true;
            }
            if s.state() == ConnectionState::Failed {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or(false);
    let ready_after_ms = started.elapsed().as_millis() as u64;
    let snapshot = fallback.latest();
    let final_endpoint = snapshot.active_endpoint().map(|e| e.to_string());
    let final_pool = snapshot.inventory().map(|i| i.pool_id);
    let members_after = snapshot
        .verified_members()
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    drop(snapshot);
    fallback.close().await;

    // Without the pool identity the members must not be tried: another pool could answer.
    let unidentified = backend.connect(options_for(None), lab.credentials());
    let mut states = BTreeSet::new();
    let mut reached_ready = false;
    let observe_until = std::time::Instant::now() + Duration::from_secs(45);
    while std::time::Instant::now() < observe_until {
        let s = unidentified.latest();
        states.insert(format!("{:?}", s.state()));
        if s.is_current() {
            reached_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let last_error = unidentified.latest().error().map(str::to_owned);
    unidentified.close().await;

    lab.record(
        "remembered-member-fallback-after-endpoint-failure",
        json!({"dead_endpoint": dead.to_string(), "remembered": remembered.iter().map(|e| e.to_string()).collect::<Vec<_>>(), "learned_through": learned_through,
               "transitions": transitions, "ready": ready, "ready_after_ms": ready_after_ms, "final_endpoint": final_endpoint, "pool_matches": final_pool == Some(pool), "members_after": members_after,
               "without_pool_identity": {"states": states, "reached_ready": reached_ready, "last_error": last_error}}),
    );
    assert!(
        ready,
        "the fallback connection did not reach Ready: {transitions:?}"
    );
    assert!(
        remembered
            .iter()
            .any(|e| Some(e.to_string()) == final_endpoint),
        "Ready through {final_endpoint:?}"
    );
    assert_eq!(final_pool, Some(pool));
    assert!(
        transitions
            .iter()
            .any(|t| t["error"].as_str().is_some_and(|e| !e.is_empty())),
        "the dead endpoint's failure was never reported: {transitions:?}"
    );
    assert!(!reached_ready, "members were tried without a pool identity");
}

/// P09-10, P09-11, P09-08: move the pool's default repository to the other host's local
/// repository and back, each read back from the pool details, with a stale default
/// expectation refused in between; rename the member's local repository and its description
/// and restore both; rescan it.
#[tokio::test]
#[ignore]
async fn lab_default_storage_and_repository_metadata_round_trip() {
    use xenadmin_core::{ResourceCommand, SrId};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let member_sr = local_sr_of(&inventory, member).expect("member local repository");
    let coordinator_sr =
        local_sr_of(&inventory, coordinator).expect("coordinator local repository");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let default_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pool
                .as_ref()
                .map(|p| (p.default_sr, p.default_sr_known))
        })
    };
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let (original_default, known) = default_of(&c).expect("pool details");
    assert!(known, "the pool's default repository must be known");
    let original_default = original_default.expect("the pool has a default repository");
    let other = if original_default == member_sr {
        coordinator_sr
    } else {
        member_sr
    };
    steps.push(json!({"step": "pool defaults", "original_default": original_default.to_string(), "other": other.to_string()}));

    let mut entry = resource_op(
        &c,
        "set default storage to the other local repository",
        ResourceCommand::SetDefaultStorage {
            pool: pool_target(&c),
            sr: other,
            expected_default: Some(original_default),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(settle(60, || default_of(&c) == Some((Some(other), true))).await);
    steps.push(entry);
    // A review that still believes the previous default is current and asks for a change
    // must be refused by the fresh pool read, not applied.
    let mut entry = resource_op(
        &c,
        "stale default expectation",
        ResourceCommand::SetDefaultStorage {
            pool: pool_target(&c),
            sr: other,
            expected_default: Some(original_default),
        },
        60,
    )
    .await;
    entry["expected_refusal"] = json!(true);
    steps.push(entry);
    let mut entry = resource_op(
        &c,
        "restore default storage",
        ResourceCommand::SetDefaultStorage {
            pool: pool_target(&c),
            sr: original_default,
            expected_default: Some(other),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || default_of(&c)
            == Some((Some(original_default), true)))
        .await
    );
    steps.push(entry);

    // Repository name and description.
    let metadata_of = |c: &Connection, sr: SrId| {
        c.latest().inventory().and_then(|i| {
            let details = i.details.storage.get(&sr)?;
            let name = i
                .storage
                .iter()
                .find(|s| SrId::from(s.uuid) == sr)?
                .name
                .clone();
            Some((name, details.description.clone()))
        })
    };
    let sr_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Storage(member_sr))
            .unwrap()
    };
    let (name, description) = metadata_of(&c, member_sr).expect("member repository metadata");
    let renamed = format!("lab-repo-{stamp}");
    let new_description = "Renamed by the live storage scenario".to_string();
    let mut entry = resource_op(
        &c,
        "rename the member repository and its description",
        ResourceCommand::RenameMetadata {
            target: sr_target(&c),
            expected_name: name.clone(),
            expected_description: description.clone(),
            name: renamed.clone(),
            description: new_description.clone(),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || metadata_of(&c, member_sr).as_ref()
            == Some(&(renamed.clone(), new_description.clone())))
        .await
    );
    steps.push(entry);
    let mut entry = resource_op(
        &c,
        "restore the member repository metadata",
        ResourceCommand::RenameMetadata {
            target: sr_target(&c),
            expected_name: renamed.clone(),
            expected_description: new_description.clone(),
            name: name.clone(),
            description: description.clone(),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || metadata_of(&c, member_sr).as_ref()
            == Some(&(name.clone(), description.clone())))
        .await
    );
    steps.push(entry);

    let entry = resource_op(
        &c,
        "rescan the member repository",
        ResourceCommand::ScanStorage { sr: sr_target(&c) },
        180,
    )
    .await;
    steps.push(entry);
    c.close().await;
    lab.record(
        "default-storage-and-repository-metadata-round-trip",
        json!({"member": inventory.hosts[&member].name, "member_repository": name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert!(
                step["accepted"] == false || step["state"] == "Rejected",
                "{step}"
            );
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Create a diskless VM from the generic template on the coordinator's local repository and
/// the management network; returns its identity and the recorded step.
async fn create_diskless_vm(
    c: &Connection,
    inventory: &xenadmin_core::Inventory,
    name: &str,
    description: &str,
) -> (xenadmin_core::VmId, Value) {
    use xenadmin_core::{GuestFirmware, VmCommand, VmLifecycleOperation};
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let sr = local_sr_of(inventory, coordinator).expect("coordinator local repository");
    let template = inventory
        .vms
        .values()
        .find(|v| v.is_template && v.name == "Other install media")
        .expect("generic template");
    let network = inventory
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(coordinator) && p.management == Some(true))
        .and_then(|p| p.network)
        .expect("management network");
    let create = c
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(template.id).unwrap(),
                operation: VmLifecycleOperation::Create {
                    name: name.to_string(),
                    description: description.to_string(),
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
    let outcome = wait_vm_outcome(&create, 300).await;
    let vm = outcome.created_vm.expect("created VM identity");
    let visible = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.vms.contains_key(&vm))
    })
    .await;
    (
        vm,
        json!({"step": "create diskless VM", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": visible}),
    )
}

/// Delete a VM record (no disks, no snapshots) and wait for it to leave the inventory.
async fn delete_vm_record(c: &Connection, vm: xenadmin_core::VmId) -> Value {
    use xenadmin_core::{VmCommand, VmLifecycleOperation};
    let operation = c
        .submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(vm).unwrap(),
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
            .is_some_and(|i| !i.vms.contains_key(&vm))
    })
    .await;
    json!({"step": "delete the record", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": gone})
}

/// P12-04, P11-01, P12-03: place a diskless VM on the member through its home server, refuse
/// a maintenance review whose resident set is stale, enter maintenance with the VM reviewed
/// as the member's resident guest so the client evacuates it, verify it running on the
/// coordinator with the member disabled, exit maintenance, then halt and delete the VM.
/// When the host reports the VM as preventing evacuation, that refusal is the recorded
/// outcome and the member must be left enabled and unchanged.
#[tokio::test]
#[ignore]
async fn lab_member_evacuation_of_a_diskless_vm() {
    use xenadmin_core::{
        HostOperation, PowerAction, PowerState, ResourceCommand, VmCommand, VmSettingEdit,
        VmSettingsPatch,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must start empty"
    );
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    for leftover in inventory
        .vms
        .values()
        .filter(|v| v.name.starts_with("lab-evac-") && !v.is_template)
    {
        if leftover.power_state != PowerState::Halted {
            halt_guest(&c, leftover.id).await;
        }
        let mut entry = delete_vm_record(&c, leftover.id).await;
        entry["step"] = json!("delete leftover VM");
        steps.push(entry);
    }
    let name = format!("lab-evac-{stamp}");
    let (vm, created) = create_diskless_vm(
        &c,
        &inventory,
        &name,
        "Disposable diskless VM for the member evacuation scenario",
    )
    .await;
    steps.push(created);
    let patch = VmSettingsPatch::new(vec![VmSettingEdit::HomeHost {
        previous: c.latest().inventory().unwrap().details.vms[&vm].affinity,
        value: Some(member),
    }])
    .unwrap();
    let home = c
        .submit(
            OperationId::new(),
            VmCommand::Configure {
                vm: c.latest().observe_vm(vm).unwrap(),
                patch,
            },
        )
        .expect("home server edit accepted");
    let row = wait_vm_outcome(&home, 120).await;
    steps.push(json!({"step": "set home server to the member", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail}));
    let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
    start["step"] = json!("start on the member");
    start["read_back"] = json!(
        settle(60, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.vms[&vm].host == Some(member)))
        .await
    );
    steps.push(start);

    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let resident = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.vms.get(&vm).map(|v| (v.power_state.clone(), v.host)))
    };
    // A review that believes the member is empty must be refused by the fresh guest check.
    let mut entry = resource_op(
        &c,
        "enter maintenance with a stale empty resident set",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: true,
                resident_vms: Vec::new(),
            },
        },
        120,
    )
    .await;
    entry["expected_refusal"] = json!(true);
    let after = c.latest().inventory().unwrap().clone();
    entry["member_still_enabled"] = json!(after.hosts[&member].enabled);
    entry["vm_still_on_member"] = json!(resident(&c).is_some_and(|(_, h)| h == Some(member)));
    entry["vm_state"] = json!(resident(&c).map(|(s, h)| json!({"power_state": format!("{s:?}"), "host": h.map(|h| after.hosts[&h].name.clone())})));
    steps.push(entry);

    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "enter maintenance evacuating the diskless VM",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: true,
                resident_vms: vec![vm],
            },
        },
        600,
    )
    .await;
    entry["read_back"] = json!(
        settle(120, || {
            resident(&c).is_some_and(|(state, host)| {
                state == PowerState::Running && host == Some(coordinator)
            }) && c
                .latest()
                .inventory()
                .is_some_and(|i| !i.hosts[&member].enabled)
        })
        .await
    );
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    entry["vm_state"] = json!(resident(&c).map(|(s, h)| json!({"power_state": format!("{s:?}"), "host": h.map(|h| inventory.hosts[&h].name.clone())})));
    entry["member_enabled"] = json!(c.latest().inventory().unwrap().hosts[&member].enabled);
    entry["vm_allowed_actions"] = json!(
        c.latest().inventory().unwrap().vms[&vm]
            .allowed_actions
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>()
    );
    steps.push(entry);
    // A host that reports the VM as a blocker refuses the evacuation before any change; that
    // report is the recorded outcome on a lab whose guests cannot be live-migrated.
    let blocked = steps.last().is_some_and(|e| {
        e["state"] == "Rejected"
            && e["detail"]
                .as_str()
                .is_some_and(|d| d.contains("prevent evacuation"))
    });
    if let Some(last) = steps.last_mut() {
        last["blocked_by_host_report"] = json!(blocked);
    }

    if !blocked {
        let mut entry = resource_op(
            &c,
            "exit maintenance",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || c
                .latest()
                .inventory()
                .is_some_and(|i| i.hosts[&member].enabled))
            .await
        );
        steps.push(entry);
    }
    // Record first, then clean up: a surprising end state must still leave evidence.
    steps.push(json!({"step": "state before cleanup", "vm_state": resident(&c).map(|(s, h)| json!({"power_state": format!("{s:?}"), "host": h.map(|h| inventory.hosts[&h].name.clone())}))}));
    if resident(&c).is_some_and(|(s, _)| s == PowerState::Running) {
        let shutdown = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    action: PowerAction::HardShutdown,
                },
            )
            .expect("hard shutdown accepted");
        let row = wait_vm_outcome(&shutdown, 300).await;
        let halted = settle(120, || {
            resident(&c).is_some_and(|(s, _)| s == PowerState::Halted)
        })
        .await;
        steps.push(json!({"step": "hard shutdown after the evacuation", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": halted}));
    }
    if resident(&c).is_some_and(|(s, _)| s == PowerState::Halted) {
        steps.push(delete_vm_record(&c, vm).await);
    }
    c.close().await;
    lab.record(
        "member-evacuation-of-a-diskless-vm",
        json!({"vm": name, "member": inventory.hosts[&member].name, "coordinator": inventory.hosts[&coordinator].name, "steps": steps}),
    );
    for step in &steps {
        if step["blocked_by_host_report"] == true {
            assert_eq!(step["state"], "Rejected", "{step}");
            assert_eq!(step["member_enabled"], true, "{step}");
            continue;
        }
        if step.get("expected_refusal").is_some() {
            assert!(
                step["accepted"] == false || step["state"] == "Rejected",
                "{step}"
            );
            assert_eq!(step["member_still_enabled"], true, "{step}");
            assert_eq!(step["vm_still_on_member"], true, "{step}");
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P11-02, P11-03: move the halted lab guest, with both of its disks, from the
/// coordinator's local repository to the member's and back through the reviewed same-pool
/// move, verifying the disks' repository placement and the guest's cleared residency after
/// each move, then start the guest where it belongs. A move whose expected host is stale is
/// refused first. Any lab-created snapshot is deleted beforehand because the client refuses
/// storage moves while snapshots exist.
#[tokio::test]
#[ignore]
async fn lab_halted_guest_move_between_local_repositories() {
    use xenadmin_core::{PowerAction, PowerState, VmCommand, VmMigration};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;
    let mut steps = Vec::new();

    // Lab-created snapshots block a storage move; remove them through the reviewed deletion.
    for snapshot in inventory
        .snapshots
        .for_vm(vm)
        .filter(|s| s.name.starts_with("lab-"))
        .map(|s| (s.id, s.name.clone()))
        .collect::<Vec<_>>()
    {
        let current = c.latest().inventory().unwrap().clone();
        let disks = current
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(snapshot.0) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(vm, snapshot.0).unwrap(),
                    disks,
                },
            )
            .expect("leftover snapshot deletion accepted");
        let outcome = wait_vm_outcome(&deletion, 300).await;
        let gone = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.snapshots.for_vm(vm).all(|s| s.id != snapshot.0))
        })
        .await;
        steps.push(json!({"step": "delete leftover snapshot", "name": snapshot.1, "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": gone}));
    }
    if guest.power_state != PowerState::Halted {
        let mut halt = halt_guest(&c, vm).await;
        halt["step"] = json!("halt the guest");
        steps.push(halt);
    }
    let home = c.latest().inventory().unwrap().vms[&vm]
        .host
        .or(Some(coordinator));
    let disks_on = |c: &Connection, sr: xenadmin_core::SrId| {
        c.latest().inventory().is_some_and(|i| {
            let placed = i
                .details
                .vm_disks
                .get(&vm)
                .map(|vbds| {
                    vbds.iter()
                        .filter_map(|b| i.details.vbds.get(b))
                        .filter(|b| b.kind == "Disk")
                        .filter_map(|b| b.vdi)
                        .filter_map(|v| i.details.vdis.get(&v))
                        .map(|v| v.sr == Some(sr))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            placed.len() == 2 && placed.iter().all(|p| *p)
        })
    };
    let halted_plan = |c: &Connection, destination: HostId| -> VmMigration {
        let current = c.latest().inventory().unwrap().clone();
        let mut plan = migration_plan(&current, vm, destination);
        plan.live = false;
        plan.expected_host = current.vms[&vm].host;
        plan.expected_affinity = current.details.vms[&vm].affinity;
        plan
    };
    let member_sr = local_sr_of(&inventory, member).expect("member local repository");
    let coordinator_sr =
        local_sr_of(&inventory, coordinator).expect("coordinator local repository");
    let source_sr = {
        let current = c.latest().inventory().unwrap().clone();
        let plan = migration_plan(&current, vm, member);
        assert_eq!(plan.disks.len(), 2, "expected the image and data disks");
        plan.disks[0].expected_sr
    };
    let (first_destination, first_sr, back_destination, back_sr) = if source_sr == member_sr {
        (coordinator, coordinator_sr, member, member_sr)
    } else {
        (member, member_sr, coordinator, coordinator_sr)
    };
    steps.push(json!({"step": "placement before", "source_repository": source_sr.to_string(), "first_destination": inventory.hosts[&first_destination].name}));

    // A review whose expected host is stale must be refused by the fresh guest read.
    let mut stale = halted_plan(&c, first_destination);
    stale.expected_host = Some(first_destination);
    let refusal = c
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: c.latest().observe_vm(vm).unwrap(),
                operation: stale,
            },
        )
        .map(|op| async move { wait_vm_outcome(&op, 120).await });
    let entry = match refusal {
        Ok(pending) => {
            let row = pending.await;
            json!({"step": "move with a stale expected host", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "expected_refusal": true})
        }
        Err(error) => {
            json!({"step": "move with a stale expected host", "accepted": false, "error": error.to_string(), "expected_refusal": true})
        }
    };
    steps.push(entry);

    for (label, destination, sr) in [
        (
            "move the halted guest to the other repository",
            first_destination,
            first_sr,
        ),
        ("move the halted guest back", back_destination, back_sr),
    ] {
        let plan = halted_plan(&c, destination);
        let started = std::time::Instant::now();
        let migration = c
            .submit(
                OperationId::new(),
                VmCommand::Migrate {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    operation: plan,
                },
            )
            .expect("halted move accepted");
        let (row, progress) = wait_vm_outcome_sampled(&migration, 1800).await;
        let placed = settle(300, || {
            disks_on(&c, sr)
                && c.latest().inventory().is_some_and(|i| {
                    i.vms[&vm].power_state == PowerState::Halted && i.vms[&vm].host.is_none()
                })
        })
        .await;
        steps.push(json!({"step": label, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": placed, "elapsed_ms": started.elapsed().as_millis() as u64, "destination": inventory.hosts[&destination].name, "progress_samples": progress}));
    }
    let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
    start["step"] = json!("start the guest");
    start["resident_host"] = json!(
        c.latest().inventory().unwrap().vms[&vm]
            .host
            .map(|h| inventory.hosts[&h].name.clone())
    );
    start["home_before"] = json!(home.map(|h| inventory.hosts[&h].name.clone()));
    steps.push(start);
    c.close().await;
    lab.record(
        "halted-guest-move-between-local-repositories",
        json!({"guest": guest.name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert!(
                step["accepted"] == false || step["state"] == "Rejected",
                "{step}"
            );
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Submit one organisation change through the pool and wait for its outcome.
async fn organization_op(
    c: &Connection,
    pool: xenadmin_core::PoolId,
    label: &str,
    object: xenadmin_core::OrganizationObjectId,
    change: xenadmin_core::OrganizationChange,
) -> Value {
    use xenadmin_core::{OrganizationEdit, OrganizationPatch, ResourceCommand};
    let patch = OrganizationPatch::new(vec![OrganizationEdit { object, change }]).unwrap();
    let command = ResourceCommand::Organization {
        pool: c
            .latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap(),
        patch,
    };
    resource_op(c, label, command, 120).await
}

/// Submit one vApp operation through the pool and wait for its outcome.
async fn appliance_op(
    c: &Connection,
    pool: xenadmin_core::PoolId,
    label: &str,
    operation: xenadmin_core::ApplianceOperation,
    seconds: u64,
) -> Value {
    let command = xenadmin_core::ResourceCommand::Appliance {
        pool: c
            .latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap(),
        operation,
    };
    resource_op(c, label, command, seconds).await
}

/// P15-01, P15-03, P15-04, P15-05: assign and remove a tag on the lab guest; add a custom
/// field definition to the pool, set and clear its value on the guest and remove the
/// definition; create a vApp, add the guest, set its order and delays, shut the vApp down
/// cleanly and start it again in order, rename the vApp, remove the guest and delete the
/// empty vApp; every change is read back from the inventory.
#[tokio::test]
#[ignore]
async fn lab_tags_custom_fields_and_vapp_round_trip() {
    use xenadmin_core::{
        ApplianceMember, ApplianceOperation, ApplianceOrder, AppliancePower, CustomFieldDefinition,
        CustomFieldType, OrganizationChange, OrganizationEdit, OrganizationObjectId,
        OrganizationPatch, PowerState, ResourceCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let object_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .organization
                .objects
                .iter()
                .find(|o| o.id == OrganizationObjectId::Vm(vm))
                .cloned()
        })
    };
    let definitions_of = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.organization.definitions.clone())
    };
    let object = object_of(&c).expect("the guest's organisation object");
    steps.push(json!({"step": "organisation before", "complete": inventory.details.organization.complete, "tags": object.tags, "folder": object.folder, "custom_fields": object.custom_fields, "definitions": definitions_of(&c).map(|d| d.len())}));

    // Tags.
    let previous_tags = object.tags.clone().unwrap_or_default();
    let tag = format!("lab-tag-{stamp}");
    let mut with_tag = previous_tags.clone();
    with_tag.push(tag.clone());
    for (label, previous, value, present) in [
        (
            "assign a tag",
            previous_tags.clone(),
            with_tag.clone(),
            true,
        ),
        (
            "remove the tag",
            with_tag.clone(),
            previous_tags.clone(),
            false,
        ),
    ] {
        let patch = OrganizationPatch::new(vec![OrganizationEdit {
            object: OrganizationObjectId::Vm(vm),
            change: OrganizationChange::Tags { previous, value },
        }])
        .unwrap();
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Organization {
                pool: pool_target(&c),
                patch,
            },
            120,
        )
        .await;
        entry["read_back"] = json!(
            settle(60, || object_of(&c).is_some_and(|o| o
                .tags
                .as_ref()
                .is_some_and(|t| t.contains(&tag) == present)))
            .await
        );
        steps.push(entry);
    }

    // Custom field definition and value.
    let previous_definitions = definitions_of(&c).unwrap_or_default();
    let definition = CustomFieldDefinition {
        name: format!("lab-field-{stamp}"),
        kind: CustomFieldType::String,
    };
    let mut with_definition = previous_definitions.clone();
    with_definition.push(definition.clone());
    let mut entry = organization_op(
        &c,
        pool,
        "add a custom field definition",
        OrganizationObjectId::Pool(pool),
        OrganizationChange::Definitions {
            previous: previous_definitions.clone(),
            value: with_definition.clone(),
        },
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || definitions_of(&c)
            .is_some_and(|d| d.contains(&definition)))
        .await
    );
    steps.push(entry);
    let mut entry = organization_op(
        &c,
        pool,
        "set the custom field value on the guest",
        OrganizationObjectId::Vm(vm),
        OrganizationChange::CustomField {
            definition: definition.clone(),
            previous: None,
            value: Some("marker".into()),
        },
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || object_of(&c).is_some_and(|o| o
            .custom_fields
            .get(&definition.name)
            .map(String::as_str)
            == Some("marker")))
        .await
    );
    steps.push(entry);
    let mut entry = organization_op(
        &c,
        pool,
        "clear the custom field value",
        OrganizationObjectId::Vm(vm),
        OrganizationChange::CustomField {
            definition: definition.clone(),
            previous: Some("marker".into()),
            value: None,
        },
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || object_of(&c).is_some_and(|o| !o
            .custom_fields
            .contains_key(&definition.name)))
        .await
    );
    steps.push(entry);
    let mut entry = organization_op(
        &c,
        pool,
        "remove the custom field definition",
        OrganizationObjectId::Pool(pool),
        OrganizationChange::Definitions {
            previous: with_definition.clone(),
            value: previous_definitions.clone(),
        },
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || definitions_of(&c)
            .is_some_and(|d| !d.contains(&definition)))
        .await
    );
    steps.push(entry);

    // vApp lifecycle with the guest as its member.
    let appliance_of = |c: &Connection, name: &str| {
        c.latest().inventory().and_then(|i| {
            i.details
                .appliances
                .values()
                .find(|a| a.name == name)
                .cloned()
        })
    };
    let vapp = format!("lab-vapp-{stamp}");
    let mut entry = appliance_op(
        &c,
        pool,
        "create an empty vApp",
        ApplianceOperation::Create {
            name: vapp.clone(),
            description: "vApp for the live organisation scenario".into(),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(settle(60, || appliance_of(&c, &vapp).is_some()).await);
    steps.push(entry);
    let appliance = appliance_of(&c, &vapp).expect("created vApp").id;
    let member_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vms
                .get(&vm)
                .map(|d| (d.appliance, d.appliance_order))
        })
    };
    let mut entry = appliance_op(
        &c,
        pool,
        "add the guest to the vApp",
        ApplianceOperation::Membership {
            appliance,
            expected_members: Vec::new(),
            vm,
            join: true,
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || member_of(&c)
            .is_some_and(|(a, _)| a == Some(appliance))
            && appliance_of(&c, &vapp).is_some_and(|a| a.vms.contains(&vm)))
        .await
    );
    steps.push(entry);
    let previous_order = member_of(&c)
        .and_then(|(_, o)| o)
        .unwrap_or(ApplianceOrder {
            order: 0,
            start_delay: 0,
            shutdown_delay: 0,
        });
    // A value other than the one found: the client refuses an edit that changes nothing,
    // and a run before this one restored its values may have left the same ones behind.
    let order = [(1, 5), (2, 7)]
        .into_iter()
        .map(|(order, delay)| ApplianceOrder {
            order,
            start_delay: delay,
            shutdown_delay: delay,
        })
        .find(|candidate| *candidate != previous_order)
        .unwrap();
    let mut entry = appliance_op(
        &c,
        pool,
        "set the guest's vApp order and delays",
        ApplianceOperation::SetOrder {
            appliance,
            vm,
            previous: previous_order,
            value: order,
        },
        120,
    )
    .await;
    entry["read_back"] =
        json!(settle(60, || member_of(&c).is_some_and(|(_, o)| o == Some(order))).await);
    steps.push(entry);
    let current_state = c.latest().inventory().unwrap().vms[&vm].power_state.clone();
    if current_state == PowerState::Running {
        let mut entry = appliance_op(
            &c,
            pool,
            "shut the vApp down cleanly",
            ApplianceOperation::Power {
                appliance,
                members: vec![ApplianceMember {
                    vm,
                    state: PowerState::Running,
                    order,
                }],
                action: AppliancePower::CleanShutdown,
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(180, || c
                .latest()
                .inventory()
                .is_some_and(|i| i.vms[&vm].power_state == PowerState::Halted))
            .await
        );
        steps.push(entry);
    } else {
        steps.push(
            json!({"step": "vApp shutdown", "skipped": format!("guest was {current_state:?}")}),
        );
    }
    let started = std::time::Instant::now();
    let mut entry = appliance_op(
        &c,
        pool,
        "start the vApp in order",
        ApplianceOperation::Power {
            appliance,
            members: vec![ApplianceMember {
                vm,
                state: PowerState::Halted,
                order,
            }],
            action: AppliancePower::Start,
        },
        600,
    )
    .await;
    entry["read_back"] = json!(
        settle(180, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.vms[&vm].power_state == PowerState::Running))
        .await
    );
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    steps.push(entry);
    let renamed = format!("{vapp}-renamed");
    let mut entry = appliance_op(
        &c,
        pool,
        "rename the vApp",
        ApplianceOperation::Rename {
            appliance,
            previous_name: vapp.clone(),
            previous_description: "vApp for the live organisation scenario".into(),
            name: renamed.clone(),
            description: "Renamed by the live organisation scenario".into(),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(settle(60, || appliance_of(&c, &renamed).is_some()).await);
    steps.push(entry);
    let mut entry = appliance_op(
        &c,
        pool,
        "put the guest's vApp order and delays back",
        ApplianceOperation::SetOrder {
            appliance,
            vm,
            previous: order,
            value: previous_order,
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || member_of(&c)
            .is_some_and(|(_, o)| o == Some(previous_order)))
        .await
    );
    steps.push(entry);
    let mut entry = appliance_op(
        &c,
        pool,
        "remove the guest from the vApp",
        ApplianceOperation::Membership {
            appliance,
            expected_members: vec![vm],
            vm,
            join: false,
        },
        120,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || member_of(&c).is_some_and(|(a, _)| a.is_none())
            && appliance_of(&c, &renamed).is_some_and(|a| a.vms.is_empty()))
        .await
    );
    steps.push(entry);
    let mut entry = appliance_op(
        &c,
        pool,
        "delete the empty vApp",
        ApplianceOperation::DeleteEmpty {
            appliance,
            expected_name: renamed.clone(),
        },
        120,
    )
    .await;
    entry["read_back"] = json!(settle(60, || appliance_of(&c, &renamed).is_none()).await);
    steps.push(entry);
    c.close().await;
    lab.record(
        "tags-custom-fields-and-vapp-round-trip",
        json!({"guest": guest.name, "tag": tag, "field": definition.name, "vapp": vapp, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P13-13: with TLS verification already enabled on the lab pool, the reviewed enable is
/// refused by the fresh pool read; the pool secret is rotated with every host live and HA
/// disabled, confirmed by the pool reporting no pending rotation, and the connection keeps
/// serving a fresh inventory afterwards.
#[tokio::test]
#[ignore]
async fn lab_pool_secret_rotation_and_tls_verification_state() {
    use xenadmin_core::{PoolOperation, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let administration = inventory
        .details
        .pool
        .as_ref()
        .map(|p| p.administration.clone())
        .expect("pool administration");
    let mut steps = Vec::new();
    steps.push(json!({"step": "pool security state", "tls_verification_enabled": administration.tls_verification_enabled, "secret_rotation_pending": administration.secret_rotation_pending, "ha_enabled": administration.ha_enabled, "hosts": inventory.hosts.len()}));
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let administration_of = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pool.as_ref().map(|p| p.administration.clone()))
    };

    let mut entry = resource_op(
        &c,
        "enable TLS verification",
        ResourceCommand::Pool {
            target: pool_target(&c),
            operation: PoolOperation::EnableTlsVerification,
        },
        180,
    )
    .await;
    if administration.tls_verification_enabled == Some(true) {
        // Already enabled: the reviewed enable must be refused by the fresh pool read.
        entry["expected_refusal"] = json!(true);
    } else {
        entry["read_back"] = json!(
            settle(120, || administration_of(&c)
                .is_some_and(|a| a.tls_verification_enabled == Some(true)))
            .await
        );
    }
    steps.push(entry);

    let revision_before = inventory.revision;
    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "rotate the pool secret",
        ResourceCommand::Pool {
            target: pool_target(&c),
            operation: PoolOperation::RotateSecret,
        },
        600,
    )
    .await;
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    entry["read_back"] = json!(
        settle(120, || administration_of(&c)
            .is_some_and(|a| a.secret_rotation_pending == Some(false)))
        .await
    );
    // The connection keeps serving fresh inventory after the rotation.
    entry["inventory_advanced"] = json!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.revision > revision_before))
        .await
    );
    entry["connection_current"] = json!(c.latest().is_current());
    entry["hosts_live"] = json!(
        c.latest()
            .inventory()
            .map(|i| i.hosts.values().filter(|h| h.enabled).count())
    );
    steps.push(entry);
    c.close().await;
    lab.record(
        "pool-secret-rotation-and-tls-verification-state",
        json!({"pool": inventory.pool_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert!(
                step["accepted"] == false || step["state"] == "Rejected",
                "{step}"
            );
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
        if step.get("inventory_advanced").is_some() {
            assert_eq!(step["inventory_advanced"], true, "{step}");
            assert_eq!(step["connection_current"], true, "{step}");
        }
    }
}

/// P15-02, P14-10, P15-06: place the lab guest in a folder, move it to another folder and
/// take it out, register and remove an empty folder; refuse a message dismissal whose review
/// digest is stale and dismiss the pool's oldest server message (or record that none exist);
/// apply a two-VM description batch over the running guest and a halted diskless VM, read
/// both back and restore them with a second batch.
#[tokio::test]
#[ignore]
async fn lab_folders_message_dismissal_and_bulk_description_batch() {
    use std::collections::BTreeSet;
    use xenadmin_core::{
        BatchVmProperties, OrganizationChange, OrganizationObjectId, PowerState, ResourceCommand,
        VmPropertiesBatch, VmSettingEdit, VmSettingsPatch,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let object_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .organization
                .objects
                .iter()
                .find(|o| o.id == OrganizationObjectId::Vm(vm))
                .cloned()
        })
    };

    // Folders.
    let original_folder = object_of(&c).and_then(|o| o.folder);
    let first = format!("/lab/folder-{stamp}");
    let second = format!("/lab/moved-{stamp}");
    for (label, previous, value) in [
        (
            "place the guest in a folder",
            original_folder.clone(),
            Some(first.clone()),
        ),
        (
            "move the guest to another folder",
            Some(first.clone()),
            Some(second.clone()),
        ),
        (
            "take the guest out of its folder",
            Some(second.clone()),
            original_folder.clone(),
        ),
    ] {
        let mut entry = organization_op(
            &c,
            pool,
            label,
            OrganizationObjectId::Vm(vm),
            OrganizationChange::Folder {
                previous,
                value: value.clone(),
            },
        )
        .await;
        entry["read_back"] =
            json!(settle(60, || object_of(&c).is_some_and(|o| o.folder == value)).await);
        steps.push(entry);
    }
    let empty_folders_of = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.organization.empty_folders.clone())
    };
    let empty_previous = empty_folders_of(&c).unwrap_or_default();
    let empty = format!("/lab/empty-{stamp}");
    let mut with_empty = empty_previous.clone();
    with_empty.push(empty.clone());
    for (label, previous, value, present) in [
        (
            "register an empty folder",
            empty_previous.clone(),
            with_empty.clone(),
            true,
        ),
        (
            "remove the empty folder",
            with_empty.clone(),
            empty_previous.clone(),
            false,
        ),
    ] {
        let mut entry = organization_op(
            &c,
            pool,
            label,
            OrganizationObjectId::Pool(pool),
            OrganizationChange::EmptyFolders { previous, value },
        )
        .await;
        entry["read_back"] = json!(
            settle(60, || empty_folders_of(&c)
                .is_some_and(|f| f.contains(&empty) == present))
            .await
        );
        steps.push(entry);
    }

    // Server messages.
    let messages = c.latest().inventory().unwrap().details.messages.clone();
    steps.push(json!({"step": "server messages", "count": messages.len(), "names": messages.values().map(|m| m.name.clone()).collect::<BTreeSet<_>>()}));
    if let Some(oldest) = messages
        .values()
        .min_by_key(|m| m.timestamp.clone())
        .cloned()
    {
        let target = |c: &Connection| {
            c.latest()
                .observe_resource(ResourceTarget::Message(oldest.id))
                .unwrap()
        };
        let mut entry = resource_op(
            &c,
            "dismiss a message with a stale review digest",
            ResourceCommand::DismissMessage {
                message: target(&c),
                expected_digest: [0u8; 32],
            },
            60,
        )
        .await;
        entry["expected_refusal"] = json!(true);
        entry["still_present"] = json!(
            c.latest()
                .inventory()
                .is_some_and(|i| i.details.messages.contains_key(&oldest.id))
        );
        steps.push(entry);
        let mut entry = resource_op(
            &c,
            "dismiss the oldest message",
            ResourceCommand::DismissMessage {
                message: target(&c),
                expected_digest: oldest.review_digest(),
            },
            120,
        )
        .await;
        entry["message"] = json!({"name": oldest.name, "class": oldest.object_class, "timestamp": oldest.timestamp});
        entry["read_back"] = json!(
            settle(60, || c
                .latest()
                .inventory()
                .is_some_and(|i| !i.details.messages.contains_key(&oldest.id)))
            .await
        );
        steps.push(entry);
    } else {
        steps.push(
            json!({"step": "message dismissal", "skipped": "the pool has no server messages"}),
        );
    }

    // Two-VM description batch.
    let (other, created) = create_diskless_vm(
        &c,
        &inventory,
        &format!("lab-batch-{stamp}"),
        "Disposable diskless VM for the bulk properties scenario",
    )
    .await;
    steps.push(created);
    let description_of = |c: &Connection, id: xenadmin_core::VmId| {
        c.latest()
            .inventory()
            .and_then(|i| i.vms.get(&id).map(|v| v.description.clone()))
    };
    let guest_state = c.latest().inventory().unwrap().vms[&vm].power_state.clone();
    let previous = (
        description_of(&c, vm).expect("guest description"),
        description_of(&c, other).expect("diskless VM description"),
    );
    let marker = format!("lab-batch-{stamp}");
    let selection = |from: (String, String), to: (String, String)| VmPropertiesBatch {
        vms: vec![
            BatchVmProperties {
                vm,
                expected_state: guest_state.clone(),
                patch: VmSettingsPatch::new(vec![VmSettingEdit::Description {
                    previous: from.0,
                    value: to.0,
                }])
                .unwrap(),
            },
            BatchVmProperties {
                vm: other,
                expected_state: PowerState::Halted,
                patch: VmSettingsPatch::new(vec![VmSettingEdit::Description {
                    previous: from.1,
                    value: to.1,
                }])
                .unwrap(),
            },
        ],
    };
    let mut entry = resource_op(
        &c,
        "batch description edit over two VMs",
        ResourceCommand::PropertiesBatch {
            pool: pool_target(&c),
            selection: selection(previous.clone(), (marker.clone(), marker.clone())),
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || description_of(&c, vm).as_deref()
            == Some(marker.as_str())
            && description_of(&c, other).as_deref()
                == Some(marker.as_str()))
        .await
    );
    steps.push(entry);
    let mut entry = resource_op(
        &c,
        "batch restore of both descriptions",
        ResourceCommand::PropertiesBatch {
            pool: pool_target(&c),
            selection: selection((marker.clone(), marker.clone()), previous.clone()),
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || description_of(&c, vm).as_deref()
            == Some(previous.0.as_str())
            && description_of(&c, other).as_deref()
                == Some(previous.1.as_str()))
        .await
    );
    steps.push(entry);
    steps.push(delete_vm_record(&c, other).await);
    c.close().await;
    lab.record(
        "folders-message-dismissal-and-bulk-description-batch",
        json!({"guest": guest.name, "folders": [first, second, empty], "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert!(
                step["accepted"] == false || step["state"] == "Rejected",
                "{step}"
            );
            assert_eq!(step["still_present"], true, "{step}");
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P11-03, P05-01: with the guest running, take a disk snapshot, submit a live migration
/// with storage relocation to the member and confirm the client refuses it as an
/// unsupported topology while snapshots exist, leaving the guest, its disks and the pool's
/// task list untouched; then delete the snapshot through the reviewed deletion.
#[tokio::test]
#[ignore]
async fn lab_storage_migration_refused_while_a_snapshot_exists() {
    use xenadmin_core::{PowerAction, PowerState, SnapshotName, VmCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    if guest.power_state != PowerState::Running {
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the guest");
        steps.push(start);
    }
    let current = c.latest().inventory().unwrap().clone();
    let source_host = current.vms[&vm].host.expect("resident host");
    let destination = current
        .hosts
        .keys()
        .copied()
        .find(|h| *h != source_host && current.hosts[h].enabled)
        .expect("another enabled host");
    let disk_srs = |c: &Connection| {
        c.latest().inventory().map(|i| {
            i.details
                .vm_disks
                .get(&vm)
                .map(|vbds| {
                    vbds.iter()
                        .filter_map(|b| i.details.vbds.get(b))
                        .filter(|b| b.kind == "Disk")
                        .filter_map(|b| b.vdi)
                        .filter_map(|v| i.details.vdis.get(&v).and_then(|d| d.sr))
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    };
    let srs_before = disk_srs(&c);
    let tasks_before = current.details.tasks.len();

    // Snapshot.
    let snapshot_name = format!("lab-topology-{stamp}");
    let snapshot = c
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: c.latest().observe_vm(vm).unwrap(),
                name: SnapshotName::new(&snapshot_name).unwrap(),
            },
        )
        .expect("snapshot accepted");
    let row = wait_vm_outcome(&snapshot, 300).await;
    let listed = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.snapshots.for_vm(vm).any(|s| s.name == snapshot_name))
    })
    .await;
    steps.push(json!({"step": "take a disk snapshot", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": listed}));
    let snapshot_id = c
        .latest()
        .inventory()
        .unwrap()
        .snapshots
        .for_vm(vm)
        .find(|s| s.name == snapshot_name)
        .map(|s| s.id)
        .expect("snapshot listed");

    // The storage move must be refused while the snapshot exists.
    let plan = migration_plan(&c.latest().inventory().unwrap().clone(), vm, destination);
    let migration = c
        .submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: c.latest().observe_vm(vm).unwrap(),
                operation: plan,
            },
        )
        .expect("migration review accepted");
    let row = wait_vm_outcome(&migration, 300).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let after = c.latest().inventory().unwrap().clone();
    steps.push(json!({"step": "live migration with storage relocation while a snapshot exists", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "expected_refusal": true,
        "host_unchanged": after.vms[&vm].host == Some(source_host), "disks_unchanged": disk_srs(&c) == srs_before, "running": after.vms[&vm].power_state == PowerState::Running, "tasks_before": tasks_before, "tasks_after": after.details.tasks.len()}));

    // Delete the snapshot through the reviewed deletion.
    let disks = after
        .details
        .vbds
        .values()
        .filter(|b| b.snapshot == Some(snapshot_id) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect::<Vec<_>>();
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(vm, snapshot_id).unwrap(),
                disks,
            },
        )
        .expect("snapshot deletion accepted");
    let row = wait_vm_outcome(&deletion, 300).await;
    let gone = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.snapshots.for_vm(vm).all(|s| s.id != snapshot_id))
    })
    .await;
    steps.push(json!({"step": "delete the snapshot", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": gone}));
    c.close().await;
    lab.record(
        "storage-migration-refused-while-a-snapshot-exists",
        json!({"guest": guest.name, "snapshot": snapshot_name, "destination": inventory.hosts[&destination].name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert_eq!(step["state"], "Rejected", "{step}");
            assert!(
                step["detail"]
                    .as_str()
                    .is_some_and(|d| d.contains("not supported")),
                "{step}"
            );
            assert_eq!(step["host_unchanged"], true, "{step}");
            assert_eq!(step["disks_unchanged"], true, "{step}");
            assert_eq!(step["running"], true, "{step}");
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P11-07, P05-01: take a disk snapshot of the running guest, export that snapshot as an
/// uncompressed XVA into the lab's private transfer directory with the client's checksum and
/// identity verification, record the archive size, then delete the snapshot and the archive.
#[tokio::test]
#[ignore]
async fn lab_snapshot_xva_export() {
    use xenadmin_core::{
        ExportSource, PowerAction, PowerState, SnapshotName, TransferAction, TransferFile,
        TransferRequest, VmCommand, XvaCompression,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    if guest.power_state != PowerState::Running {
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the guest");
        steps.push(start);
    }
    let snapshot_name = format!("lab-export-{stamp}");
    let snapshot = c
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: c.latest().observe_vm(vm).unwrap(),
                name: SnapshotName::new(&snapshot_name).unwrap(),
            },
        )
        .expect("snapshot accepted");
    let row = wait_vm_outcome(&snapshot, 300).await;
    let listed = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.snapshots.for_vm(vm).any(|s| s.name == snapshot_name))
    })
    .await;
    steps.push(json!({"step": "take a disk snapshot", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": listed}));
    let snapshot_id = c
        .latest()
        .inventory()
        .unwrap()
        .snapshots
        .for_vm(vm)
        .find(|s| s.name == snapshot_name)
        .map(|s| s.id)
        .expect("snapshot listed");

    let directory = lab.evidence.parent().unwrap().join("transfers");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{snapshot_name}.xva"));
    let started = std::time::Instant::now();
    let export = c
        .transfer(
            OperationId::new(),
            c.latest()
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .unwrap(),
            TransferRequest {
                action: TransferAction::ExportXva {
                    source: ExportSource::Snapshot {
                        snapshot: snapshot_id,
                        parent: vm,
                    },
                    compression: XvaCompression::Uncompressed,
                },
                file: TransferFile::destination(&path).expect("export destination"),
            },
        )
        .expect("snapshot export accepted");
    let outcome = wait_outcome(&export, 1800).await;
    let size = std::fs::metadata(&path).map(|m| m.len()).ok();
    steps.push(json!({"step": "export the snapshot as an uncompressed XVA", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "bytes": size, "read_back": size.is_some_and(|s| s > 0),
        "steps": outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()}));
    let _ = std::fs::remove_file(&path);

    let current = c.latest().inventory().unwrap().clone();
    let disks = current
        .details
        .vbds
        .values()
        .filter(|b| b.snapshot == Some(snapshot_id) && b.kind == "Disk")
        .filter_map(|b| b.vdi)
        .collect::<Vec<_>>();
    let deletion = c
        .submit(
            OperationId::new(),
            VmCommand::DeleteSnapshot {
                snapshot: c.latest().observe_snapshot(vm, snapshot_id).unwrap(),
                disks,
            },
        )
        .expect("snapshot deletion accepted");
    let row = wait_vm_outcome(&deletion, 300).await;
    let gone = settle(60, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.snapshots.for_vm(vm).all(|s| s.id != snapshot_id))
    })
    .await;
    steps.push(json!({"step": "delete the snapshot", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": gone}));
    c.close().await;
    lab.record(
        "snapshot-xva-export",
        json!({"guest": guest.name, "snapshot": snapshot_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P15-06, P05-01: take a disk snapshot and a memory checkpoint of the running guest and
/// export both in one reviewed batch as uncompressed XVA into the lab's private transfer
/// directory, the checkpoint with its memory chosen for that item. Each archive is then read
/// back with the client's own inspection: one guest each, under the snapshot's UUID, with one
/// memory image in the checkpoint's archive and none in the disk snapshot's. The archives,
/// the snapshot and the checkpoint are removed afterwards.
#[tokio::test]
#[ignore]
async fn lab_snapshot_batch_xva_export() {
    use xenadmin_core::{
        BatchVmWorkflow, ExportSource, PowerAction, PowerState, SnapshotName, TransferAction,
        TransferFile, VmCommand, VmExportFormat, VmWorkflowBatch, XvaCompression,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .expect("lab guest")
        .clone();
    let vm = guest.id;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    if guest.power_state != PowerState::Running {
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the guest");
        steps.push(start);
    }
    let listed = |name: &str| {
        c.latest().inventory().and_then(|i| {
            i.snapshots
                .for_vm(vm)
                .find(|s| s.name == name)
                .map(|s| s.id)
        })
    };
    let snapshot_name = format!("lab-batch-snapshot-{stamp}");
    let snapshot = c
        .submit(
            OperationId::new(),
            VmCommand::CreateSnapshot {
                vm: c.latest().observe_vm(vm).unwrap(),
                name: SnapshotName::new(&snapshot_name).unwrap(),
            },
        )
        .expect("snapshot accepted");
    let row = wait_vm_outcome(&snapshot, 300).await;
    let read_back = settle(60, || listed(&snapshot_name).is_some()).await;
    steps.push(json!({"step": "take a disk snapshot", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": read_back}));
    let checkpoint_name = format!("lab-batch-checkpoint-{stamp}");
    let checkpoint = c
        .submit(
            OperationId::new(),
            VmCommand::CreateCheckpoint {
                vm: c.latest().observe_vm(vm).unwrap(),
                name: SnapshotName::new(&checkpoint_name).unwrap(),
            },
        )
        .expect("checkpoint accepted");
    let row = wait_vm_outcome(&checkpoint, 300).await;
    let read_back = settle(60, || listed(&checkpoint_name).is_some()).await;
    steps.push(json!({"step": "take a memory checkpoint", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": read_back}));
    let snapshot_id = listed(&snapshot_name).expect("snapshot listed");
    let checkpoint_id = listed(&checkpoint_name).expect("checkpoint listed");

    // What a review shows: the checkpoint's memory image as the inventory has it, with
    // its memory chosen, and the availability of each single export.
    let reviewed = |preserve_memory: bool| {
        c.latest()
            .inventory()
            .and_then(|i| ExportSource::checkpoint(i, checkpoint_id, vm, preserve_memory))
    };
    let memory_known = settle(120, || reviewed(true).is_some()).await;
    // Without a memory image to review there is nothing to export; what was created is
    // still removed below, and the missing review fails the scenario at the end.
    let sources = [
        Some(ExportSource::Snapshot {
            snapshot: snapshot_id,
            parent: vm,
        }),
        reviewed(true),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let current = c.latest().inventory().unwrap().clone();
    let available = sources
        .iter()
        .map(|source| {
            TransferAction::ExportXva {
                source: source.clone(),
                compression: XvaCompression::Uncompressed,
            }
            .availability(&current)
            .explanation()
        })
        .collect::<Vec<_>>();
    let reviewable = sources.len() == 2 && available.iter().all(|a| a == "Available");
    steps.push(json!({"step": "review both sources", "memory_image_listed": memory_known, "availability": available,
        "read_back": reviewable}));
    let format = VmExportFormat::Xva {
        compression: XvaCompression::Uncompressed,
    };
    let batch = VmWorkflowBatch {
        items: sources
            .iter()
            .map(|source| BatchVmWorkflow::snapshot_export(source, format.clone()).unwrap())
            .collect(),
    };
    let directory = lab.evidence.parent().unwrap().join("transfers");
    std::fs::create_dir_all(&directory).unwrap();
    let paths =
        [&snapshot_name, &checkpoint_name].map(|name| directory.join(format!("{name}.xva")));
    let outputs = [snapshot_id, checkpoint_id]
        .into_iter()
        .zip(&paths)
        .filter_map(|(id, path)| Some((id, TransferFile::destination(path).ok()?)))
        .collect::<Vec<_>>();
    let started = std::time::Instant::now();
    let export = if reviewable && outputs.len() == 2 {
        c.submit_snapshot_export_batch(
            OperationId::new(),
            c.latest()
                .observe_resource(ResourceTarget::Pool(current.pool_id))
                .unwrap(),
            batch,
            outputs,
        )
        .map_err(|error| error.to_string())
    } else {
        Err("the review or a destination was not available".to_owned())
    };
    match export {
        Ok(export) => {
            let outcome = wait_outcome(&export, 3600).await;
            let elapsed_ms = started.elapsed().as_millis() as u64;
            // Read each archive back whole, as an import would before it is offered.
            let archives = paths
                .iter()
                .map(|path| match TransferFile::source(path) {
                    Ok(file) => {
                        let archive = file.archive().expect("an XVA archive");
                        json!({"bytes": file.size(), "guests": archive.guests.len(),
                            "guest": archive.guests.first().map(|g| g.uuid.to_string()),
                            "memory_images": archive.memory_images(), "data_disks": archive.data_disks})
                    }
                    Err(error) => json!({"error": error.to_string()}),
                })
                .collect::<Vec<_>>();
            let verified = [(snapshot_id, 0), (checkpoint_id, 1)]
                .iter()
                .zip(&archives)
                .all(|((id, memory_images), archive)| {
                    archive["guests"] == 1
                        && archive["guest"] == id.to_string()
                        && archive["memory_images"] == *memory_images
                });
            steps.push(json!({"step": "export the snapshot and the checkpoint with its memory in one batch", "accepted": true,
                "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": elapsed_ms,
                "children": outcome.workflow_batch.iter().map(|r| json!({"child": r.child.to_string(), "state": r.state.as_ref().map(|s| format!("{s:?}")), "detail": r.detail})).collect::<Vec<_>>(),
                "steps": outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>(),
                "archives": archives, "read_back": verified}));
        }
        Err(error) => steps.push(json!({"step": "export the snapshot and the checkpoint with its memory in one batch", "accepted": false, "state": "Not submitted", "detail": error})),
    }
    for path in &paths {
        let _ = std::fs::remove_file(path);
    }
    let removed = paths.iter().all(|path| !path.exists());
    steps.push(json!({"step": "remove the archives", "read_back": removed}));

    for (label, id) in [("snapshot", snapshot_id), ("checkpoint", checkpoint_id)] {
        let disks = c
            .latest()
            .inventory()
            .unwrap()
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(id) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(vm, id).unwrap(),
                    disks,
                },
            )
            .expect("deletion accepted");
        let row = wait_vm_outcome(&deletion, 300).await;
        let gone = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.snapshots.get(&id).is_none())
        })
        .await;
        steps.push(json!({"step": format!("delete the {label}"), "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": gone}));
    }
    c.close().await;
    lab.record(
        "snapshot-batch-xva-export",
        json!({"guest": guest.name, "snapshot": snapshot_name, "checkpoint": checkpoint_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P15-06, failure column of the batch of snapshot exports: Stop and an interruption. A
/// memory checkpoint and two disk snapshots of the running guest are taken. A first batch
/// exports the checkpoint with its memory, which takes long enough to act on, and the two
/// snapshots; Stop is asked for during the first export, which must finish and be the only
/// archive. A second batch, the checkpoint again and one snapshot, is cut off by the
/// application closing during its first export. On restart the batch is unknown and a
/// recheck reads what there is: nothing is downloaded again, nothing is published and the
/// second export is never started. Whatever is left unknown is acknowledged, and a third
/// batch of one snapshot shows the pool is free again. Everything made is removed.
#[tokio::test]
#[ignore]
async fn lab_snapshot_batch_stop_and_interruption() {
    use xenadmin_core::{
        BatchVmWorkflow, ExportSource, OperationControl, PowerAction, PowerState, SnapshotName,
        TransferFile, VmCommand, VmExportFormat, VmWorkflowBatch, XvaCompression,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let directory = lab.evidence.parent().unwrap().join("transfers");
    std::fs::create_dir_all(&directory).unwrap();
    let format = VmExportFormat::Xva {
        compression: XvaCompression::Uncompressed,
    };
    let mut steps = Vec::new();
    let mut failures = Vec::new();
    let names = [
        format!("lab-stop-checkpoint-{stamp}"),
        format!("lab-stop-snapshot-a-{stamp}"),
        format!("lab-stop-snapshot-b-{stamp}"),
    ];
    let path = |leg: &str, name: &str| directory.join(format!("{name}-{leg}.xva"));
    let children = |row: &xenadmin_core::ResourceOperationRecord| {
        row.workflow_batch
            .iter()
            .map(|r| json!({"state": r.state.as_ref().map(|s| format!("{s:?}")), "detail": r.detail}))
            .collect::<Vec<_>>()
    };

    // First process: the snapshots, the stopped batch, and the batch that is cut off.
    let interrupted = OperationId::new();
    let (vm, ids) = {
        let backend = Backend::with_state_directory(state.path().into()).unwrap();
        let c = connect(&lab, &backend, 0, false).await;
        let guest = c
            .latest()
            .inventory()
            .unwrap()
            .guests()
            .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
            .expect("lab guest")
            .clone();
        let vm = guest.id;
        if guest.power_state != PowerState::Running {
            power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        }
        let listed = |c: &Connection, name: &str| {
            c.latest().inventory().and_then(|i| {
                i.snapshots
                    .for_vm(vm)
                    .find(|s| s.name == name)
                    .map(|s| s.id)
            })
        };
        for (index, name) in names.iter().enumerate() {
            let observed = c.latest().observe_vm(vm).unwrap();
            let name = SnapshotName::new(name).unwrap();
            let command = if index == 0 {
                VmCommand::CreateCheckpoint { vm: observed, name }
            } else {
                VmCommand::CreateSnapshot { vm: observed, name }
            };
            let row = wait_vm_outcome(
                &c.submit(OperationId::new(), command).expect("accepted"),
                300,
            )
            .await;
            let seen = settle(60, || listed(&c, &names[index]).is_some()).await;
            assert!(
                row.state == OperationState::Succeeded && seen,
                "{} was not taken: {}",
                names[index],
                row.detail
            );
        }
        let ids = names
            .iter()
            .map(|n| listed(&c, n).expect("listed"))
            .collect::<Vec<_>>();
        let memory_known = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| ExportSource::checkpoint(i, ids[0], vm, true).is_some())
        })
        .await;
        assert!(memory_known, "the checkpoint's memory image is not listed");
        let source = |c: &Connection, index: usize| {
            if index == 0 {
                ExportSource::checkpoint(c.latest().inventory().unwrap(), ids[0], vm, true).unwrap()
            } else {
                ExportSource::Snapshot {
                    snapshot: ids[index],
                    parent: vm,
                }
            }
        };
        let submit = |c: &Connection, id: OperationId, leg: &str, which: &[usize]| {
            let batch = VmWorkflowBatch {
                items: which
                    .iter()
                    .map(|i| {
                        BatchVmWorkflow::snapshot_export(&source(c, *i), format.clone()).unwrap()
                    })
                    .collect(),
            };
            let outputs = which
                .iter()
                .map(|i| {
                    (
                        ids[*i],
                        TransferFile::destination(path(leg, &names[*i])).unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let pool = c.latest().inventory().unwrap().pool_id;
            c.submit_snapshot_export_batch(
                id,
                c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                batch,
                outputs,
            )
            .expect("batch accepted")
        };
        let first_under_way = |operation: &xenadmin_core::ResourceOperation| {
            operation.latest().workflow_batch.first().is_some_and(|r| {
                r.state
                    .as_ref()
                    .is_some_and(|s| *s == OperationState::Tracking)
            })
        };

        // Stop during the first export.
        let stopped = submit(&c, OperationId::new(), "stop", &[0, 1, 2]);
        let under_way = settle(120, || first_under_way(&stopped)).await;
        let asked = c
            .control_resource(stopped.latest().id, OperationControl::RequestCancellation)
            .map(|_| true)
            .map_err(|e| e.to_string());
        let row = wait_outcome(&stopped, 1800).await;
        let files = [0, 1, 2].map(|i| path("stop", &names[i]).exists());
        steps.push(json!({"step": "stop a batch of three during its first export", "first_export_under_way": under_way, "asked": asked,
            "state": format!("{:?}", row.state), "detail": row.detail, "children": children(&row), "archives_present": files}));
        if !(under_way
            && files == [true, false, false]
            && row.workflow_batch[0].state == Some(OperationState::Succeeded)
            && row.workflow_batch[1..].iter().all(|r| r.state.is_none()))
        {
            failures.push(format!(
                "Stop: {:?}: {}; archives {files:?}",
                row.state, row.detail
            ));
        }

        // Cut off during the first export of a second batch.
        let cut = submit(&c, interrupted, "cut", &[0, 1]);
        let under_way = settle(120, || first_under_way(&cut)).await;
        // Let some of the archive arrive, so that there is a transfer to cut.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state_at_close = format!("{:?}", cut.latest().state);
        let children_at_close = children(&cut.latest());
        c.close().await;
        drop(backend);
        steps.push(json!({"step": "close the application during the first export of a batch of two", "first_export_under_way": under_way,
            "state_at_close": state_at_close, "children_at_close": children_at_close}));
        if !under_way {
            failures.push("the second batch was not under way when the application closed".into());
        }
        (vm, ids)
    };

    // Second process, same state directory.
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let row =
        |c: &Connection, id: OperationId| c.resource_history().iter().find(|r| r.id == id).cloned();
    let recovered = settle(120, || {
        row(&c, interrupted)
            .is_some_and(|r| r.state.terminal() || r.state == OperationState::OutcomeUnknown)
    })
    .await;
    let after_restart = row(&c, interrupted);
    let present_after_restart = [0, 1].map(|i| path("cut", &names[i]).exists());
    steps.push(json!({"step": "after the restart", "record_found": after_restart.is_some(), "settled": recovered,
        "state": after_restart.as_ref().map(|r| format!("{:?}", r.state)), "detail": after_restart.as_ref().map(|r| r.detail.clone()),
        "children": after_restart.as_ref().map(&children),
        "archives_present": present_after_restart}));
    let mut last = after_restart.clone();
    let mut rechecks = Vec::new();
    while last
        .as_ref()
        .is_some_and(|r| r.state == OperationState::OutcomeUnknown)
        && rechecks.len() < 6
    {
        let before = last
            .as_ref()
            .map(|r| (format!("{:?}", r.state), r.detail.clone(), children(r)));
        let asked = c
            .control_resource(interrupted, OperationControl::Recheck)
            .map(|_| true)
            .map_err(|e| e.to_string());
        let moved = settle(60, || {
            row(&c, interrupted).map(|r| (format!("{:?}", r.state), r.detail.clone(), children(&r)))
                != before
        })
        .await;
        settle(120, || {
            row(&c, interrupted)
                .is_some_and(|r| r.state.terminal() || r.state == OperationState::OutcomeUnknown)
        })
        .await;
        last = row(&c, interrupted);
        rechecks.push(json!({"asked": asked, "moved": moved, "state": last.as_ref().map(|r| format!("{:?}", r.state)),
            "detail": last.as_ref().map(|r| r.detail.clone()), "children": last.as_ref().map(&children)}));
        if !moved {
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let published = [0, 1].map(|i| path("cut", &names[i]).exists());
    steps.push(json!({"step": "recheck the interrupted batch", "rechecks": rechecks, "archives_present": published}));
    match &last {
        Some(r) => {
            // Nothing is published by a recheck, and the second export was never started.
            if published != [false, false] {
                failures.push(format!(
                    "an archive appeared after the interruption: {published:?}"
                ));
            }
            if r.workflow_batch
                .get(1)
                .is_some_and(|child| child.state.is_some())
            {
                failures.push(format!("the second export was started: {:?}", children(r)));
            }
        }
        None => failures.push("the interrupted batch has no record after the restart".into()),
    }
    if last
        .as_ref()
        .is_some_and(|r| r.state == OperationState::OutcomeUnknown)
    {
        let asked = c
            .control_resource(interrupted, OperationControl::AcknowledgeUnknown)
            .map(|_| true)
            .map_err(|e| e.to_string());
        let released = settle(60, || {
            row(&c, interrupted).is_some_and(|r| r.state == OperationState::AcknowledgedUnknown)
        })
        .await;
        steps.push(json!({"step": "acknowledge what stayed unknown", "asked": asked, "acknowledged": released}));
        if !released {
            failures.push("the unknown batch could not be acknowledged".into());
        }
    }

    // The pool is free again: a batch of one snapshot goes through.
    let source = ExportSource::Snapshot {
        snapshot: ids[1],
        parent: vm,
    };
    let pool = c.latest().inventory().unwrap().pool_id;
    let free = c.submit_snapshot_export_batch(
        OperationId::new(),
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap(),
        VmWorkflowBatch {
            items: vec![BatchVmWorkflow::snapshot_export(&source, format.clone()).unwrap()],
        },
        vec![(
            ids[1],
            TransferFile::destination(path("after", &names[1])).unwrap(),
        )],
    );
    match free {
        Ok(operation) => {
            let row = wait_outcome(&operation, 900).await;
            let present = path("after", &names[1]).exists();
            steps.push(json!({"step": "a batch of one afterwards", "state": format!("{:?}", row.state), "detail": row.detail, "archive_present": present}));
            if row.state != OperationState::Succeeded || !present {
                failures.push(format!(
                    "a batch afterwards did not go through: {:?}: {}",
                    row.state, row.detail
                ));
            }
        }
        Err(error) => failures.push(format!("a batch afterwards was not accepted: {error}")),
    }

    // Remove the archives, then the snapshots and the checkpoint with their disks.
    for leg in ["stop", "cut", "after"] {
        for name in &names {
            let _ = std::fs::remove_file(path(leg, name));
        }
    }
    for (index, id) in ids.iter().enumerate() {
        let disks = c
            .latest()
            .inventory()
            .unwrap()
            .details
            .vbds
            .values()
            .filter(|b| b.snapshot == Some(*id) && b.kind == "Disk")
            .filter_map(|b| b.vdi)
            .collect::<Vec<_>>();
        let deletion = c
            .submit(
                OperationId::new(),
                VmCommand::DeleteSnapshot {
                    snapshot: c.latest().observe_snapshot(vm, *id).unwrap(),
                    disks,
                },
            )
            .expect("deletion accepted");
        let row = wait_vm_outcome(&deletion, 300).await;
        let gone = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.snapshots.get(id).is_none())
        })
        .await;
        if row.state != OperationState::Succeeded || !gone {
            failures.push(format!("{} was not deleted: {}", names[index], row.detail));
        }
    }
    c.close().await;
    lab.record(
        "snapshot-batch-stop-and-interruption",
        json!({"steps": steps, "failures": failures}),
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

/// P10-04, P10-09, P09-02, P09-03, P09-05, P09-06, P09-08: rescan each pool host's physical
/// interfaces so its storage NIC appears, give that NIC a static address on the isolated
/// storage network, probe the appliance's NFS exports through the coordinator, create a
/// shared NFS repository on the appliance and confirm both hosts attach it, detach and
/// reattach it on the member, then rescan it. The repository is kept for the shared-storage
/// scenarios that follow; a repository left by an earlier run is reused.
#[tokio::test]
#[ignore]
async fn lab_storage_interfaces_and_shared_nfs_repository() {
    use xenadmin_core::{
        Ipv4Configuration, Ipv4Mode, NetworkOperation, NfsVersion, PifIdentity, ResourceCommand,
        StorageDiscoveryResult, StorageLocation, StorageOperation, StorageQuery,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("the manifest records a storage network (lab/manage.py storage-network)");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let prefix = storage
        .subnet
        .split('/')
        .next()
        .and_then(|a| a.rsplit_once('.'))
        .map(|(p, _)| p.to_string())
        .expect("storage subnet prefix");
    let mut steps = Vec::new();
    let pif_by_mac = |c: &Connection, host: HostId, macs: &[String]| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pifs
                .values()
                .find(|p| {
                    p.host == Some(host)
                        && macs.first().is_some_and(|m| p.mac.eq_ignore_ascii_case(m))
                })
                .cloned()
        })
    };
    for host in [coordinator, member] {
        let host_name = inventory.hosts[&host].name.clone();
        let lab_index = lab
            .hosts
            .iter()
            .position(|h| h.name == host_name)
            .expect("host in the manifest");
        let macs = storage
            .host_nics
            .get(&host_name)
            .cloned()
            .unwrap_or_default();
        assert!(
            !macs.is_empty(),
            "{host_name} has no storage NICs in the manifest"
        );
        let mut entry = resource_op(
            &c,
            &format!("rescan physical interfaces on {host_name}"),
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Host(host))
                    .unwrap(),
                operation: NetworkOperation::ScanInterfaces,
            },
            300,
        )
        .await;
        entry["read_back"] = json!(settle(120, || pif_by_mac(&c, host, &macs).is_some()).await);
        note(&mut steps, entry);
        let Some(pif) = pif_by_mac(&c, host, &macs) else {
            lab.record(
                "storage-interfaces-and-shared-nfs-repository",
                json!({"appliance": storage.nas_ip, "failed": format!("storage interface of {host_name} not listed"), "steps": steps}),
            );
            panic!("storage interface of {host_name} not listed: {steps:?}");
        };
        let address = format!("{prefix}.{}", 11 + lab_index);
        let before = Ipv4Configuration {
            mode: match pif.ip_configuration_mode.as_str() {
                "DHCP" => Ipv4Mode::Dhcp,
                "Static" => Ipv4Mode::Static,
                _ => Ipv4Mode::None,
            },
            address: pif.ip.clone(),
            netmask: pif.netmask.clone(),
            gateway: pif.gateway.clone(),
            dns: pif.dns.clone(),
        };
        note(
            &mut steps,
            json!({"step": "storage interface", "host": host_name, "device": pif.device, "attached": pif.currently_attached, "mode_before": pif.ip_configuration_mode, "address_before": pif.ip}),
        );
        if pif.ip == address && pif.ip_configuration_mode == "Static" {
            note(
                &mut steps,
                json!({"step": format!("configure a static storage address on {host_name}"), "skipped": "already configured"}),
            );
        } else {
            let mut entry = resource_op(
                &c,
                &format!("configure a static storage address on {host_name}"),
                ResourceCommand::Network {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Pif(pif.id))
                        .unwrap(),
                    operation: NetworkOperation::ConfigureIpv4 {
                        expected: PifIdentity::from_inventory(&pif).expect("interface identity"),
                        before,
                        configuration: Ipv4Configuration {
                            mode: Ipv4Mode::Static,
                            address: address.clone(),
                            netmask: "255.255.255.0".into(),
                            gateway: String::new(),
                            dns: String::new(),
                        },
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || pif_by_mac(&c, host, &macs)
                    .is_some_and(|p| p.ip == address
                        && p.ip_configuration_mode == "Static"
                        && p.currently_attached == Some(true)))
                .await
            );
            entry["address"] = json!(address);
            note(&mut steps, entry);
        }
        // Configuring an address does not bring a secondary interface up; attach it.
        let pif = pif_by_mac(&c, host, &macs).expect("storage interface");
        if pif.currently_attached != Some(true) {
            let mut entry = resource_op(
                &c,
                &format!("attach the storage interface on {host_name}"),
                ResourceCommand::Network {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Pif(pif.id))
                        .unwrap(),
                    operation: NetworkOperation::SetAttached {
                        expected: PifIdentity::from_inventory(&pif).expect("interface identity"),
                        expected_attached: false,
                        attached: true,
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || pif_by_mac(&c, host, &macs)
                    .is_some_and(
                        |p| p.currently_attached == Some(true) && p.ip == address
                    ))
                .await
            );
            note(&mut steps, entry);
        }
    }

    // Probe the appliance's exports through the coordinator.
    let probe = c
        .discover_storage(
            c.latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .unwrap(),
            StorageQuery::NfsExports {
                host: coordinator,
                server: storage.nas_ip.clone(),
                version: NfsVersion::Automatic,
            },
        )
        .expect("probe accepted");
    let result = tokio::time::timeout(deadline(180), async {
        loop {
            if let Some(result) = probe.latest() {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("probe answered");
    let exports = match &result {
        Ok(StorageDiscoveryResult::NfsExports(rows)) => Some(
            rows.iter()
                .map(|r| json!({"path": r.path, "access": r.access}))
                .collect::<Vec<_>>(),
        ),
        _ => None,
    };
    note(
        &mut steps,
        json!({"step": "probe NFS exports", "server": storage.nas_ip, "exports": exports, "error": result.as_ref().err(), "read_back": exports.as_ref().is_some_and(|e| e.iter().any(|x| x["path"] == "/srv/nfs/sr"))}),
    );

    // Create (or reuse) the shared repository.
    let find_sr = |c: &Connection, prefix: &str| {
        c.latest().inventory().and_then(|i| {
            i.storage
                .iter()
                .find(|s| s.name.starts_with(prefix))
                .map(|s| (xenadmin_core::SrId::from(s.uuid), s.name.clone()))
        })
    };
    let attachments = |c: &Connection, sr: xenadmin_core::SrId| {
        c.latest()
            .inventory()
            .map(|i| {
                i.details
                    .pbds
                    .values()
                    .filter(|p| p.sr == Some(sr))
                    .map(|p| (p.host, p.currently_attached))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let (sr, sr_name) = if let Some(existing) = find_sr(&c, "lab-nfs-") {
        note(
            &mut steps,
            json!({"step": "create shared NFS repository", "reused": existing.1}),
        );
        existing
    } else {
        let name = format!("lab-nfs-{stamp}");
        let mut entry = resource_op(
            &c,
            "create shared NFS repository",
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                operation: StorageOperation::Create {
                    host: coordinator,
                    name: name.clone(),
                    description: "Shared NFS repository on the lab appliance".into(),
                    location: StorageLocation::Nfs {
                        server: storage.nas_ip.clone(),
                        path: "/srv/nfs/sr".into(),
                        version: NfsVersion::Automatic,
                    },
                    hba_device: None,
                    local_device: None,
                },
            },
            900,
        )
        .await;
        entry["read_back"] = json!(settle(180, || find_sr(&c, &name).is_some()).await);
        note(&mut steps, entry);
        match find_sr(&c, &name) {
            Some(found) => found,
            None => {
                lab.record(
                    "storage-interfaces-and-shared-nfs-repository",
                    json!({"appliance": storage.nas_ip, "failed": "shared repository not listed", "steps": steps}),
                );
                panic!("shared repository not listed: {steps:?}");
            }
        }
    };
    let shared = settle(180, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .storage
                .get(&sr)
                .is_some_and(|d| d.shared == Some(true))
        }) && attachments(&c, sr).len() == 2
            && attachments(&c, sr).iter().all(|(_, a)| *a == Some(true))
    })
    .await;
    note(
        &mut steps,
        json!({"step": "shared repository attached on both hosts", "repository": sr_name, "read_back": shared, "attachments": attachments(&c, sr).iter().map(|(h, a)| json!({"host": h.map(|h| inventory.hosts[&h].name.clone()), "attached": a})).collect::<Vec<_>>()}),
    );

    // Attach, detach and reattach on the member, starting from its actual state.
    let member_attached = |c: &Connection| {
        attachments(c, sr)
            .iter()
            .find(|(h, _)| *h == Some(member))
            .map(|(_, a)| *a)
    };
    let member_pbd = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pbds
                .values()
                .find(|p| p.sr == Some(sr) && p.host == Some(member))
                .map(|p| p.id)
        })
    };
    let mut plan = Vec::new();
    if member_attached(&c) != Some(Some(true)) {
        plan.push(("attach the repository on the member", true));
    }
    plan.push(("detach the repository on the member", false));
    plan.push(("reattach the repository on the member", true));
    for (label, attached) in plan {
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pbd(
                        member_pbd(&c).expect("the member's attachment record"),
                    ))
                    .unwrap(),
                operation: StorageOperation::SetAttached {
                    sr,
                    host: member,
                    expected_attached: !attached,
                    attached,
                },
            },
            600,
        )
        .await;
        entry["read_back"] =
            json!(settle(120, || member_attached(&c) == Some(Some(attached))).await);
        // The client refuses to detach a repository that running guests use anywhere in the
        // pool, even on a host without them; record that guard instead of failing.
        if entry["state"] == "Rejected"
            && entry["detail"]
                .as_str()
                .is_some_and(|d| d.contains("in use"))
        {
            entry["expected_refusal"] = json!(true);
            note(&mut steps, entry);
            break;
        }
        note(&mut steps, entry);
    }
    let entry = resource_op(
        &c,
        "rescan the shared repository",
        ResourceCommand::ScanStorage {
            sr: c
                .latest()
                .observe_resource(ResourceTarget::Storage(sr))
                .unwrap(),
        },
        180,
    )
    .await;
    note(&mut steps, entry);
    c.close().await;
    lab.record(
        "storage-interfaces-and-shared-nfs-repository",
        json!({"appliance": storage.nas_ip, "repository": sr_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert_eq!(step["state"], "Rejected", "{step}");
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P11-09, P08-02, P06-11, P07-14, P11-01: build a tools-equipped guest on the shared NFS
/// repository from the lab's remastered Alpine image: import the isohybrid image as a raw
/// disk on that repository, create a VM from the generic template there, attach the disk as
/// its boot device and start it; wait for the guest agent to report a tools version and an
/// address; connect and disconnect its NIC while it runs (hot plug needs PV drivers); then
/// live-migrate it to the member and back without moving storage. The guest stays running on
/// the coordinator for later scenarios and a guest left by an earlier run is reused.
#[tokio::test]
#[ignore]
async fn lab_tools_guest_on_shared_storage() {
    use xenadmin_core::{
        DeviceOperation, DiskImageFormat, DiskPlacement, GuestFirmware, NicPlacement, PowerAction,
        PowerState, TransferAction, TransferFile, TransferRequest, VmCommand, VmLifecycleOperation,
        VmMigration,
    };
    let lab = Lab::load();
    let image = lab
        .guest_tools_image
        .clone()
        .expect("the manifest records a tools guest image (lab/manage.py guest-image)");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let shared: xenadmin_core::SrId = inventory
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| s.uuid.into())
        .expect("the shared NFS repository from the storage scenario");
    let network = inventory
        .details
        .pifs
        .values()
        .find(|p| p.host == Some(coordinator) && p.management == Some(true))
        .and_then(|p| p.network)
        .expect("management network");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();

    let vm = if let Some(existing) = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
    {
        note(
            &mut steps,
            json!({"step": "reuse the tools guest", "name": existing.name, "power_state": format!("{:?}", existing.power_state)}),
        );
        existing.id
    } else {
        // Import the bootable image as a raw disk on the shared repository.
        let disk_name = format!("lab-tools-image-{stamp}");
        let file = TransferFile::disk_source(&image, DiskImageFormat::Raw).expect("inspect image");
        let started = std::time::Instant::now();
        let import = c
            .transfer(
                OperationId::new(),
                c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                TransferRequest {
                    action: TransferAction::ImportDisk {
                        sr: shared,
                        name: disk_name.clone(),
                        format: DiskImageFormat::Raw,
                    },
                    file,
                },
            )
            .expect("import accepted");
        let outcome = wait_outcome(&import, 900).await;
        let imported = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .vdis
                    .values()
                    .any(|v| v.name == disk_name && v.sr == Some(shared))
            })
        })
        .await;
        note(
            &mut steps,
            json!({"step": "import the tools image onto the shared repository", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "read_back": imported}),
        );
        let vdi = c
            .latest()
            .inventory()
            .unwrap()
            .details
            .vdis
            .values()
            .find(|v| v.name == disk_name && v.sr == Some(shared))
            .map(|v| v.id)
            .expect("imported disk listed");
        let template = inventory
            .vms
            .values()
            .find(|v| v.is_template && v.name == "Other install media")
            .expect("generic template");
        let name = format!("lab-tools-{stamp}");
        let create = c
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: c.latest().observe_vm(template.id).unwrap(),
                    operation: VmLifecycleOperation::Create {
                        name: name.clone(),
                        description: "Lab guest with xe-guest-utilities on the shared repository"
                            .into(),
                        sr: shared,
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
        let outcome = wait_vm_outcome(&create, 300).await;
        let vm = outcome.created_vm.expect("created VM identity");
        let visible = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms.contains_key(&vm))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "create the tools guest", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail, "read_back": visible}),
        );
        let mut entry = device_op(
            &c,
            vm,
            DeviceOperation::AttachDisk {
                vdi,
                device: "0".into(),
                read_only: false,
            },
            300,
        )
        .await;
        entry["step"] = json!("attach the imported disk as the boot device");
        entry["read_back"] = json!(
            settle(60, || c.latest().inventory().is_some_and(|i| i
                .details
                .vm_disks
                .get(&vm)
                .is_some_and(|d| !d.is_empty())))
            .await
        );
        note(&mut steps, entry);
        vm
    };
    if c.latest().inventory().unwrap().vms[&vm].power_state != PowerState::Running {
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the tools guest");
        note(&mut steps, start);
    }
    let agent = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            let g = i.details.vms.get(&vm)?.guest_metrics?;
            let m = i.details.guest_metrics.get(&g)?;
            Some((
                m.tools_version.clone(),
                m.networks.clone(),
                m.os_version.clone(),
            ))
        })
    };
    let reported = settle(240, || {
        agent(&c).is_some_and(|(tools, _, _)| !tools.is_empty())
    })
    .await;
    let addressed = settle(60, || {
        agent(&c).is_some_and(|(_, networks, _)| !networks.is_empty())
    })
    .await;
    let (tools, networks, os) = agent(&c).unwrap_or_default();
    note(
        &mut steps,
        json!({"step": "guest agent reports its tools version", "read_back": reported, "address_reported": addressed, "tools_version": tools, "network_keys": networks.keys().collect::<Vec<_>>(), "os": os.get("name"), "host": c.latest().inventory().unwrap().vms[&vm].host.map(|h| inventory.hosts[&h].name.clone())}),
    );

    // Hot plug with PV drivers present.
    let nic = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vifs
        .values()
        .find(|v| v.vm == Some(vm))
        .cloned()
        .expect("the guest's NIC");
    let vif = nic.id;
    let vif_view = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.vifs.get(&vif).cloned())
    };
    let hotplug = settle(60, || {
        vif_view(&c).is_some_and(|v| {
            v.allowed_operations
                .as_deref()
                .is_some_and(|ops| ops.iter().any(|o| o == "unplug"))
        })
    })
    .await;
    note(
        &mut steps,
        json!({"step": "NIC hot unplug advertised", "read_back": hotplug, "allowed_operations": vif_view(&c).and_then(|v| v.allowed_operations)}),
    );
    if hotplug {
        let mut entry = device_op(&c, vm, DeviceOperation::UnplugNic { vif }, 180).await;
        entry["step"] = json!("disconnect the NIC while running");
        entry["read_back"] = json!(
            settle(60, || vif_view(&c)
                .is_some_and(|v| v.currently_attached == Some(false)))
            .await
        );
        note(&mut steps, entry);
        let mut entry = device_op(&c, vm, DeviceOperation::PlugNic { vif }, 180).await;
        entry["step"] = json!("reconnect the NIC while running");
        entry["read_back"] = json!(
            settle(60, || vif_view(&c)
                .is_some_and(|v| v.currently_attached == Some(true)))
            .await
        );
        note(&mut steps, entry);
    }

    // Live migration on shared storage: disks stay where they are.
    let current = c.latest().inventory().unwrap().clone();
    let source = current.vms[&vm].host.expect("resident host");
    for (label, destination) in [
        (
            "live-migrate to the member on shared storage",
            if source == coordinator {
                member
            } else {
                coordinator
            },
        ),
        ("live-migrate back", source),
    ] {
        let now = c.latest().inventory().unwrap().clone();
        let disks = now.details.vm_disks[&vm]
            .iter()
            .filter_map(|b| now.details.vbds.get(b))
            .filter(|b| b.kind == "Disk")
            .filter_map(|b| {
                let vdi = b.vdi?;
                let sr = now.details.vdis.get(&vdi)?.sr?;
                Some(DiskPlacement {
                    vdi,
                    expected_sr: sr,
                    destination_sr: sr,
                })
            })
            .collect::<Vec<_>>();
        let networks = now
            .details
            .vifs
            .values()
            .filter(|v| v.vm == Some(vm))
            .filter_map(|v| {
                let network = v.network?;
                Some(NicPlacement {
                    vif: v.id,
                    expected_network: network,
                    destination_network: network,
                })
            })
            .collect::<Vec<_>>();
        let advertised = settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.details.vms.get(&vm).is_some_and(|d| {
                    d.allowed_operations
                        .as_ref()
                        .is_some_and(|ops| ops.iter().any(|op| op == "pool_migrate"))
                })
            })
        })
        .await;
        let started = std::time::Instant::now();
        let migration = c
            .submit(
                OperationId::new(),
                VmCommand::Migrate {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    operation: VmMigration {
                        destination,
                        expected_host: now.vms[&vm].host,
                        expected_affinity: None,
                        live: true,
                        disks,
                        networks,
                        transfer_network: None,
                    },
                },
            )
            .expect("migration accepted");
        let (row, progress) = wait_vm_outcome_sampled(&migration, 900).await;
        let placed = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms[&vm].host == Some(destination)
                    && i.vms[&vm].power_state == PowerState::Running
            })
        })
        .await;
        note(
            &mut steps,
            json!({"step": label, "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": placed, "pool_migrate_advertised": advertised, "elapsed_ms": started.elapsed().as_millis() as u64, "progress_samples": progress, "destination": inventory.hosts[&destination].name}),
        );
    }
    c.close().await;
    lab.record(
        "tools-guest-on-shared-storage",
        json!({"vm": vm.to_string(), "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P13-05, P06-07: record that enabling HA is refused while a storage interface the
/// heartbeat repository is reached over is not protected from unplug (finding 36), protect
/// those interfaces, enable HA on the pool with the shared NFS repository as its heartbeat
/// store, protect the tools guest with the restart priority, raise the tolerated host
/// failures to one and read every value back, then restore the priority and tolerance and
/// disable HA. HA left enabled by an interrupted run is disabled first; the scenario always
/// records before asserting so an unexpected end state leaves evidence.
#[tokio::test]
#[ignore]
async fn lab_ha_enable_with_shared_heartbeat_and_protected_guest() {
    use xenadmin_core::{
        HaRestartPriority, NetworkOperation, PifIdentity, PoolOperation, ResourceCommand,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let shared: xenadmin_core::SrId = inventory
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| s.uuid.into())
        .expect("the shared NFS repository from the storage scenario");
    let guest = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned();
    let mut steps = Vec::new();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let administration = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pool.as_ref().map(|p| p.administration.clone()))
    };
    let a = administration(&c).expect("pool administration");
    note(
        &mut steps,
        json!({"step": "HA before", "enabled": a.ha_enabled, "failures_to_tolerate": a.ha_failures_to_tolerate, "heartbeat_srs": a.heartbeat_srs.len(), "plan_exists_for": a.ha_plan_exists_for}),
    );
    if a.ha_enabled == Some(true) {
        let mut entry = resource_op(
            &c,
            "disable HA left by an earlier run",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::DisableHa,
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(180, || administration(&c)
                .is_some_and(|a| a.ha_enabled == Some(false)))
            .await
        );
        note(&mut steps, entry);
    }
    let failures_before = administration(&c)
        .and_then(|a| a.ha_failures_to_tolerate)
        .unwrap_or(0);

    // Finding 36: the storage interfaces the heartbeat repository is reached over must be
    // kept attached at boot. While one is not, the client refuses to enable HA; that
    // refusal is recorded first, then each is protected for the length of this scenario.
    let shared_server: std::net::Ipv4Addr = lab
        .storage
        .as_ref()
        .and_then(|s| s.nas_ip.parse().ok())
        .expect("appliance storage address");
    let heartbeat_pifs = inventory
        .details
        .pifs
        .values()
        .filter(|p| {
            let (Ok(ip), Ok(mask)) = (
                p.ip.parse::<std::net::Ipv4Addr>(),
                p.netmask.parse::<std::net::Ipv4Addr>(),
            ) else {
                return false;
            };
            p.management != Some(true)
                && (u32::from(ip) & u32::from(mask)) == (u32::from(shared_server) & u32::from(mask))
        })
        .cloned()
        .collect::<Vec<_>>();
    let unplug_protected = |c: &Connection, pif: xenadmin_core::PifId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pifs.get(&pif).and_then(|p| p.disallow_unplug))
    };
    let unprotected = heartbeat_pifs
        .iter()
        .filter(|p| p.disallow_unplug != Some(true))
        .cloned()
        .collect::<Vec<_>>();
    let mut refusal = None;
    if !unprotected.is_empty() {
        let entry = resource_op(
            &c,
            "enable HA while a heartbeat interface is not protected from unplug",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::EnableHa {
                    heartbeat_srs: vec![shared],
                    expected_failures: failures_before,
                },
            },
            300,
        )
        .await;
        // Kept apart from the steps that must succeed: this one must be refused.
        refusal = Some(
            json!({"step": entry["step"], "refused": entry["state"] == "Rejected", "detail": entry["detail"],
                              "ha_enabled_after": administration(&c).and_then(|a| a.ha_enabled)}),
        );
    }
    let mut protected = Vec::new();
    for pif in &unprotected {
        let host_name = pif
            .host
            .and_then(|h| inventory.hosts.get(&h))
            .map(|h| h.name.clone())
            .unwrap_or_default();
        let mut entry = resource_op(
            &c,
            &format!(
                "protect the heartbeat interface {} on {host_name} from unplug",
                pif.device
            ),
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(pif.id))
                    .unwrap(),
                operation: NetworkOperation::SetUnplugProtection {
                    expected: PifIdentity::from_inventory(pif).expect("interface identity"),
                    before: false,
                    enabled: true,
                },
            },
            300,
        )
        .await;
        entry["read_back"] =
            json!(settle(120, || unplug_protected(&c, pif.id) == Some(true)).await);
        note(&mut steps, entry);
        protected.push(pif.id);
    }

    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "enable HA with the shared repository as heartbeat store",
        ResourceCommand::Pool {
            target: pool_target(&c),
            operation: PoolOperation::EnableHa {
                heartbeat_srs: vec![shared],
                expected_failures: failures_before,
            },
        },
        900,
    )
    .await;
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    entry["read_back"] = json!(
        settle(300, || administration(&c).is_some_and(|a| a.ha_enabled
            == Some(true)
            && a.heartbeat_srs.contains(&shared)))
        .await
    );
    note(&mut steps, entry);
    let enabled = administration(&c).is_some_and(|a| a.ha_enabled == Some(true));

    if enabled {
        if let Some(guest) = &guest {
            let priority_of = |c: &Connection| {
                c.latest().inventory().and_then(|i| {
                    i.details
                        .vms
                        .get(&guest.id)
                        .and_then(|d| d.ha_restart_priority.clone())
                })
            };
            let from = priority_of(&c)
                .as_deref()
                .and_then(HaRestartPriority::parse)
                .unwrap_or(HaRestartPriority::Disabled);
            // The guest may carry the restart priority already, from an earlier scenario.
            // An edit that changes nothing is refused, so the round trip starts from what
            // the guest has: to the other value and back, with the tolerance checked
            // while the guest is protected.
            let protect = "protect the tools guest with the restart priority";
            let unprotect = "remove the tools guest's restart priority";
            let round_trip = if from == HaRestartPriority::Restart {
                [
                    (unprotect, from, HaRestartPriority::Disabled),
                    (protect, HaRestartPriority::Disabled, from),
                ]
            } else {
                [
                    (protect, from, HaRestartPriority::Restart),
                    (unprotect, HaRestartPriority::Restart, from),
                ]
            };
            // The tolerance likewise moves to a value it does not have, and back.
            let tolerated = if failures_before == 1 { 0 } else { 1 };
            for (label, expected, priority) in round_trip {
                let wire = if priority == HaRestartPriority::Restart {
                    "restart"
                } else {
                    ""
                };
                let mut entry = resource_op(
                    &c,
                    label,
                    ResourceCommand::Pool {
                        target: pool_target(&c),
                        operation: PoolOperation::SetVmRestartPriority {
                            vm: guest.id,
                            expected,
                            priority,
                        },
                    },
                    300,
                )
                .await;
                entry["read_back"] = json!(
                    settle(120, || priority_of(&c)
                        .as_deref()
                        .map(|p| if p == "restart" { "restart" } else { "" })
                        == Some(wire))
                    .await
                );
                note(&mut steps, entry);
                if label.starts_with("protect") {
                    // Change the tolerated host failures while the guest is protected.
                    let mut entry = resource_op(
                        &c,
                        &format!("tolerate {tolerated} host failures"),
                        ResourceCommand::Pool {
                            target: pool_target(&c),
                            operation: PoolOperation::SetHaFailuresToTolerate {
                                expected: failures_before,
                                failures: tolerated,
                            },
                        },
                        300,
                    )
                    .await;
                    entry["read_back"] = json!(
                        settle(120, || administration(&c)
                            .is_some_and(|a| a.ha_failures_to_tolerate == Some(tolerated)))
                        .await
                    );
                    entry["plan_exists_for"] =
                        json!(administration(&c).and_then(|a| a.ha_plan_exists_for));
                    note(&mut steps, entry);
                    let mut entry = resource_op(
                        &c,
                        "restore the tolerated failures",
                        ResourceCommand::Pool {
                            target: pool_target(&c),
                            operation: PoolOperation::SetHaFailuresToTolerate {
                                expected: tolerated,
                                failures: failures_before,
                            },
                        },
                        300,
                    )
                    .await;
                    entry["read_back"] = json!(
                        settle(120, || administration(&c).is_some_and(|a| a
                            .ha_failures_to_tolerate
                            == Some(failures_before)))
                        .await
                    );
                    note(&mut steps, entry);
                }
            }
        } else {
            steps
                .push(json!({"step": "restart priority", "skipped": "no tools guest on the pool"}));
        }
        let started = std::time::Instant::now();
        let mut entry = resource_op(
            &c,
            "disable HA",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::DisableHa,
            },
            900,
        )
        .await;
        entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
        entry["read_back"] = json!(
            settle(300, || administration(&c)
                .is_some_and(|a| a.ha_enabled == Some(false)))
            .await
        );
        note(&mut steps, entry);
    }
    // The protection this scenario set is taken off again, as the takeover scenario does.
    for pif in protected {
        let Some(current) = c
            .latest()
            .inventory()
            .and_then(|i| i.details.pifs.get(&pif).cloned())
        else {
            continue;
        };
        let mut entry = resource_op(
            &c,
            &format!("remove the unplug protection of {}", current.device),
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(pif))
                    .unwrap(),
                operation: NetworkOperation::SetUnplugProtection {
                    expected: PifIdentity::from_inventory(&current).expect("interface identity"),
                    before: true,
                    enabled: false,
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(settle(120, || unplug_protected(&c, pif) == Some(false)).await);
        note(&mut steps, entry);
    }
    c.close().await;
    lab.record(
        "ha-enable-with-shared-heartbeat-and-protected-guest",
        json!({"heartbeat_repository": shared.to_string(), "guest": guest.as_ref().map(|g| g.name.clone()),
               "refusal_while_unprotected": refusal, "steps": steps}),
    );
    if let Some(refusal) = &refusal {
        assert_eq!(refusal["refused"], true, "{refusal}");
        assert_eq!(refusal["ha_enabled_after"], false, "{refusal}");
    }
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P12-04, P12-03, P11-01: with the tools guest running on shared storage, move it to the
/// member by a live migration without storage, enter maintenance on the member with the guest
/// reviewed as its resident so the client evacuates it, verify the guest running on the
/// coordinator with the member disabled, then exit maintenance.
#[tokio::test]
#[ignore]
async fn lab_member_evacuation_of_the_tools_guest() {
    use xenadmin_core::{
        DiskPlacement, HostOperation, NicPlacement, PowerAction, PowerState, ResourceCommand,
        VmCommand, VmMigration,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let guest = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned()
        .expect("the tools guest from the shared-storage scenario");
    let vm = guest.id;
    let mut steps = Vec::new();
    if guest.power_state != PowerState::Running {
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the tools guest");
        note(&mut steps, start);
    }
    let resident = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.vms.get(&vm).map(|v| (v.power_state.clone(), v.host)))
    };
    if resident(&c).is_some_and(|(_, h)| h != Some(member)) {
        let now = c.latest().inventory().unwrap().clone();
        let disks = now.details.vm_disks[&vm]
            .iter()
            .filter_map(|b| now.details.vbds.get(b))
            .filter(|b| b.kind == "Disk")
            .filter_map(|b| {
                let vdi = b.vdi?;
                let sr = now.details.vdis.get(&vdi)?.sr?;
                Some(DiskPlacement {
                    vdi,
                    expected_sr: sr,
                    destination_sr: sr,
                })
            })
            .collect::<Vec<_>>();
        let networks = now
            .details
            .vifs
            .values()
            .filter(|v| v.vm == Some(vm))
            .filter_map(|v| {
                let network = v.network?;
                Some(NicPlacement {
                    vif: v.id,
                    expected_network: network,
                    destination_network: network,
                })
            })
            .collect::<Vec<_>>();
        settle(60, || {
            c.latest().inventory().is_some_and(|i| {
                i.details.vms.get(&vm).is_some_and(|d| {
                    d.allowed_operations
                        .as_ref()
                        .is_some_and(|ops| ops.iter().any(|op| op == "pool_migrate"))
                })
            })
        })
        .await;
        let migration = c
            .submit(
                OperationId::new(),
                VmCommand::Migrate {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    operation: VmMigration {
                        destination: member,
                        expected_host: now.vms[&vm].host,
                        expected_affinity: None,
                        live: true,
                        disks,
                        networks,
                        transfer_network: None,
                    },
                },
            )
            .expect("migration accepted");
        let row = wait_vm_outcome(&migration, 900).await;
        let placed = settle(120, || {
            resident(&c).is_some_and(|(s, h)| s == PowerState::Running && h == Some(member))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "live-migrate the tools guest to the member", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": placed}),
        );
    }
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "enter maintenance evacuating the tools guest",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: true,
                resident_vms: vec![vm],
            },
        },
        900,
    )
    .await;
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    entry["read_back"] = json!(
        settle(180, || {
            resident(&c).is_some_and(|(s, h)| s == PowerState::Running && h == Some(coordinator))
                && c.latest()
                    .inventory()
                    .is_some_and(|i| !i.hosts[&member].enabled)
        })
        .await
    );
    entry["vm_state"] = json!(resident(&c).map(|(s, h)| json!({"power_state": format!("{s:?}"), "host": h.map(|h| inventory.hosts[&h].name.clone())})));
    note(&mut steps, entry);
    let entered = c
        .latest()
        .inventory()
        .is_some_and(|i| !i.hosts[&member].enabled);
    if entered {
        let mut entry = resource_op(
            &c,
            "exit maintenance",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || c
                .latest()
                .inventory()
                .is_some_and(|i| i.hosts[&member].enabled))
            .await
        );
        note(&mut steps, entry);
    }
    c.close().await;
    lab.record(
        "member-evacuation-of-the-tools-guest",
        json!({"guest": guest.name, "member": inventory.hosts[&member].name, "coordinator": inventory.hosts[&coordinator].name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P02-08: list every physical interface of the pool with its host, device, MAC, network,
/// address configuration and attachment state, and the networks it serves. Read-only; used
/// to inspect the lab's interfaces after hot-plugging NICs.
#[tokio::test]
#[ignore]
async fn lab_inventory_interfaces() {
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, true).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let mut pifs = inventory
        .details
        .pifs
        .values()
        .map(|p| {
            json!({
                "host": p.host.map(|h| inventory.hosts[&h].name.clone()),
                "device": p.device, "mac": p.mac, "physical": p.physical, "vlan": p.vlan,
                "network": p.network.and_then(|n| inventory.networks.iter().find(|x| xenadmin_core::NetworkId::from(x.uuid) == n).map(|x| x.name.clone())),
                "management": p.management, "attached": p.currently_attached,
                "mode": p.ip_configuration_mode, "ip": p.ip, "netmask": p.netmask, "gateway": p.gateway,
                "capabilities": p.capabilities,
            })
        })
        .collect::<Vec<_>>();
    pifs.sort_by_key(|p| (p["host"].to_string(), p["device"].to_string()));
    for pif in &pifs {
        eprintln!("pif: {pif}");
    }
    c.close().await;
    lab.record("inventory-interfaces", json!({"pifs": pifs}));
}

/// P12-03, P12-05, P10-04, P10-09, P09-06: put the empty member into maintenance, reboot it
/// through the reviewed reboot so the NIC defined for its next start appears and the
/// hot-plugged NIC gets its persistent name, exit maintenance, rescan its interfaces and
/// confirm both storage NICs by MAC, configure and attach the first with its static storage
/// address, attach the shared repository on the member, and leave the second NIC
/// unconfigured for the bond scenario.
#[tokio::test]
#[ignore]
async fn lab_member_reboot_adds_the_second_storage_nic() {
    use xenadmin_core::{
        HostOperation, Ipv4Configuration, Ipv4Mode, NetworkOperation, PifIdentity, ResourceCommand,
        StorageOperation,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("the manifest records a storage network");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator)
        .expect("member");
    let member_name = inventory.hosts[&member].name.clone();
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must not host guests"
    );
    let lab_index = lab
        .hosts
        .iter()
        .position(|h| h.name == member_name)
        .expect("member in the manifest");
    let macs = storage
        .host_nics
        .get(&member_name)
        .cloned()
        .unwrap_or_default();
    assert!(macs.len() >= 2, "two storage NICs recorded for the member");
    let prefix = storage
        .subnet
        .split('/')
        .next()
        .and_then(|a| a.rsplit_once('.'))
        .map(|(p, _)| p.to_string())
        .expect("storage subnet prefix");
    let address = format!("{prefix}.{}", 11 + lab_index);
    let mut steps = Vec::new();
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let details = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&member).cloned())
    };
    let pif_by_mac = |c: &Connection, mac: &str| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pifs
                .values()
                .find(|p| p.host == Some(member) && p.mac.eq_ignore_ascii_case(mac))
                .cloned()
        })
    };
    note(
        &mut steps,
        json!({"step": "interfaces before", "pifs": macs.iter().map(|m| pif_by_mac(&c, m).map(|p| json!({"device": p.device, "mode": p.ip_configuration_mode, "ip": p.ip, "attached": p.currently_attached}))).collect::<Vec<_>>()}),
    );

    // Maintenance, reboot, exit (finding 12: clear a leftover marker first).
    if details(&c).and_then(|h| h.maintenance_mode) == Some(true) {
        let entry = resource_op(
            &c,
            "clear the leftover maintenance marker",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        note(&mut steps, entry);
    }
    let enabled = c.latest().inventory().unwrap().hosts[&member].enabled;
    let mut entry = resource_op(
        &c,
        "enter maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: enabled,
                resident_vms: Vec::new(),
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || details(&c).and_then(|h| h.maintenance_mode)
            == Some(true))
        .await
    );
    note(&mut steps, entry);
    let boot_before = details(&c).and_then(|h| h.boot_time).expect("boot time");
    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "reboot the member",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::Reboot {
                expected_boot_time: boot_before.clone(),
            },
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
    entry["read_back"] = json!(rebooted);
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    note(&mut steps, entry);
    let live = settle(300, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .hosts
                .get(&member)
                .and_then(|h| h.metrics)
                .and_then(|m| i.details.host_metrics.get(&m))
                .and_then(|m| m.live)
                == Some(true)
        })
    })
    .await;
    note(
        &mut steps,
        json!({"step": "member live again", "read_back": live}),
    );
    let mut entry = resource_op(
        &c,
        "exit maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::ExitMaintenance,
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.hosts[&member].enabled))
        .await
    );
    note(&mut steps, entry);

    // Both storage NICs after the restart.
    let mut entry = resource_op(
        &c,
        "rescan physical interfaces",
        ResourceCommand::Network {
            target: host_target(&c),
            operation: NetworkOperation::ScanInterfaces,
        },
        300,
    )
    .await;
    entry["read_back"] =
        json!(settle(120, || macs.iter().all(|m| pif_by_mac(&c, m).is_some())).await);
    note(&mut steps, entry);
    note(
        &mut steps,
        json!({"step": "interfaces after", "pifs": macs.iter().map(|m| pif_by_mac(&c, m).map(|p| json!({"device": p.device, "mode": p.ip_configuration_mode, "ip": p.ip, "attached": p.currently_attached}))).collect::<Vec<_>>()}),
    );
    let first = pif_by_mac(&c, &macs[0]).expect("first storage NIC");
    if !(first.ip_configuration_mode == "Static" && first.ip == address) {
        let before = Ipv4Configuration {
            mode: match first.ip_configuration_mode.as_str() {
                "DHCP" => Ipv4Mode::Dhcp,
                "Static" => Ipv4Mode::Static,
                _ => Ipv4Mode::None,
            },
            address: first.ip.clone(),
            netmask: first.netmask.clone(),
            gateway: first.gateway.clone(),
            dns: first.dns.clone(),
        };
        let mut entry = resource_op(
            &c,
            "configure the static storage address",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(first.id))
                    .unwrap(),
                operation: NetworkOperation::ConfigureIpv4 {
                    expected: PifIdentity::from_inventory(&first).expect("interface identity"),
                    before,
                    configuration: Ipv4Configuration {
                        mode: Ipv4Mode::Static,
                        address: address.clone(),
                        netmask: "255.255.255.0".into(),
                        gateway: String::new(),
                        dns: String::new(),
                    },
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || pif_by_mac(&c, &macs[0]).is_some_and(|p| p.ip
                == address
                && p.ip_configuration_mode == "Static"))
            .await
        );
        note(&mut steps, entry);
    }
    let first = pif_by_mac(&c, &macs[0]).expect("first storage NIC");
    if first.currently_attached != Some(true) {
        let mut entry = resource_op(
            &c,
            "attach the storage interface",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(first.id))
                    .unwrap(),
                operation: NetworkOperation::SetAttached {
                    expected: PifIdentity::from_inventory(&first).expect("interface identity"),
                    expected_attached: false,
                    attached: true,
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || pif_by_mac(&c, &macs[0])
                .is_some_and(|p| p.currently_attached == Some(true)))
            .await
        );
        note(&mut steps, entry);
    }

    // The shared repository on the member.
    if let Some(sr) = c
        .latest()
        .inventory()
        .unwrap()
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| xenadmin_core::SrId::from(s.uuid))
    {
        let member_attached = |c: &Connection| {
            c.latest().inventory().and_then(|i| {
                i.details
                    .pbds
                    .values()
                    .find(|p| p.sr == Some(sr) && p.host == Some(member))
                    .map(|p| p.currently_attached)
            })
        };
        let member_pbd = |c: &Connection| {
            c.latest().inventory().and_then(|i| {
                i.details
                    .pbds
                    .values()
                    .find(|p| p.sr == Some(sr) && p.host == Some(member))
                    .map(|p| p.id)
            })
        };
        if member_attached(&c) != Some(Some(true)) {
            let mut entry = resource_op(
                &c,
                "attach the shared repository on the member",
                ResourceCommand::Storage {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Pbd(
                            member_pbd(&c).expect("the member's attachment record"),
                        ))
                        .unwrap(),
                    operation: StorageOperation::SetAttached {
                        sr,
                        host: member,
                        expected_attached: false,
                        attached: true,
                    },
                },
                600,
            )
            .await;
            entry["read_back"] =
                json!(settle(120, || member_attached(&c) == Some(Some(true))).await);
            note(&mut steps, entry);
        } else {
            note(
                &mut steps,
                json!({"step": "shared repository on the member", "already_attached": true}),
            );
        }
    }
    c.close().await;
    lab.record(
        "member-reboot-adds-the-second-storage-nic",
        json!({"member": member_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P10-05, P10-09, P09-06: on the member, free its configured storage interface, bond both
/// storage NICs into a new network (active-backup, then balance-slb and back), give the bond
/// the static storage address and attach it, re-attach the shared repository over the bond,
/// then undo everything: detach, clear the bond's address, destroy the bond, delete the bond
/// network, restore the first NIC's address and re-attach the repository.
#[tokio::test]
#[ignore]
async fn lab_member_storage_bond_round_trip() {
    use xenadmin_core::{
        BondHash, BondMode, Ipv4Configuration, Ipv4Mode, NetworkOperation, PifIdentity,
        ResourceCommand, StorageOperation,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("the manifest records a storage network");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let member_name = inventory.hosts[&member].name.clone();
    let lab_index = lab
        .hosts
        .iter()
        .position(|h| h.name == member_name)
        .expect("member in the manifest");
    let macs = storage
        .host_nics
        .get(&member_name)
        .cloned()
        .unwrap_or_default();
    assert!(macs.len() >= 2, "two storage NICs recorded for the member");
    let prefix = storage
        .subnet
        .split('/')
        .next()
        .and_then(|a| a.rsplit_once('.'))
        .map(|(p, _)| p.to_string())
        .expect("storage subnet prefix");
    let address = format!("{prefix}.{}", 11 + lab_index);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let pif_by_mac = |c: &Connection, mac: &str| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pifs
                .values()
                .find(|p| p.host == Some(member) && p.mac.eq_ignore_ascii_case(mac))
                .cloned()
        })
    };
    let pif_by_id = |c: &Connection, id: xenadmin_core::PifId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pifs.get(&id).cloned())
    };
    let shared = inventory
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| xenadmin_core::SrId::from(s.uuid));
    let member_pbd = |c: &Connection| {
        shared.and_then(|sr| {
            c.latest().inventory().and_then(|i| {
                i.details
                    .pbds
                    .values()
                    .find(|p| p.sr == Some(sr) && p.host == Some(member))
                    .map(|p| (p.id, p.currently_attached))
            })
        })
    };
    let pif_target = |c: &Connection, id: xenadmin_core::PifId| {
        c.latest()
            .observe_resource(ResourceTarget::Pif(id))
            .unwrap()
    };
    let ipv4 = |pif: &xenadmin_core::PhysicalInterface| Ipv4Configuration {
        mode: match pif.ip_configuration_mode.as_str() {
            "DHCP" => Ipv4Mode::Dhcp,
            "Static" => Ipv4Mode::Static,
            _ => Ipv4Mode::None,
        },
        address: pif.ip.clone(),
        netmask: pif.netmask.clone(),
        gateway: pif.gateway.clone(),
        dns: pif.dns.clone(),
    };
    let none = Ipv4Configuration {
        mode: Ipv4Mode::None,
        address: String::new(),
        netmask: String::new(),
        gateway: String::new(),
        dns: String::new(),
    };
    let static_address = Ipv4Configuration {
        mode: Ipv4Mode::Static,
        address: address.clone(),
        netmask: "255.255.255.0".into(),
        gateway: String::new(),
        dns: String::new(),
    };

    // Helpers as nested async blocks are awkward; repeat the small patterns inline.
    macro_rules! set_attached {
        ($label:expr, $pif:expr, $attached:expr) => {{
            let pif = $pif;
            let mut entry = resource_op(
                &c,
                $label,
                ResourceCommand::Network {
                    target: pif_target(&c, pif.id),
                    operation: NetworkOperation::SetAttached {
                        expected: PifIdentity::from_inventory(&pif).expect("interface identity"),
                        expected_attached: !$attached,
                        attached: $attached,
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || pif_by_id(&c, pif.id)
                    .is_some_and(|p| p.currently_attached == Some($attached)))
                .await
            );
            note(&mut steps, entry);
        }};
    }
    macro_rules! configure {
        ($label:expr, $pif:expr, $configuration:expr) => {{
            let pif = $pif;
            let configuration: Ipv4Configuration = $configuration;
            let wanted = (configuration.mode.clone(), configuration.address.clone());
            let mut entry = resource_op(
                &c,
                $label,
                ResourceCommand::Network {
                    target: pif_target(&c, pif.id),
                    operation: NetworkOperation::ConfigureIpv4 {
                        expected: PifIdentity::from_inventory(&pif).expect("interface identity"),
                        before: ipv4(&pif),
                        configuration,
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || pif_by_id(&c, pif.id).is_some_and(|p| {
                    p.ip == wanted.1
                        && matches!(
                            (wanted.0.clone(), p.ip_configuration_mode.as_str()),
                            (Ipv4Mode::None, "None")
                                | (Ipv4Mode::Static, "Static")
                                | (Ipv4Mode::Dhcp, "DHCP")
                        )
                }))
                .await
            );
            note(&mut steps, entry);
        }};
    }
    macro_rules! repository_attached {
        ($label:expr, $attached:expr) => {{
            if let (Some(sr), Some((pbd, current))) = (shared, member_pbd(&c)) {
                if current == Some($attached) {
                    note(&mut steps, json!({"step": $label, "skipped": "already in that state"}));
                } else {
                    let mut entry = resource_op(
                        &c,
                        $label,
                        ResourceCommand::Storage {
                            target: c
                                .latest()
                                .observe_resource(ResourceTarget::Pbd(pbd))
                                .unwrap(),
                            operation: StorageOperation::SetAttached {
                                sr,
                                host: member,
                                expected_attached: !$attached,
                                attached: $attached,
                            },
                        },
                        600,
                    )
                    .await;
                    entry["read_back"] = json!(
                        settle(180, || member_pbd(&c).is_some_and(|(_, a)| a == Some($attached)))
                            .await
                    );
                    if entry["state"] == "Rejected"
                        && entry["detail"]
                            .as_str()
                            .is_some_and(|d| d.contains("in use"))
                    {
                        entry["expected_refusal"] = json!(true);
                    }
                    note(&mut steps, entry);
                }
            }
        }};
    }

    let first = pif_by_mac(&c, &macs[0]).expect("first storage NIC");
    let second = pif_by_mac(&c, &macs[1]).expect("second storage NIC");
    note(
        &mut steps,
        json!({"step": "storage NICs", "first": {"device": first.device, "mode": first.ip_configuration_mode, "attached": first.currently_attached}, "second": {"device": second.device, "mode": second.ip_configuration_mode, "attached": second.currently_attached}}),
    );

    // Free both NICs: a bond member must carry no address and be detached.
    repository_attached!("detach the shared repository on the member", false);
    for (index, mac) in macs.iter().take(2).enumerate() {
        let nic = pif_by_mac(&c, mac).expect("storage NIC");
        if nic.currently_attached == Some(true) {
            set_attached!(
                &format!("detach storage NIC {}", index + 1),
                nic.clone(),
                false
            );
        }
        let nic = pif_by_mac(&c, mac).expect("storage NIC");
        if nic.ip_configuration_mode != "None" {
            configure!(
                &format!("clear storage NIC {}'s address", index + 1),
                nic,
                none.clone()
            );
        }
    }

    // Bond network and bond.
    let bond_network_name = format!("lab-bond-{stamp}");
    let mut entry = resource_op(
        &c,
        "create the bond network",
        ResourceCommand::CreateInternalNetwork {
            pool: c
                .latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .unwrap(),
            name: bond_network_name.clone(),
            description: "Network carried by the member's storage bond".into(),
            mtu: 1500,
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| i
            .networks
            .iter()
            .any(|n| n.name == bond_network_name)))
        .await
    );
    note(&mut steps, entry);
    let bond_network: xenadmin_core::NetworkId = c
        .latest()
        .inventory()
        .unwrap()
        .networks
        .iter()
        .find(|n| n.name == bond_network_name)
        .expect("bond network")
        .uuid
        .into();
    let (first, second) = (
        pif_by_mac(&c, &macs[0]).unwrap(),
        pif_by_mac(&c, &macs[1]).unwrap(),
    );
    let bond_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .bonds
                .values()
                .find(|b| b.slaves.contains(&first.id) && b.slaves.contains(&second.id))
                .cloned()
        })
    };
    let mut entry = resource_op(
        &c,
        "create an active-backup bond of both storage NICs",
        ResourceCommand::Network {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Network(bond_network))
                .unwrap(),
            operation: NetworkOperation::CreateBond {
                members: vec![
                    (
                        first.id,
                        PifIdentity::from_inventory(&first).expect("identity"),
                    ),
                    (
                        second.id,
                        PifIdentity::from_inventory(&second).expect("identity"),
                    ),
                ],
                mode: BondMode::ActiveBackup,
                hash: BondHash::SourceMac,
            },
        },
        600,
    )
    .await;
    entry["read_back"] = json!(settle(120, || bond_of(&c).is_some()).await);
    note(&mut steps, entry);
    let Some(bond) = bond_of(&c) else {
        lab.record(
            "member-storage-bond-round-trip",
            json!({"member": member_name, "failed": "bond not listed", "steps": steps}),
        );
        panic!("bond not listed: {steps:?}");
    };
    note(
        &mut steps,
        json!({"step": "bond", "mode": bond.mode, "links_up": bond.links_up, "master": bond.master.map(|m| pif_by_id(&c, m).map(|p| p.device))}),
    );
    for (label, expected, mode, wire) in [
        (
            "switch the bond to balance-slb",
            "active-backup",
            BondMode::BalanceSlb,
            "balance-slb",
        ),
        (
            "switch the bond back to active-backup",
            "balance-slb",
            BondMode::ActiveBackup,
            "active-backup",
        ),
    ] {
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Bond(bond.id))
                    .unwrap(),
                operation: NetworkOperation::SetBondMode {
                    expected_mode: expected.into(),
                    mode,
                },
            },
            300,
        )
        .await;
        entry["read_back"] =
            json!(settle(120, || bond_of(&c).is_some_and(|b| b.mode == wire)).await);
        note(&mut steps, entry);
    }

    // The bond carries the storage address and the repository.
    let master = bond
        .master
        .and_then(|m| pif_by_id(&c, m))
        .expect("bond master interface");
    configure!(
        "give the bond the static storage address",
        master.clone(),
        static_address.clone()
    );
    let master = pif_by_id(&c, master.id).unwrap();
    if master.currently_attached != Some(true) {
        set_attached!("attach the bond", master.clone(), true);
    }
    repository_attached!("re-attach the shared repository over the bond", true);

    // Undo.
    repository_attached!(
        "detach the shared repository before dissolving the bond",
        false
    );
    let master = pif_by_id(&c, master.id).unwrap();
    if master.currently_attached == Some(true) {
        set_attached!("detach the bond", master.clone(), false);
    }
    configure!(
        "clear the bond's address",
        pif_by_id(&c, master.id).unwrap(),
        none.clone()
    );
    let mut entry = resource_op(
        &c,
        "destroy the bond",
        ResourceCommand::Network {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Bond(bond.id))
                .unwrap(),
            operation: NetworkOperation::DeleteBond {
                master: master.id,
                members: vec![first.id, second.id],
                primary: bond.primary_slave.unwrap_or(first.id),
            },
        },
        600,
    )
    .await;
    entry["read_back"] = json!(settle(120, || bond_of(&c).is_none()).await);
    note(&mut steps, entry);
    let mut entry = resource_op(
        &c,
        "delete the bond network",
        ResourceCommand::DeleteInternalNetwork {
            network: c
                .latest()
                .observe_resource(ResourceTarget::Network(bond_network))
                .unwrap(),
            expected_name: bond_network_name.clone(),
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || c.latest().inventory().is_some_and(|i| !i
            .networks
            .iter()
            .any(|n| n.name == bond_network_name)))
        .await
    );
    note(&mut steps, entry);
    let first = pif_by_mac(&c, &macs[0]).expect("first storage NIC after the bond");
    configure!(
        "restore the first storage NIC's address",
        first.clone(),
        static_address.clone()
    );
    let first = pif_by_mac(&c, &macs[0]).unwrap();
    if first.currently_attached != Some(true) {
        set_attached!("re-attach the first storage NIC", first.clone(), true);
    }
    repository_attached!("re-attach the shared repository on the member", true);
    c.close().await;
    lab.record(
        "member-storage-bond-round-trip",
        json!({"member": member_name, "bond_network": bond_network_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            assert_eq!(step["state"], "Rejected", "{step}");
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P12-03, P12-05: put the empty member into maintenance and shut it down through the
/// reviewed shutdown, confirming the host goes offline. The operator then powers the lab
/// domain back on (a device defined for the next start needs a real power cycle, not an OS
/// reboot); `lab_member_return_and_second_storage_nic` records the return.
#[tokio::test]
#[ignore]
async fn lab_member_shutdown_through_maintenance() {
    use xenadmin_core::{HostOperation, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator)
        .expect("member");
    let member_name = inventory.hosts[&member].name.clone();
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must not host guests"
    );
    let mut steps = Vec::new();
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let details = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&member).cloned())
    };
    let live = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .hosts
                .get(&member)
                .and_then(|h| h.metrics)
                .and_then(|m| i.details.host_metrics.get(&m))
                .and_then(|m| m.live)
        })
    };
    if details(&c).and_then(|h| h.maintenance_mode) == Some(true) {
        let entry = resource_op(
            &c,
            "clear the leftover maintenance marker",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        note(&mut steps, entry);
    }
    let enabled = c.latest().inventory().unwrap().hosts[&member].enabled;
    let mut entry = resource_op(
        &c,
        "enter maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: enabled,
                resident_vms: Vec::new(),
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || details(&c).and_then(|h| h.maintenance_mode)
            == Some(true))
        .await
    );
    note(&mut steps, entry);
    let boot_before = details(&c).and_then(|h| h.boot_time).expect("boot time");
    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "shut the member down",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::Shutdown {
                expected_boot_time: boot_before,
            },
        },
        900,
    )
    .await;
    entry["read_back"] = json!(settle(600, || live(&c) == Some(false)).await);
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    entry["enabled_after"] = json!(c.latest().inventory().unwrap().hosts[&member].enabled);
    note(&mut steps, entry);
    c.close().await;
    lab.record(
        "member-shutdown-through-maintenance",
        json!({"member": member_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P19-05, P12-03, P10-04, P10-09, P09-06: after the operator powers the shut-down member's
/// domain back on, wait for it to report live, clear the maintenance marker the restart left
/// behind, rescan its interfaces and confirm both storage NICs by MAC (the second one appears
/// only after a real power cycle), make sure the first carries the static storage address
/// and is attached, and attach the shared repository on the member.
#[tokio::test]
#[ignore]
async fn lab_member_return_and_second_storage_nic() {
    use xenadmin_core::{
        HostOperation, Ipv4Configuration, Ipv4Mode, NetworkOperation, PifIdentity, ResourceCommand,
        StorageOperation,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("the manifest records a storage network");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator)
        .expect("member");
    let member_name = inventory.hosts[&member].name.clone();
    let lab_index = lab
        .hosts
        .iter()
        .position(|h| h.name == member_name)
        .expect("member in the manifest");
    let macs = storage
        .host_nics
        .get(&member_name)
        .cloned()
        .unwrap_or_default();
    assert!(macs.len() >= 2, "two storage NICs recorded for the member");
    let prefix = storage
        .subnet
        .split('/')
        .next()
        .and_then(|a| a.rsplit_once('.'))
        .map(|(p, _)| p.to_string())
        .expect("storage subnet prefix");
    let address = format!("{prefix}.{}", 11 + lab_index);
    let mut steps = Vec::new();
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let details = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&member).cloned())
    };
    let live = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .hosts
                .get(&member)
                .and_then(|h| h.metrics)
                .and_then(|m| i.details.host_metrics.get(&m))
                .and_then(|m| m.live)
        })
    };
    let pif_by_mac = |c: &Connection, mac: &str| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pifs
                .values()
                .find(|p| p.host == Some(member) && p.mac.eq_ignore_ascii_case(mac))
                .cloned()
        })
    };
    let started = std::time::Instant::now();
    let back = settle(900, || live(&c) == Some(true)).await;
    note(
        &mut steps,
        json!({"step": "member live after the power cycle", "read_back": back, "elapsed_ms": started.elapsed().as_millis() as u64, "enabled": c.latest().inventory().unwrap().hosts[&member].enabled, "maintenance_marker": details(&c).and_then(|h| h.maintenance_mode)}),
    );
    if details(&c).and_then(|h| h.maintenance_mode) == Some(true) {
        let mut entry = resource_op(
            &c,
            "exit maintenance",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || c
                .latest()
                .inventory()
                .is_some_and(|i| i.hosts[&member].enabled))
            .await
        );
        note(&mut steps, entry);
    }
    let mut entry = resource_op(
        &c,
        "rescan physical interfaces",
        ResourceCommand::Network {
            target: host_target(&c),
            operation: NetworkOperation::ScanInterfaces,
        },
        300,
    )
    .await;
    entry["read_back"] =
        json!(settle(120, || macs.iter().all(|m| pif_by_mac(&c, m).is_some())).await);
    note(&mut steps, entry);
    note(
        &mut steps,
        json!({"step": "storage interfaces", "pifs": macs.iter().map(|m| pif_by_mac(&c, m).map(|p| json!({"device": p.device, "mode": p.ip_configuration_mode, "ip": p.ip, "attached": p.currently_attached}))).collect::<Vec<_>>()}),
    );
    let Some(first) = pif_by_mac(&c, &macs[0]) else {
        lab.record(
            "member-return-and-second-storage-nic",
            json!({"member": member_name, "failed": "first storage NIC missing", "steps": steps}),
        );
        panic!("first storage NIC missing: {steps:?}");
    };
    if !(first.ip_configuration_mode == "Static" && first.ip == address) {
        let before = Ipv4Configuration {
            mode: match first.ip_configuration_mode.as_str() {
                "DHCP" => Ipv4Mode::Dhcp,
                "Static" => Ipv4Mode::Static,
                _ => Ipv4Mode::None,
            },
            address: first.ip.clone(),
            netmask: first.netmask.clone(),
            gateway: first.gateway.clone(),
            dns: first.dns.clone(),
        };
        let mut entry = resource_op(
            &c,
            "configure the static storage address",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(first.id))
                    .unwrap(),
                operation: NetworkOperation::ConfigureIpv4 {
                    expected: PifIdentity::from_inventory(&first).expect("interface identity"),
                    before,
                    configuration: Ipv4Configuration {
                        mode: Ipv4Mode::Static,
                        address: address.clone(),
                        netmask: "255.255.255.0".into(),
                        gateway: String::new(),
                        dns: String::new(),
                    },
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || pif_by_mac(&c, &macs[0]).is_some_and(|p| p.ip
                == address
                && p.ip_configuration_mode == "Static"))
            .await
        );
        note(&mut steps, entry);
    }
    let first = pif_by_mac(&c, &macs[0]).expect("first storage NIC");
    if first.currently_attached != Some(true) {
        let mut entry = resource_op(
            &c,
            "attach the storage interface",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(first.id))
                    .unwrap(),
                operation: NetworkOperation::SetAttached {
                    expected: PifIdentity::from_inventory(&first).expect("interface identity"),
                    expected_attached: false,
                    attached: true,
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || pif_by_mac(&c, &macs[0])
                .is_some_and(|p| p.currently_attached == Some(true)))
            .await
        );
        note(&mut steps, entry);
    }
    if let Some(sr) = c
        .latest()
        .inventory()
        .unwrap()
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| xenadmin_core::SrId::from(s.uuid))
    {
        let member_pbd = |c: &Connection| {
            c.latest().inventory().and_then(|i| {
                i.details
                    .pbds
                    .values()
                    .find(|p| p.sr == Some(sr) && p.host == Some(member))
                    .map(|p| (p.id, p.currently_attached))
            })
        };
        match member_pbd(&c) {
            Some((_, Some(true))) => note(
                &mut steps,
                json!({"step": "shared repository on the member", "already_attached": true}),
            ),
            Some((pbd, _)) => {
                let mut entry = resource_op(
                    &c,
                    "attach the shared repository on the member",
                    ResourceCommand::Storage {
                        target: c
                            .latest()
                            .observe_resource(ResourceTarget::Pbd(pbd))
                            .unwrap(),
                        operation: StorageOperation::SetAttached {
                            sr,
                            host: member,
                            expected_attached: false,
                            attached: true,
                        },
                    },
                    600,
                )
                .await;
                entry["read_back"] = json!(
                    settle(180, || member_pbd(&c).is_some_and(|(_, a)| a == Some(true))).await
                );
                note(&mut steps, entry);
            }
            None => note(
                &mut steps,
                json!({"step": "shared repository on the member", "skipped": "no attachment record"}),
            ),
        }
    }
    c.close().await;
    lab.record(
        "member-return-and-second-storage-nic",
        json!({"member": member_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P10-04, P10-09, P09-02, P09-06: repair the member's storage path. Probe the appliance's
/// exports through the member; when the host cannot reach it although its storage interface
/// record says attached, forget that record, rescan so the kernel's current device is
/// introduced again, configure and attach it, and probe once more; then attach the shared
/// repository on the member. A member that already reaches the appliance only records that.
#[tokio::test]
#[ignore]
async fn lab_member_storage_interface_repair() {
    use xenadmin_core::{
        Ipv4Configuration, Ipv4Mode, NetworkOperation, NfsVersion, PifIdentity, ResourceCommand,
        StorageDiscoveryResult, StorageOperation, StorageQuery,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("the manifest records a storage network");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let member_name = inventory.hosts[&member].name.clone();
    let lab_index = lab
        .hosts
        .iter()
        .position(|h| h.name == member_name)
        .expect("member in the manifest");
    let macs = storage
        .host_nics
        .get(&member_name)
        .cloned()
        .unwrap_or_default();
    assert!(!macs.is_empty(), "storage NICs recorded for the member");
    let prefix = storage
        .subnet
        .split('/')
        .next()
        .and_then(|a| a.rsplit_once('.'))
        .map(|(p, _)| p.to_string())
        .expect("storage subnet prefix");
    let address = format!("{prefix}.{}", 11 + lab_index);
    let mut steps = Vec::new();
    let pif_by_mac = |c: &Connection, mac: &str| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pifs
                .values()
                .find(|p| p.host == Some(member) && p.mac.eq_ignore_ascii_case(mac))
                .cloned()
        })
    };
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let probe = |c: &Connection| {
        let probe = c
            .discover_storage(
                c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                StorageQuery::NfsExports {
                    host: member,
                    server: storage.nas_ip.clone(),
                    version: NfsVersion::Automatic,
                },
            )
            .expect("probe accepted");
        async move {
            let result = tokio::time::timeout(deadline(180), async {
                loop {
                    if let Some(result) = probe.latest() {
                        break result;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            })
            .await
            .expect("probe answered");
            match result {
                Ok(StorageDiscoveryResult::NfsExports(rows)) => {
                    Ok(rows.into_iter().map(|r| r.path).collect::<Vec<_>>())
                }
                Ok(_) => Err("unexpected discovery result".to_string()),
                Err(e) => Err(e),
            }
        }
    };
    let first = pif_by_mac(&c, &macs[0]).expect("storage interface record");
    note(
        &mut steps,
        json!({"step": "storage interface before", "device": first.device, "mode": first.ip_configuration_mode, "ip": first.ip, "attached": first.currently_attached}),
    );
    let before = probe(&c).await;
    note(
        &mut steps,
        json!({"step": "probe the appliance through the member", "exports": before.as_ref().ok(), "error": before.as_ref().err(), "reachable": before.is_ok()}),
    );
    if before.is_err() {
        // Forget the record, rescan, configure and attach the re-introduced interface.
        let mut entry = resource_op(
            &c,
            "forget the storage interface record",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(first.id))
                    .unwrap(),
                operation: NetworkOperation::ForgetInterface {
                    expected: PifIdentity::from_inventory(&first).expect("interface identity"),
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(60, || c
                .latest()
                .inventory()
                .is_some_and(|i| !i.details.pifs.contains_key(&first.id)))
            .await
        );
        note(&mut steps, entry);
        let mut entry = resource_op(
            &c,
            "rescan physical interfaces",
            ResourceCommand::Network {
                target: host_target(&c),
                operation: NetworkOperation::ScanInterfaces,
            },
            300,
        )
        .await;
        entry["read_back"] = json!(settle(120, || pif_by_mac(&c, &macs[0]).is_some()).await);
        note(&mut steps, entry);
        let Some(fresh) = pif_by_mac(&c, &macs[0]) else {
            lab.record("member-storage-interface-repair", json!({"member": member_name, "failed": "interface not re-introduced", "steps": steps}));
            panic!("interface not re-introduced: {steps:?}");
        };
        note(
            &mut steps,
            json!({"step": "storage interface re-introduced", "device": fresh.device, "mode": fresh.ip_configuration_mode, "attached": fresh.currently_attached, "all_storage_pifs": macs.iter().map(|m| pif_by_mac(&c, m).map(|p| p.device)).collect::<Vec<_>>()}),
        );
        let mut entry = resource_op(
            &c,
            "configure the static storage address",
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(fresh.id))
                    .unwrap(),
                operation: NetworkOperation::ConfigureIpv4 {
                    expected: PifIdentity::from_inventory(&fresh).expect("interface identity"),
                    before: Ipv4Configuration {
                        mode: match fresh.ip_configuration_mode.as_str() {
                            "DHCP" => Ipv4Mode::Dhcp,
                            "Static" => Ipv4Mode::Static,
                            _ => Ipv4Mode::None,
                        },
                        address: fresh.ip.clone(),
                        netmask: fresh.netmask.clone(),
                        gateway: fresh.gateway.clone(),
                        dns: fresh.dns.clone(),
                    },
                    configuration: Ipv4Configuration {
                        mode: Ipv4Mode::Static,
                        address: address.clone(),
                        netmask: "255.255.255.0".into(),
                        gateway: String::new(),
                        dns: String::new(),
                    },
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || pif_by_mac(&c, &macs[0]).is_some_and(|p| p.ip
                == address
                && p.ip_configuration_mode == "Static"))
            .await
        );
        note(&mut steps, entry);
        let fresh = pif_by_mac(&c, &macs[0]).expect("storage interface");
        if fresh.currently_attached == Some(true) {
            note(
                &mut steps,
                json!({"step": "attach the storage interface", "skipped": "already attached after the rescan"}),
            );
        } else {
            let mut entry = resource_op(
                &c,
                "attach the storage interface",
                ResourceCommand::Network {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Pif(fresh.id))
                        .unwrap(),
                    operation: NetworkOperation::SetAttached {
                        expected: PifIdentity::from_inventory(&fresh).expect("interface identity"),
                        expected_attached: false,
                        attached: true,
                    },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(
                settle(120, || pif_by_mac(&c, &macs[0])
                    .is_some_and(|p| p.currently_attached == Some(true)))
                .await
            );
            note(&mut steps, entry);
        }
        let after = probe(&c).await;
        note(
            &mut steps,
            json!({"step": "probe the appliance through the member after the repair", "exports": after.as_ref().ok(), "error": after.as_ref().err(), "read_back": after.is_ok()}),
        );
    }
    if let Some(sr) = c
        .latest()
        .inventory()
        .unwrap()
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| xenadmin_core::SrId::from(s.uuid))
    {
        let member_pbd = |c: &Connection| {
            c.latest().inventory().and_then(|i| {
                i.details
                    .pbds
                    .values()
                    .find(|p| p.sr == Some(sr) && p.host == Some(member))
                    .map(|p| (p.id, p.currently_attached))
            })
        };
        match member_pbd(&c) {
            Some((_, Some(true))) => note(
                &mut steps,
                json!({"step": "shared repository on the member", "already_attached": true}),
            ),
            Some((pbd, _)) => {
                let mut entry = resource_op(
                    &c,
                    "attach the shared repository on the member",
                    ResourceCommand::Storage {
                        target: c
                            .latest()
                            .observe_resource(ResourceTarget::Pbd(pbd))
                            .unwrap(),
                        operation: StorageOperation::SetAttached {
                            sr,
                            host: member,
                            expected_attached: false,
                            attached: true,
                        },
                    },
                    600,
                )
                .await;
                entry["read_back"] = json!(
                    settle(180, || member_pbd(&c).is_some_and(|(_, a)| a == Some(true))).await
                );
                note(&mut steps, entry);
            }
            None => note(
                &mut steps,
                json!({"step": "shared repository on the member", "skipped": "no attachment record"}),
            ),
        }
    }
    c.close().await;
    lab.record(
        "member-storage-interface-repair",
        json!({"member": member_name, "appliance": storage.nas_ip, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P03-01, P06-11: clean-reboot the tools guest through the client (the agent makes the
/// clean operations available), wait for the agent to report again after the restart, and
/// record whether it now reports an address from the lab network's DHCP range.
#[tokio::test]
#[ignore]
async fn lab_tools_guest_clean_reboot_and_address() {
    use xenadmin_core::{PowerAction, PowerState, VmCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let guest = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned()
        .expect("the tools guest");
    let vm = guest.id;
    let mut steps = Vec::new();
    if guest.power_state != PowerState::Running {
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the tools guest");
        note(&mut steps, start);
    }
    let metrics_start = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vms
                .get(&vm)
                .and_then(|v| v.metrics)
                .and_then(|m| i.details.vm_metrics.get(&m))
                .and_then(|m| m.start_time.clone())
        })
    };
    let agent = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            let g = i.details.vms.get(&vm)?.guest_metrics?;
            let m = i.details.guest_metrics.get(&g)?;
            Some((m.tools_version.clone(), m.networks.clone()))
        })
    };
    let advertised = settle(60, || {
        c.latest().inventory().is_some_and(|i| {
            i.vms
                .get(&vm)
                .is_some_and(|v| v.allowed_actions.contains(&PowerAction::Reboot))
        })
    })
    .await;
    note(
        &mut steps,
        json!({"step": "clean reboot advertised with the agent present", "read_back": advertised, "allowed_actions": c.latest().inventory().unwrap().vms[&vm].allowed_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>()}),
    );
    let start_before = metrics_start(&c);
    let started = std::time::Instant::now();
    let reboot = c
        .submit(
            OperationId::new(),
            VmCommand::Power {
                vm: c.latest().observe_vm(vm).unwrap(),
                action: PowerAction::Reboot,
            },
        )
        .expect("clean reboot accepted");
    let row = wait_vm_outcome(&reboot, 300).await;
    let restarted = settle(240, || {
        metrics_start(&c).is_some_and(|t| Some(&t) != start_before.as_ref())
            && c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&vm].power_state == PowerState::Running)
    })
    .await;
    note(
        &mut steps,
        json!({"step": "clean reboot", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": restarted, "elapsed_ms": started.elapsed().as_millis() as u64}),
    );
    let reported = settle(240, || {
        agent(&c).is_some_and(|(tools, _)| !tools.is_empty())
    })
    .await;
    let addressed = settle(120, || {
        agent(&c).is_some_and(|(_, networks)| !networks.is_empty())
    })
    .await;
    let (tools, networks) = agent(&c).unwrap_or_default();
    note(
        &mut steps,
        json!({"step": "agent after the restart", "read_back": reported, "address_reported": addressed, "tools_version": tools, "network_keys": networks.keys().collect::<Vec<_>>(), "addresses": networks.values().map(|v| if v.contains(':') { "<ipv6>".to_string() } else { v.rsplit_once('.').map(|(p, _)| format!("{p}.x")).unwrap_or_default() }).collect::<Vec<_>>()}),
    );
    c.close().await;
    lab.record(
        "tools-guest-clean-reboot-and-address",
        json!({"guest": guest.name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("reached").is_some() {
            assert_eq!(step["reached"], true, "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Utility: cleanly shut the tools guest down through the client (its agent makes the
/// clean operations available) so the appliance can be restarted without guest I/O.
#[tokio::test]
#[ignore]
async fn lab_tools_guest_clean_shutdown() {
    use xenadmin_core::{PowerAction, PowerState};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let Some(guest) = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned()
    else {
        eprintln!("no tools guest");
        return;
    };
    if guest.power_state == PowerState::Running {
        let outcome = power_to(&c, guest.id, PowerAction::CleanShutdown, PowerState::Halted).await;
        eprintln!("clean shutdown: {outcome}");
    } else {
        eprintln!("tools guest already {:?}", guest.power_state);
    }
    c.close().await;
}

/// Utility: start the tools guest if it is halted.
#[tokio::test]
#[ignore]
async fn lab_tools_guest_start() {
    use xenadmin_core::{PowerAction, PowerState};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    if let Some(guest) = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned()
        && guest.power_state != PowerState::Running
    {
        let outcome = power_to(&c, guest.id, PowerAction::Start, PowerState::Running).await;
        eprintln!("start: {outcome}");
    }
    c.close().await;
}

/// P09-02, P09-03, P09-06, P09-08: discover the appliance's iSCSI targets and the open
/// target's LUN through the coordinator, confirm the CHAP-protected target refuses an
/// unauthenticated LUN probe, create (or reuse) a shared LVM-over-iSCSI repository on the
/// open target and confirm both hosts attach it, detach and reattach it on the member, and
/// rescan it. The repository is kept for the multipath scenario.
#[tokio::test]
#[ignore]
async fn lab_iscsi_targets_and_lvm_repository() {
    use xenadmin_core::{
        NfsVersion, ResourceCommand, StorageAllocation, StorageDiscoveryResult, StorageLocation,
        StorageOperation, StorageQuery,
    };
    let _ = NfsVersion::Automatic;
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("storage network in the manifest");
    let iscsi = storage
        .iscsi
        .as_ref()
        .expect("iSCSI targets in the manifest");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator && inventory.hosts[h].enabled)
        .expect("enabled member");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let discover = |c: &Connection, query: StorageQuery| {
        let probe = c
            .discover_storage(
                c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                query,
            )
            .expect("probe accepted");
        async move {
            tokio::time::timeout(deadline(240), async {
                loop {
                    if let Some(result) = probe.latest() {
                        break result;
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            })
            .await
            .expect("probe answered")
        }
    };
    let targets = discover(
        &c,
        StorageQuery::IscsiTargets {
            host: coordinator,
            portal: storage.nas_ip.clone(),
            port: iscsi.port,
        },
    )
    .await;
    let iqns = match &targets {
        Ok(StorageDiscoveryResult::IscsiTargets(rows)) => Some(
            rows.iter()
                .map(|t| json!({"iqn": t.iqn, "portal": t.portal, "port": t.port}))
                .collect::<Vec<_>>(),
        ),
        _ => None,
    };
    note(
        &mut steps,
        json!({"step": "discover iSCSI targets", "targets": iqns, "error": targets.as_ref().err(), "read_back": iqns.as_ref().is_some_and(|t| t.iter().any(|x| x["iqn"] == iscsi.open_iqn))}),
    );
    let luns = discover(
        &c,
        StorageQuery::IscsiLuns {
            host: coordinator,
            portal: storage.nas_ip.clone(),
            port: iscsi.port,
            target_iqn: iscsi.open_iqn.clone(),
        },
    )
    .await;
    let lun_rows = match &luns {
        Ok(StorageDiscoveryResult::IscsiLuns(rows)) => Some(rows.iter().map(|l| json!({"scsi_id": l.scsi_id, "lun": l.lun, "size_bytes": l.size_bytes, "vendor": l.vendor})).collect::<Vec<_>>()),
        _ => None,
    };
    note(
        &mut steps,
        json!({"step": "discover the open target's LUNs", "luns": lun_rows, "error": luns.as_ref().err(), "read_back": lun_rows.as_ref().is_some_and(|l| !l.is_empty())}),
    );
    let refused = discover(
        &c,
        StorageQuery::IscsiLuns {
            host: coordinator,
            portal: storage.nas_ip.clone(),
            port: iscsi.port,
            target_iqn: iscsi.chap_iqn.clone(),
        },
    )
    .await;
    note(
        &mut steps,
        json!({"step": "unauthenticated LUN probe of the CHAP target", "error": refused.as_ref().err(), "expected_refusal": true, "refused": refused.is_err()}),
    );
    let scsi_id = match &luns {
        Ok(StorageDiscoveryResult::IscsiLuns(rows)) => rows.first().map(|l| l.scsi_id.clone()),
        _ => None,
    };
    let Some(scsi_id) = scsi_id else {
        lab.record(
            "iscsi-targets-and-lvm-repository",
            json!({"appliance": storage.nas_ip, "failed": "no LUN discovered", "steps": steps}),
        );
        panic!("no LUN discovered: {steps:?}");
    };

    let find_sr = |c: &Connection, prefix: &str| {
        c.latest().inventory().and_then(|i| {
            i.storage
                .iter()
                .find(|s| s.name.starts_with(prefix) && !s.name.starts_with("lab-iscsi-chap"))
                .map(|s| (xenadmin_core::SrId::from(s.uuid), s.name.clone()))
        })
    };
    let attachments = |c: &Connection, sr: xenadmin_core::SrId| {
        c.latest()
            .inventory()
            .map(|i| {
                i.details
                    .pbds
                    .values()
                    .filter(|p| p.sr == Some(sr))
                    .map(|p| (p.id, p.host, p.currently_attached))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let (sr, sr_name) = if let Some(existing) = find_sr(&c, "lab-iscsi-") {
        note(
            &mut steps,
            json!({"step": "create LVM-over-iSCSI repository", "reused": existing.1}),
        );
        existing
    } else {
        let name = format!("lab-iscsi-{stamp}");
        let mut entry = resource_op(
            &c,
            "create LVM-over-iSCSI repository",
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                operation: StorageOperation::Create {
                    host: coordinator,
                    name: name.clone(),
                    description: "Shared LVM-over-iSCSI repository on the lab appliance".into(),
                    location: StorageLocation::Iscsi {
                        portal: storage.nas_ip.clone(),
                        port: iscsi.port,
                        target_iqn: iscsi.open_iqn.clone(),
                        scsi_id: scsi_id.clone(),
                        allocation: StorageAllocation::DriverDefault,
                    },
                    hba_device: None,
                    local_device: None,
                },
            },
            900,
        )
        .await;
        entry["read_back"] = json!(settle(180, || find_sr(&c, &name).is_some()).await);
        note(&mut steps, entry);
        match find_sr(&c, &name) {
            Some(found) => found,
            None => {
                lab.record("iscsi-targets-and-lvm-repository", json!({"appliance": storage.nas_ip, "failed": "repository not listed", "steps": steps}));
                panic!("repository not listed: {steps:?}");
            }
        }
    };
    let shared = settle(240, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .storage
                .get(&sr)
                .is_some_and(|d| d.shared == Some(true))
        }) && attachments(&c, sr).len() == 2
            && attachments(&c, sr).iter().all(|(_, _, a)| *a == Some(true))
    })
    .await;
    note(
        &mut steps,
        json!({"step": "repository attached on both hosts", "repository": sr_name, "read_back": shared, "kind": c.latest().inventory().unwrap().storage.iter().find(|s| xenadmin_core::SrId::from(s.uuid) == sr).map(|s| s.kind.clone()), "attachments": attachments(&c, sr).iter().map(|(_, h, a)| json!({"host": h.map(|h| inventory.hosts[&h].name.clone()), "attached": a})).collect::<Vec<_>>()}),
    );
    for (label, attached) in [
        ("detach the repository on the member", false),
        ("reattach the repository on the member", true),
    ] {
        let Some((pbd, _, _)) = attachments(&c, sr)
            .into_iter()
            .find(|(_, h, _)| *h == Some(member))
        else {
            break;
        };
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pbd(pbd))
                    .unwrap(),
                operation: StorageOperation::SetAttached {
                    sr,
                    host: member,
                    expected_attached: !attached,
                    attached,
                },
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(180, || attachments(&c, sr)
                .iter()
                .any(|(_, h, a)| *h == Some(member) && *a == Some(attached)))
            .await
        );
        if entry["state"] == "Rejected"
            && entry["detail"]
                .as_str()
                .is_some_and(|d| d.contains("in use"))
        {
            entry["expected_refusal"] = json!(true);
            note(&mut steps, entry);
            break;
        }
        note(&mut steps, entry);
    }
    let entry = resource_op(
        &c,
        "rescan the repository",
        ResourceCommand::ScanStorage {
            sr: c
                .latest()
                .observe_resource(ResourceTarget::Storage(sr))
                .unwrap(),
        },
        300,
    )
    .await;
    note(&mut steps, entry);
    c.close().await;
    lab.record(
        "iscsi-targets-and-lvm-repository",
        json!({"appliance": storage.nas_ip, "repository": sr_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("expected_refusal").is_some() {
            if step.get("refused").is_some() {
                assert_eq!(step["refused"], true, "{step}");
            } else {
                assert_eq!(step["state"], "Rejected", "{step}");
            }
            continue;
        }
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Detach a repository on every host, members first and the coordinator last, waiting for
/// the host to advertise `unplug` before each detach, then destroy or forget it; every step is recorded
/// with the wait and whether the detach was advertised when it was submitted.
async fn remove_repository(
    c: &Connection,
    inventory: &xenadmin_core::Inventory,
    sr: xenadmin_core::SrId,
    expected_name: &str,
    destroy: bool,
    steps: &mut Vec<Value>,
) {
    use xenadmin_core::{ResourceCommand, StorageOperation};
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator);
    let attachments = |c: &Connection| {
        c.latest()
            .inventory()
            .map(|i| {
                i.details
                    .pbds
                    .values()
                    .filter(|p| p.sr == Some(sr))
                    .map(|p| (p.id, p.host, p.currently_attached))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let unplug_advertised = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| {
                i.details
                    .storage
                    .get(&sr)
                    .and_then(|d| d.allowed_operations.clone())
            })
            .is_some_and(|ops| ops.iter().any(|op| op == "unplug"))
    };
    let mut attached = attachments(c)
        .into_iter()
        .filter(|(_, _, a)| *a == Some(true))
        .collect::<Vec<_>>();
    attached.sort_by_key(|(_, host, _)| host.is_some() && *host == coordinator);
    for (pbd, host, _) in attached {
        let Some(host) = host else { continue };
        let waited = std::time::Instant::now();
        let advertised = settle(120, || unplug_advertised(c)).await;
        let mut entry = resource_op(
            c,
            &format!("detach {expected_name} on {}", inventory.hosts[&host].name),
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pbd(pbd))
                    .unwrap(),
                operation: StorageOperation::SetAttached {
                    sr,
                    host,
                    expected_attached: true,
                    attached: false,
                },
            },
            600,
        )
        .await;
        entry["unplug_advertised"] = json!(advertised);
        entry["waited_ms"] = json!(waited.elapsed().as_millis() as u64);
        entry["read_back"] = json!(
            entry["state"] == "Succeeded"
                && settle(180, || attachments(c)
                    .iter()
                    .any(|(p, _, a)| *p == pbd && *a == Some(false)))
                .await
        );
        note(steps, entry);
    }
    let expected_disks = c
        .latest()
        .inventory()
        .and_then(|i| i.details.sr_disks.get(&sr).cloned())
        .unwrap_or_default();
    let operation = if destroy {
        StorageOperation::Destroy {
            expected_name: expected_name.to_string(),
            expected_disks,
        }
    } else {
        StorageOperation::Forget {
            expected_name: expected_name.to_string(),
            expected_disks,
        }
    };
    let mut entry = resource_op(
        c,
        &format!(
            "{} {expected_name}",
            if destroy { "destroy" } else { "forget" }
        ),
        ResourceCommand::Storage {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Storage(sr))
                .unwrap(),
            operation,
        },
        600,
    )
    .await;
    entry["read_back"] = json!(
        entry["state"] == "Succeeded"
            && settle(120, || c.latest().inventory().is_some_and(|i| {
                !i.storage
                    .iter()
                    .any(|s| xenadmin_core::SrId::from(s.uuid) == sr)
            }))
            .await
    );
    note(steps, entry);
}

/// P09-03 (CHAP): probe the CHAP-protected target's LUN with transient credentials, create a
/// shared LVM-over-iSCSI repository on it through the authenticated path and confirm both
/// hosts attach it with credentials required, then detach it everywhere and destroy it so no
/// CHAP material stays on the pool.
#[tokio::test]
#[ignore]
async fn lab_iscsi_chap_repository_with_transient_credentials() {
    use xenadmin_core::{
        StorageAllocation, StorageCredentials, StorageDiscoveryResult, StorageLocation,
        StorageOperation, StorageQuery,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("storage network in the manifest");
    let iscsi = storage
        .iscsi
        .as_ref()
        .expect("iSCSI targets in the manifest");
    let Some((user, secret)) = iscsi.chap.clone() else {
        eprintln!("XENADMIN_LAB_CHAP_FILE not set; skipping");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let leftovers = inventory
        .storage
        .iter()
        .filter(|s| s.name.starts_with("lab-iscsi-chap-"))
        .map(|s| (xenadmin_core::SrId::from(s.uuid), s.name.clone()))
        .collect::<Vec<_>>();
    for (sr, leftover) in leftovers {
        note(
            &mut steps,
            json!({"step": "remove a repository left by an earlier attempt", "repository": leftover}),
        );
        remove_repository(&c, &inventory, sr, &leftover, true, &mut steps).await;
    }
    let probe = c
        .discover_authenticated_storage(
            c.latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .unwrap(),
            StorageQuery::IscsiLuns {
                host: coordinator,
                portal: storage.nas_ip.clone(),
                port: iscsi.port,
                target_iqn: iscsi.chap_iqn.clone(),
            },
            StorageCredentials::chap(user.clone(), secret.clone()),
        )
        .expect("authenticated probe accepted");
    let result = tokio::time::timeout(deadline(240), async {
        loop {
            if let Some(result) = probe.latest() {
                break result;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .expect("probe answered");
    let scsi_id = match &result {
        Ok(StorageDiscoveryResult::IscsiLuns(rows)) => rows.first().map(|l| l.scsi_id.clone()),
        _ => None,
    };
    note(
        &mut steps,
        json!({"step": "authenticated LUN probe of the CHAP target", "error": result.as_ref().err(), "read_back": scsi_id.is_some(), "luns": match &result { Ok(StorageDiscoveryResult::IscsiLuns(rows)) => rows.len(), _ => 0 }}),
    );
    let Some(scsi_id) = scsi_id else {
        lab.record(
            "iscsi-chap-repository",
            json!({"failed": "no LUN discovered with credentials", "steps": steps}),
        );
        panic!("no LUN discovered with credentials: {steps:?}");
    };
    let name = format!("lab-iscsi-chap-{stamp}");
    let find_sr = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.storage
                .iter()
                .find(|s| s.name == name)
                .map(|s| xenadmin_core::SrId::from(s.uuid))
        })
    };
    let attachments = |c: &Connection, sr: xenadmin_core::SrId| {
        c.latest()
            .inventory()
            .map(|i| {
                i.details
                    .pbds
                    .values()
                    .filter(|p| p.sr == Some(sr))
                    .map(|p| (p.id, p.host, p.currently_attached, p.requires_credentials))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let started = std::time::Instant::now();
    let create = c
        .submit_authenticated_storage(
            OperationId::new(),
            c.latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .unwrap(),
            StorageOperation::Create {
                host: coordinator,
                name: name.clone(),
                description: "CHAP-protected LVM-over-iSCSI repository on the lab appliance".into(),
                location: StorageLocation::Iscsi {
                    portal: storage.nas_ip.clone(),
                    port: iscsi.port,
                    target_iqn: iscsi.chap_iqn.clone(),
                    scsi_id,
                    allocation: StorageAllocation::DriverDefault,
                },
                hba_device: None,
                local_device: None,
            },
            StorageCredentials::chap(user, secret),
        )
        .expect("authenticated creation accepted");
    let row = wait_outcome(&create, 900).await;
    let listed = settle(180, || find_sr(&c).is_some()).await;
    note(
        &mut steps,
        json!({"step": "create the CHAP repository with transient credentials", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "elapsed_ms": started.elapsed().as_millis() as u64, "read_back": listed,
        "steps": row.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state)})).collect::<Vec<_>>()}),
    );
    if let Some(sr) = find_sr(&c) {
        let both = settle(240, || {
            attachments(&c, sr).len() == 2
                && attachments(&c, sr)
                    .iter()
                    .all(|(_, _, a, _)| *a == Some(true))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "CHAP repository attached on both hosts", "read_back": both, "attachments": attachments(&c, sr).iter().map(|(_, h, a, cred)| json!({"host": h.map(|h| inventory.hosts[&h].name.clone()), "attached": a, "requires_credentials": cred})).collect::<Vec<_>>()}),
        );
        remove_repository(&c, &inventory, sr, &name, true, &mut steps).await;
    }
    c.close().await;
    lab.record(
        "iscsi-chap-repository",
        json!({"appliance": storage.nas_ip, "repository": name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P01-04, P12-10, P12-05, P12-11: with HA enabled on the shared heartbeat repository and the
/// tools guest protected and resident on the coordinator, cut power to the coordinator's
/// domain under a live session. The surviving member must take over as coordinator, the
/// client must follow it through a remembered member endpoint without operator input, and
/// HA must restart the protected guest on the survivor. Then start the old coordinator again,
/// confirm it rejoins as a member, disable HA, hand the coordinator role back, restore the
/// guest placement and restart the local-storage guest the power cut halted.
#[tokio::test]
#[ignore]
async fn lab_coordinator_loss_with_ha_member_takeover() {
    use xenadmin_core::{
        ConnectionState, DiskPlacement, HaRestartPriority, NetworkOperation, NicPlacement,
        PifIdentity, PoolOperation, PowerAction, PowerState, ResourceCommand, VmCommand,
        VmMigration,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory.coordinator.expect("coordinator");
    assert_eq!(
        inventory.hosts[&coordinator].address, lab.hosts[0].ip,
        "the first manifest host is expected to be the coordinator"
    );
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator)
        .expect("member");
    let shared: xenadmin_core::SrId = inventory
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| s.uuid.into())
        .expect("the shared NFS repository");
    let guest = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned()
        .expect("tools guest");
    let local_guest = inventory
        .vms
        .values()
        .find(|v| v.name.starts_with("lab-guest-") && !v.is_template)
        .cloned();
    let mut steps = Vec::new();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let administration = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pool.as_ref().map(|p| p.administration.clone()))
    };
    let live = |c: &Connection, host: HostId| {
        c.latest().inventory().and_then(|i| {
            i.details
                .hosts
                .get(&host)
                .and_then(|h| h.metrics)
                .and_then(|m| i.details.host_metrics.get(&m))
                .and_then(|m| m.live)
        })
    };
    let priority_of = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .vms
                .get(&guest.id)
                .and_then(|d| d.ha_restart_priority.clone())
        })
    };
    let placement = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.vms
                .get(&guest.id)
                .map(|v| (v.host, v.power_state.clone()))
        })
    };
    let migrate = |c: &Connection, destination: HostId| {
        let now = c.latest().inventory().unwrap().clone();
        let disks = now.details.vm_disks[&guest.id]
            .iter()
            .filter_map(|b| now.details.vbds.get(b))
            .filter(|b| b.kind == "Disk")
            .filter_map(|b| {
                let vdi = b.vdi?;
                let sr = now.details.vdis.get(&vdi)?.sr?;
                Some(DiskPlacement {
                    vdi,
                    expected_sr: sr,
                    destination_sr: sr,
                })
            })
            .collect::<Vec<_>>();
        let networks = now
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
        c.submit(
            OperationId::new(),
            VmCommand::Migrate {
                vm: c.latest().observe_vm(guest.id).unwrap(),
                operation: VmMigration {
                    destination,
                    expected_host: now.vms[&guest.id].host,
                    expected_affinity: None,
                    live: true,
                    disks,
                    networks,
                    transfer_network: None,
                },
            },
        )
    };

    // The protected guest must be resident on the coordinator so that HA has to restart it.
    if guest.host != Some(coordinator) {
        let migration = migrate(&c, coordinator).expect("migration accepted");
        let row = wait_vm_outcome(&migration, 900).await;
        let placed = settle(120, || {
            placement(&c) == Some((Some(coordinator), PowerState::Running))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "move the tools guest onto the coordinator", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": placed}),
        );
    }
    // Finding 36: the storage interfaces that carry the heartbeat repository must be kept
    // attached at boot, or a rebooted host cannot rejoin the pool while HA is armed.
    let shared_server: std::net::Ipv4Addr = lab
        .storage
        .as_ref()
        .and_then(|s| s.nas_ip.parse().ok())
        .expect("appliance storage address");
    let heartbeat_pifs = inventory
        .details
        .pifs
        .values()
        .filter(|p| {
            let (Ok(ip), Ok(mask)) = (
                p.ip.parse::<std::net::Ipv4Addr>(),
                p.netmask.parse::<std::net::Ipv4Addr>(),
            ) else {
                return false;
            };
            p.management != Some(true)
                && (u32::from(ip) & u32::from(mask)) == (u32::from(shared_server) & u32::from(mask))
        })
        .cloned()
        .collect::<Vec<_>>();
    let unplug_protected = |c: &Connection, pif: xenadmin_core::PifId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pifs.get(&pif).and_then(|p| p.disallow_unplug))
    };
    let mut protected = Vec::new();
    for pif in &heartbeat_pifs {
        if pif.disallow_unplug == Some(true) {
            continue;
        }
        let host_name = pif
            .host
            .and_then(|h| inventory.hosts.get(&h))
            .map(|h| h.name.clone())
            .unwrap_or_default();
        let mut entry = resource_op(
            &c,
            &format!(
                "protect the heartbeat interface {} on {host_name} from unplug",
                pif.device
            ),
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(pif.id))
                    .unwrap(),
                operation: NetworkOperation::SetUnplugProtection {
                    expected: PifIdentity::from_inventory(pif).expect("interface identity"),
                    before: false,
                    enabled: true,
                },
            },
            300,
        )
        .await;
        entry["read_back"] =
            json!(settle(120, || unplug_protected(&c, pif.id) == Some(true)).await);
        note(&mut steps, entry);
        protected.push(pif.id);
    }
    let a = administration(&c).expect("pool administration");
    note(
        &mut steps,
        json!({"step": "HA before", "enabled": a.ha_enabled, "failures_to_tolerate": a.ha_failures_to_tolerate, "heartbeat_srs": a.heartbeat_srs.len(), "heartbeat_interfaces": heartbeat_pifs.len()}),
    );
    let failures_before = a.ha_failures_to_tolerate.unwrap_or(0);
    if a.ha_enabled != Some(true) {
        let started = std::time::Instant::now();
        let mut entry = resource_op(
            &c,
            "enable HA with the shared repository as heartbeat store",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::EnableHa {
                    heartbeat_srs: vec![shared],
                    expected_failures: failures_before,
                },
            },
            900,
        )
        .await;
        entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
        entry["read_back"] = json!(
            settle(300, || administration(&c)
                .is_some_and(|a| a.ha_enabled == Some(true)))
            .await
        );
        note(&mut steps, entry);
    }
    let priority_before = priority_of(&c)
        .as_deref()
        .and_then(HaRestartPriority::parse)
        .unwrap_or(HaRestartPriority::Disabled);
    if priority_before != HaRestartPriority::Restart {
        let mut entry = resource_op(
            &c,
            "protect the tools guest with the restart priority",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::SetVmRestartPriority {
                    vm: guest.id,
                    expected: priority_before,
                    priority: HaRestartPriority::Restart,
                },
            },
            300,
        )
        .await;
        entry["read_back"] =
            json!(settle(120, || priority_of(&c).as_deref() == Some("restart")).await);
        note(&mut steps, entry);
    }
    let failures_now = administration(&c)
        .and_then(|a| a.ha_failures_to_tolerate)
        .unwrap_or(0);
    if failures_now < 1 {
        let mut entry = resource_op(
            &c,
            "tolerate one host failure",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::SetHaFailuresToTolerate {
                    expected: failures_now,
                    failures: 1,
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(
            settle(120, || administration(&c)
                .is_some_and(|a| a.ha_failures_to_tolerate == Some(1)))
            .await
        );
        entry["plan_exists_for"] = json!(administration(&c).and_then(|a| a.ha_plan_exists_for));
        note(&mut steps, entry);
    }
    let armed = steps.iter().all(|s| {
        s.get("accepted").is_none_or(|_| s["state"] == "Succeeded")
            && s.get("read_back").is_none_or(|r| *r == true)
    });
    if !armed {
        lab.record(
            "coordinator-loss-with-ha-member-takeover",
            json!({"failed": "HA preparation", "steps": steps}),
        );
        panic!("HA preparation failed: {steps:?}");
    }

    // Outage: cut power to the coordinator's domain under the live session.
    let generation_before = c.latest().generation();
    let endpoint_before = c.latest().active_endpoint().map(|e| e.to_string());
    let remembered = c
        .latest()
        .verified_members()
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    let domain = lab.hosts[0].name.clone();
    let cut_at = std::time::Instant::now();
    virsh(&["destroy", &domain]);
    let (stale_timeline, stale_seen) = state_timeline(&c, 300, |c| !c.latest().is_current()).await;
    let stale_after_ms = cut_at.elapsed().as_millis() as u64;
    let (takeover_timeline, followed) = state_timeline(&c, 1200, |c| {
        let s = c.latest();
        s.is_current() && s.inventory().is_some_and(|i| i.coordinator == Some(member))
    })
    .await;
    let followed_after_ms = cut_at.elapsed().as_millis() as u64;
    let endpoint_after = c.latest().active_endpoint().map(|e| e.to_string());
    let generation_after = c.latest().generation();
    let restarted = settle(900, || {
        placement(&c) == Some((Some(member), PowerState::Running))
    })
    .await;
    let restarted_after_ms = cut_at.elapsed().as_millis() as u64;
    note(
        &mut steps,
        json!({"step": "coordinator power cut", "stale_seen": stale_seen, "stale_after_ms": stale_after_ms,
               "followed_member_as_coordinator": followed, "followed_after_ms": followed_after_ms,
               "endpoint_before": endpoint_before, "endpoint_after": endpoint_after, "remembered_members": remembered,
               "generation_before": generation_before, "generation_after": generation_after,
               "protected_guest_restarted_on_member": restarted, "restarted_after_ms": restarted_after_ms,
               "old_coordinator_live": live(&c, coordinator),
               "old_coordinator_enabled": c.latest().inventory().map(|i| i.hosts[&coordinator].enabled),
               "timeline": {"stale": stale_timeline, "takeover": takeover_timeline}}),
    );

    // Return: the old coordinator boots and rejoins as a member of the surviving coordinator.
    // A host that has not rejoined after fifteen minutes is reset once more, and that is
    // recorded: the first attempt on this lab left XAPI down on the returning host.
    let start_at = std::time::Instant::now();
    virsh(&["start", &domain]);
    let rejoined = |c: &Connection| {
        live(c, coordinator) == Some(true)
            && c.latest()
                .inventory()
                .is_some_and(|i| i.coordinator == Some(member) && i.hosts[&coordinator].enabled)
    };
    let mut returned = settle(900, || rejoined(&c)).await;
    let mut resets = 0;
    if !returned {
        virsh(&["reset", &domain]);
        resets += 1;
        returned = settle(900, || rejoined(&c)).await;
    }
    note(
        &mut steps,
        json!({"step": "old coordinator returns as a member", "read_back": returned, "after_ms": start_at.elapsed().as_millis() as u64, "resets": resets,
               "coordinator_now": c.latest().inventory().and_then(|i| i.coordinator).and_then(|h| inventory.hosts.get(&h).map(|x| x.name.clone()))}),
    );

    // Restore: disable HA, drop the priority we added, hand the coordinator role back. A
    // connection that never became current again is recorded as such rather than unwrapped.
    let current = settle(300, || c.latest().is_current()).await;
    if !current {
        note(
            &mut steps,
            json!({"step": "restore", "skipped": "connection not current", "read_back": false, "state": format!("{:?}", c.latest().state()), "error": c.latest().error()}),
        );
        c.close().await;
        lab.record(
            "coordinator-loss-with-ha-member-takeover",
            json!({"coordinator": lab.hosts[0].name, "member": inventory.hosts[&member].name, "steps": steps}),
        );
        panic!(
            "the connection never became current again; the lab needs manual restoration: {steps:?}"
        );
    }
    let mut entry = resource_op(
        &c,
        "disable HA",
        ResourceCommand::Pool {
            target: pool_target(&c),
            operation: PoolOperation::DisableHa,
        },
        600,
    )
    .await;
    entry["read_back"] = json!(
        settle(180, || administration(&c)
            .is_some_and(|a| a.ha_enabled == Some(false)))
        .await
    );
    note(&mut steps, entry);
    if priority_before != HaRestartPriority::Restart {
        let mut entry = resource_op(
            &c,
            "remove the tools guest's restart priority",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::SetVmRestartPriority {
                    vm: guest.id,
                    expected: HaRestartPriority::Restart,
                    priority: priority_before,
                },
            },
            300,
        )
        .await;
        entry["read_back"] =
            json!(settle(120, || priority_of(&c).as_deref() != Some("restart")).await);
        note(&mut steps, entry);
    }
    let handover_at = std::time::Instant::now();
    let handover = c.submit_resource(
        OperationId::new(),
        ResourceCommand::Pool {
            target: pool_target(&c),
            operation: PoolOperation::DesignateCoordinator {
                expected: member,
                host: coordinator,
            },
        },
    );
    let mut entry = match &handover {
        Ok(operation) => {
            let row = wait_outcome(operation, 600).await;
            json!({"step": "hand the coordinator role back", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
        }
        Err(error) => {
            json!({"step": "hand the coordinator role back", "accepted": false, "error": error.to_string()})
        }
    };
    entry["read_back"] = json!(
        settle(600, || {
            let s = c.latest();
            s.state() == ConnectionState::Ready
                && s.inventory()
                    .is_some_and(|i| i.coordinator == Some(coordinator))
        })
        .await
    );
    entry["followed_after_ms"] = json!(handover_at.elapsed().as_millis() as u64);
    entry["endpoint_after"] = json!(c.latest().active_endpoint().map(|e| e.to_string()));
    // The handover restarts both toolstacks and loses the reply; once every host is enabled
    // again, the operator's recheck confirms the durable record without repeating anything.
    let hosts_enabled = settle(300, || {
        c.latest()
            .inventory()
            .is_some_and(|i| i.hosts.values().all(|h| h.enabled))
    })
    .await;
    entry["hosts_enabled_after_ms"] = json!(handover_at.elapsed().as_millis() as u64);
    if let Ok(operation) = &handover
        && entry["state"] == "OutcomeUnknown"
        && hosts_enabled
    {
        let id = operation.latest().id;
        let requested = c
            .control_resource(id, xenadmin_core::OperationControl::Recheck)
            .map_err(|e| e.to_string());
        entry["recheck_requested"] = json!(requested.is_ok());
        entry["recheck_error"] = json!(requested.err());
        settle(180, || {
            c.resource_history()
                .iter()
                .any(|r| r.id == id && r.state.terminal())
        })
        .await;
        if let Some(row) = c.resource_history().iter().find(|r| r.id == id) {
            entry["recheck_state"] = json!(format!("{:?}", row.state));
            entry["recheck_detail"] = json!(row.detail);
            if row.state == OperationState::Succeeded {
                entry["state"] = json!("Succeeded");
            }
        }
    }
    note(&mut steps, entry);

    // Restore the storage interfaces' unplug protection to the lab baseline.
    for pif in protected {
        let Some(current) = c
            .latest()
            .inventory()
            .and_then(|i| i.details.pifs.get(&pif).cloned())
        else {
            continue;
        };
        let host_name = current
            .host
            .and_then(|h| inventory.hosts.get(&h))
            .map(|h| h.name.clone())
            .unwrap_or_default();
        let mut entry = resource_op(
            &c,
            &format!(
                "remove the unplug protection of {} on {host_name}",
                current.device
            ),
            ResourceCommand::Network {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pif(pif))
                    .unwrap(),
                operation: NetworkOperation::SetUnplugProtection {
                    expected: PifIdentity::from_inventory(&current).expect("interface identity"),
                    before: true,
                    enabled: false,
                },
            },
            300,
        )
        .await;
        entry["read_back"] = json!(settle(120, || unplug_protected(&c, pif) == Some(false)).await);
        note(&mut steps, entry);
    }

    // Baseline: the tools guest back on the coordinator, the local-storage guest running.
    if placement(&c).map(|p| p.0) == Some(Some(member)) {
        // The restarted toolstacks advertise the guest's operations again a little later.
        settle(180, || {
            c.latest().inventory().is_some_and(|i| {
                i.details.vms.get(&guest.id).is_some_and(|d| {
                    d.allowed_operations
                        .as_ref()
                        .is_some_and(|ops| ops.iter().any(|op| op == "pool_migrate"))
                })
            })
        })
        .await;
        let migration = migrate(&c, coordinator).expect("migration accepted");
        let row = wait_vm_outcome(&migration, 900).await;
        let placed = settle(120, || {
            placement(&c) == Some((Some(coordinator), PowerState::Running))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "move the tools guest back onto the coordinator", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": placed}),
        );
    }
    if let Some(local) = local_guest
        && c.latest()
            .inventory()
            .is_some_and(|i| i.vms[&local.id].power_state == PowerState::Halted)
    {
        let start = c
            .submit(
                OperationId::new(),
                VmCommand::Power {
                    vm: c.latest().observe_vm(local.id).unwrap(),
                    action: PowerAction::Start,
                },
            )
            .expect("start accepted");
        let row = wait_vm_outcome(&start, 300).await;
        let running = settle(120, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&local.id].power_state == PowerState::Running)
        })
        .await;
        note(
            &mut steps,
            json!({"step": "start the local-storage guest halted by the power cut", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": running}),
        );
    }
    c.close().await;
    lab.record(
        "coordinator-loss-with-ha-member-takeover",
        json!({"coordinator": lab.hosts[0].name, "member": inventory.hosts[&member].name, "steps": steps}),
    );
    assert!(
        stale_seen,
        "the connection stayed current after the coordinator lost power"
    );
    assert!(
        followed,
        "the client did not follow the member that took over as coordinator"
    );
    assert!(
        restarted,
        "HA did not restart the protected guest on the member"
    );
    assert!(returned, "the old coordinator did not return as a member");
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Plain HTTP probe of a Xen Orchestra web endpoint from the workstation: reachable when
/// the server answers with a status line, with the page title when the body carries it.
async fn xo_probe(address: &str) -> Value {
    let address = address.to_owned();
    tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        let target = format!("{address}:80");
        let Some(socket) = std::net::ToSocketAddrs::to_socket_addrs(&target)
            .ok()
            .and_then(|mut a| a.next())
        else {
            return json!({"reachable": false, "error": "unresolvable"});
        };
        let mut stream =
            match std::net::TcpStream::connect_timeout(&socket, Duration::from_secs(5)) {
                Ok(s) => s,
                Err(e) => return json!({"reachable": false, "error": e.to_string()}),
            };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        if let Err(e) =
            stream.write_all(format!("GET / HTTP/1.0\r\nHost: {address}\r\n\r\n").as_bytes())
        {
            return json!({"reachable": false, "error": e.to_string()});
        }
        let mut body = Vec::new();
        let _ = stream.take(65536).read_to_end(&mut body);
        let text = String::from_utf8_lossy(&body);
        let status = text.lines().next().unwrap_or_default().to_owned();
        let title = text
            .split("<title>")
            .nth(1)
            .and_then(|t| t.split("</title>").next())
            .map(|t| t.trim().to_owned());
        json!({"reachable": status.starts_with("HTTP/"), "status": status, "title": title, "bytes": body.len()})
    })
    .await
    .unwrap_or_else(|e| json!({"reachable": false, "error": e.to_string()}))
}

/// The first IPv4 address the XO appliance's guest agent reports.
fn xo_address(c: &Connection, xo: xenadmin_core::VmId) -> Option<String> {
    c.latest().inventory().and_then(|i| {
        let g = i.details.vms.get(&xo)?.guest_metrics?;
        let m = i.details.guest_metrics.get(&g)?;
        m.networks
            .iter()
            .filter(|(k, _)| k.ends_with("/ip") || k.contains("/ipv4/"))
            .map(|(_, v)| v.clone())
            .find(|v| v.parse::<std::net::Ipv4Addr>().is_ok())
    })
}

/// Polls the appliance's web endpoint until it is reachable (or not, when `expect` is
/// false) within `seconds`; without a reported address, "unreachable" is immediate.
async fn xo_reachable_within(
    c: &Connection,
    xo: xenadmin_core::VmId,
    seconds: u64,
    expect: bool,
) -> Value {
    let started = std::time::Instant::now();
    let mut last = json!(null);
    let ok = tokio::time::timeout(deadline(seconds), async {
        loop {
            if let Some(a) = xo_address(c, xo) {
                last = xo_probe(&a).await;
                if last["reachable"] == expect {
                    return true;
                }
            } else if !expect {
                last = json!({"reachable": false, "error": "no address reported"});
                return true;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
    .await
    .unwrap_or(false);
    json!({"as_expected": ok, "after_ms": started.elapsed().as_millis() as u64, "address": xo_address(c, xo), "probe": last})
}

/// M2 recovery gate, P11-09, P03-01, P06-01: import the Xen Orchestra appliance image onto
/// the shared repository (reused if a VM named XOA already exists), size it for the lab host,
/// start it and confirm XO answers over HTTP from the workstation. Then shut it down so XO is
/// unreachable and, through the direct host connection only, locate the XO VM by name and
/// description, start it, open its console and confirm XO answers again. The appliance stays
/// running for later scenarios.
#[tokio::test]
#[ignore]
async fn lab_xo_vm_recovery_gate() {
    use xenadmin_core::{
        MemoryLimits, PowerAction, PowerState, TransferAction, TransferFile, TransferRequest,
        VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    let lab = Lab::load();
    let Ok(image) = std::env::var("XENADMIN_LAB_XOA_XVA") else {
        eprintln!("XENADMIN_LAB_XOA_XVA not set; skipping");
        return;
    };
    let image = std::path::PathBuf::from(image);
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let shared: xenadmin_core::SrId = inventory
        .storage
        .iter()
        .find(|s| s.name.starts_with("lab-nfs-"))
        .map(|s| s.uuid.into())
        .expect("the shared NFS repository");
    let mut steps = Vec::new();
    let find_xo = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.guests()
                .find(|v| v.name == "XOA" && !v.is_template)
                .map(|v| (v.id, v.description.clone(), v.host, v.power_state.clone()))
        })
    };
    let xo = if let Some((id, _, _, _)) = find_xo(&c) {
        note(
            &mut steps,
            json!({"step": "import the Xen Orchestra appliance", "reused": id.to_string()}),
        );
        id
    } else {
        let started = std::time::Instant::now();
        let import = c
            .transfer(
                OperationId::new(),
                c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                TransferRequest {
                    action: TransferAction::ImportXva { sr: shared },
                    file: TransferFile::source(&image).expect("inspect archive"),
                },
            )
            .expect("import accepted");
        let outcome = wait_outcome(&import, 3600).await;
        let imported = outcome.transfer_created.vms.first().copied();
        let visible = settle(120, || {
            imported.is_some_and(|id| {
                c.latest()
                    .inventory()
                    .is_some_and(|i| i.vms.get(&id).is_some_and(|v| v.name == "XOA"))
            })
        })
        .await;
        note(
            &mut steps,
            json!({"step": "import the Xen Orchestra appliance", "accepted": true, "state": format!("{:?}", outcome.state), "detail": outcome.detail,
                   "elapsed_ms": started.elapsed().as_millis() as u64, "read_back": visible,
                   "image_bytes": std::fs::metadata(&image).map(|m| m.len()).ok(), "created_vms": outcome.transfer_created.vms.len(),
                   "transfer": format!("{:?}", outcome.transfer),
                   "steps": outcome.steps.iter().map(|s| json!({"label": s.label, "state": format!("{:?}", s.state), "detail": s.detail})).collect::<Vec<_>>()}),
        );
        imported.expect("imported XO VM")
    };

    // The appliance ships with 4 GiB; the nested lab hosts have 8 GiB, so size it down while halted.
    let target: u64 = 2 * 1024 * 1024 * 1024;
    let details = c.latest().inventory().unwrap().details.vms[&xo].clone();
    if find_xo(&c).map(|x| x.3) == Some(PowerState::Halted)
        && details.memory_static_max.is_some_and(|m| m > target)
    {
        let previous = MemoryLimits {
            static_min: details.memory_static_min.unwrap_or(target),
            dynamic_min: details.memory_dynamic_min.unwrap_or(target),
            dynamic_max: details.memory_dynamic_max.unwrap_or(target),
            static_max: details.memory_static_max.unwrap_or(target),
        };
        let patch = VmSettingsPatch::new(vec![VmSettingEdit::Memory {
            previous: previous.clone(),
            value: MemoryLimits {
                static_min: previous.static_min.min(target),
                dynamic_min: target,
                dynamic_max: target,
                static_max: target,
            },
        }])
        .unwrap();
        let edit = c
            .submit(
                OperationId::new(),
                VmCommand::Configure {
                    vm: c.latest().observe_vm(xo).unwrap(),
                    patch,
                },
            )
            .expect("memory edit accepted");
        let row = wait_vm_outcome(&edit, 300).await;
        let read = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.details.vms[&xo].memory_static_max == Some(target))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "size the appliance for the lab host", "accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "read_back": read, "previous_static_max": previous.static_max, "static_max": target}),
        );
    }
    let mut power = Vec::new();

    // XO up: start the appliance if needed and wait for its web endpoint.
    if find_xo(&c).map(|x| x.3) == Some(PowerState::Halted) {
        power.push(power_to(&c, xo, PowerAction::Start, PowerState::Running).await);
    }
    let up = xo_reachable_within(&c, xo, 900, true).await;
    note(
        &mut steps,
        json!({"step": "XO answers over HTTP", "read_back": up["as_expected"], "probe": up}),
    );

    // XO unreachable: a clean shutdown through the guest agent, hard if not advertised.
    let shutdown = power_to(&c, xo, PowerAction::CleanShutdown, PowerState::Halted).await;
    let clean = shutdown["reached"] == true;
    power.push(shutdown);
    if !clean {
        power.push(power_to(&c, xo, PowerAction::HardShutdown, PowerState::Halted).await);
    }
    let down = xo_reachable_within(&c, xo, 120, false).await;
    note(
        &mut steps,
        json!({"step": "XO unreachable after the shutdown", "read_back": down["as_expected"], "clean_shutdown": clean, "probe": down}),
    );

    // The gate: with XO down, locate the XO VM through the direct connection, start it,
    // open its console and wait for XO to answer again.
    let located = find_xo(&c);
    note(
        &mut steps,
        json!({"step": "locate the XO VM with XO unreachable", "read_back": located.as_ref().is_some_and(|(_, d, _, _)| d.contains("Xen Orchestra")),
               "name": "XOA", "description": located.as_ref().map(|x| x.1.clone()), "power_state": located.as_ref().map(|x| format!("{:?}", x.3))}),
    );
    let start_at = std::time::Instant::now();
    power.push(power_to(&c, xo, PowerAction::Start, PowerState::Running).await);
    let (console_state, frame_file) = match c
        .latest()
        .observe_vm(xo)
        .map_err(|e| e.to_string())
        .and_then(|target| c.open_console(target).map_err(|e| e.to_string()))
    {
        Ok(console) => {
            let got = tokio::time::timeout(deadline(120), async {
                loop {
                    let snapshot = console.latest();
                    if snapshot.state == xenadmin_core::ConsoleState::Connected
                        && snapshot.frame.is_some()
                    {
                        return true;
                    }
                    if matches!(snapshot.state, xenadmin_core::ConsoleState::Failed(_)) {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap_or(false);
            // Give the appliance a moment to draw its boot output, then keep that frame.
            tokio::time::sleep(Duration::from_secs(20)).await;
            let digest = console
                .latest()
                .frame
                .as_ref()
                .map(|frame| frame_digest(frame))
                .unwrap_or_default();
            let saved = got
                .then(|| save_frame(&lab, &console, "xo-vm-console"))
                .flatten();
            let state = format!("{:?}", console.latest().state);
            console.close();
            (
                json!({"connected": got, "state": state, "frame_digest": digest}),
                saved,
            )
        }
        Err(error) => (json!({"connected": false, "error": error}), None),
    };
    let back = xo_reachable_within(&c, xo, 900, true).await;
    note(
        &mut steps,
        json!({"step": "start the XO VM, open its console and wait for XO", "read_back": back["as_expected"] == true && console_state["connected"] == true,
               "console": console_state, "console_frame_file": frame_file, "probe": back, "recovered_after_ms": start_at.elapsed().as_millis() as u64}),
    );
    c.close().await;
    lab.record(
        "xo-vm-recovery-gate",
        json!({"vm": xo.to_string(), "image": image.file_name().map(|f| f.to_string_lossy().into_owned()), "power": power, "steps": steps}),
    );
    for entry in power.iter().filter(|e| e["action"] == "Start") {
        assert_eq!(entry["reached"], true, "{entry}");
    }
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Polls a fixed address until the XO web endpoint answers (or stops answering, when
/// `expect` is false) within `seconds`.
async fn xo_answers_at(address: &str, seconds: u64, expect: bool) -> Value {
    let started = std::time::Instant::now();
    let mut last = json!(null);
    let ok = tokio::time::timeout(deadline(seconds), async {
        loop {
            last = xo_probe(address).await;
            if last["reachable"] == expect {
                return true;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await
    .unwrap_or(false);
    json!({"as_expected": ok, "after_ms": started.elapsed().as_millis() as u64, "address": address, "probe": last})
}

/// M2 recovery gate, P11-09, P07-03: XO made unreachable by a network fault while its VM
/// keeps running. The appliance's only NIC is disconnected, so XO stops answering although
/// the VM is Running. Through the direct host connection alone the client then locates the
/// XO VM, shows it Running with its NIC detached, opens its console, reconnects the NIC and
/// waits for XO to answer again. A reboot through the client is the fallback when the guest
/// does not take its address back; which of the two restored XO is recorded.
#[tokio::test]
#[ignore]
async fn lab_xo_network_fault_is_diagnosed_and_repaired_through_the_direct_connection() {
    use xenadmin_core::{DeviceOperation, PowerAction, PowerState};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let find_xo = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.guests()
                .find(|v| v.name == "XOA" && !v.is_template)
                .map(|v| (v.id, v.description.clone(), v.power_state.clone()))
        })
    };
    let Some((xo, _, power_state)) = find_xo(&c) else {
        eprintln!("no XOA appliance in the lab pool; run lab_xo_vm_recovery_gate first");
        c.close().await;
        return;
    };
    let mut steps = Vec::new();
    let mut power = Vec::new();
    if power_state == PowerState::Halted {
        power.push(power_to(&c, xo, PowerAction::Start, PowerState::Running).await);
    }
    let up = xo_reachable_within(&c, xo, 900, true).await;
    let address = up["address"].as_str().map(str::to_owned);
    note(
        &mut steps,
        json!({"step": "XO answers over HTTP before the fault", "read_back": up["as_expected"], "probe": up}),
    );
    let address = address.expect("the appliance reports an address while XO answers");
    let vif_view = |c: &Connection, vif: xenadmin_core::VifId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.vifs.get(&vif).cloned())
    };
    let nics: Vec<_> = c
        .latest()
        .inventory()
        .unwrap()
        .details
        .vifs
        .values()
        .filter(|v| v.vm == Some(xo))
        .cloned()
        .collect();
    assert_eq!(nics.len(), 1, "the appliance should carry exactly one NIC");
    let vif = nics[0].id;
    let hotplug = nics[0]
        .allowed_operations
        .as_deref()
        .is_some_and(|ops| ops.iter().any(|o| o == "unplug"));
    note(
        &mut steps,
        json!({"step": "the appliance's NIC", "read_back": nics[0].currently_attached == Some(true) && hotplug,
               "device": nics[0].device, "currently_attached": nics[0].currently_attached, "allowed_operations": nics[0].allowed_operations}),
    );
    assert!(hotplug, "the appliance does not advertise a NIC unplug");

    // The fault: the NIC is disconnected while the appliance runs.
    let mut fault = device_op(&c, xo, DeviceOperation::UnplugNic { vif }, 180).await;
    fault["step"] = json!("fault: disconnect the appliance's NIC while it runs");
    let detached = settle(60, || {
        vif_view(&c, vif).is_some_and(|v| v.currently_attached == Some(false))
    })
    .await;
    fault["read_back"] = json!(detached);
    note(&mut steps, fault);
    let down = xo_answers_at(&address, 120, false).await;
    note(
        &mut steps,
        json!({"step": "XO unreachable at its address with its VM running", "read_back": down["as_expected"], "probe": down}),
    );

    // Diagnosis through the direct connection only.
    let located = find_xo(&c);
    let nic = vif_view(&c, vif);
    note(
        &mut steps,
        json!({"step": "locate the XO VM and read its state with XO unreachable",
               "read_back": located.as_ref().is_some_and(|(_, d, p)| d.contains("Xen Orchestra") && *p == PowerState::Running)
                   && nic.as_ref().is_some_and(|v| v.currently_attached == Some(false))
                   && c.latest().state() == xenadmin_core::ConnectionState::Ready,
               "connection": format!("{:?}", c.latest().state()), "name": "XOA",
               "description": located.as_ref().map(|x| x.1.clone()), "power_state": located.as_ref().map(|x| format!("{:?}", x.2)),
               "nic_currently_attached": nic.as_ref().and_then(|v| v.currently_attached),
               "reported_address": xo_address(&c, xo)}),
    );
    let (console_state, frame_file) = match c
        .latest()
        .observe_vm(xo)
        .map_err(|e| e.to_string())
        .and_then(|target| c.open_console(target).map_err(|e| e.to_string()))
    {
        Ok(console) => {
            let got = tokio::time::timeout(deadline(120), async {
                loop {
                    let snapshot = console.latest();
                    if snapshot.state == xenadmin_core::ConsoleState::Connected
                        && snapshot.frame.is_some()
                    {
                        return true;
                    }
                    if matches!(snapshot.state, xenadmin_core::ConsoleState::Failed(_)) {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
            .await
            .unwrap_or(false);
            let digest = console
                .latest()
                .frame
                .as_ref()
                .map(|frame| frame_digest(frame))
                .unwrap_or_default();
            let saved = got
                .then(|| save_frame(&lab, &console, "xo-vm-console-network-fault"))
                .flatten();
            let state = format!("{:?}", console.latest().state);
            console.close();
            (
                json!({"connected": got, "state": state, "frame_digest": digest}),
                saved,
            )
        }
        Err(error) => (json!({"connected": false, "error": error}), None),
    };
    note(
        &mut steps,
        json!({"step": "open the XO VM's console with XO unreachable", "read_back": console_state["connected"],
               "console": console_state, "console_frame_file": frame_file}),
    );

    // The repair: reconnect the NIC and wait for XO. The guest may come back on another
    // address, so the reported address is preferred and the earlier one is the fallback.
    let repair_at = std::time::Instant::now();
    let mut repair = device_op(&c, xo, DeviceOperation::PlugNic { vif }, 180).await;
    repair["step"] = json!("repair: reconnect the appliance's NIC");
    let attached = settle(60, || {
        vif_view(&c, vif).is_some_and(|v| v.currently_attached == Some(true))
    })
    .await;
    repair["read_back"] = json!(attached);
    note(&mut steps, repair);
    let answers = |seconds: u64| {
        let c = &c;
        let address = address.clone();
        async move {
            let started = std::time::Instant::now();
            let mut last = json!(null);
            let mut used = address.clone();
            let ok = tokio::time::timeout(deadline(seconds), async {
                loop {
                    used = xo_address(c, xo).unwrap_or_else(|| address.clone());
                    last = xo_probe(&used).await;
                    if last["reachable"] == true {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            })
            .await
            .unwrap_or(false);
            json!({"as_expected": ok, "after_ms": started.elapsed().as_millis() as u64, "address": used, "probe": last})
        }
    };
    let mut back = answers(240).await;
    let mut restored_by = "reconnecting the NIC";
    if back["as_expected"] != true {
        note(
            &mut steps,
            json!({"step": "XO did not answer after the NIC was reconnected", "probe": back}),
        );
        power.push(power_to(&c, xo, PowerAction::Reboot, PowerState::Running).await);
        back = answers(900).await;
        restored_by = "a reboot through the client after reconnecting the NIC";
    }
    note(
        &mut steps,
        json!({"step": "XO answers over HTTP again", "read_back": back["as_expected"], "restored_by": restored_by,
               "probe": back, "recovered_after_ms": repair_at.elapsed().as_millis() as u64}),
    );
    c.close().await;
    lab.record(
        "xo-network-fault",
        json!({"vm": xo.to_string(), "power": power, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// The area in which two console frames differ, as top, left, rows and columns in pixels:
/// the whole frame when their sizes differ, none when they are the same.
fn changed_area(
    a: &xenadmin_core::ConsoleFrame,
    b: &xenadmin_core::ConsoleFrame,
) -> Option<(usize, usize, usize, usize)> {
    if (a.width, a.height) != (b.width, b.height) {
        return Some((0, 0, usize::from(b.height), usize::from(b.width)));
    }
    let width = usize::from(a.width);
    let (mut top, mut bottom, mut left, mut right) = (usize::MAX, 0, usize::MAX, 0);
    for (index, (p, q)) in a.rgba.chunks(4).zip(b.rgba.chunks(4)).enumerate() {
        if p != q {
            let (row, column) = (index / width, index % width);
            top = top.min(row);
            bottom = bottom.max(row);
            left = left.min(column);
            right = right.max(column);
        }
    }
    (top != usize::MAX).then(|| (top, left, bottom - top + 1, right - left + 1))
}

/// The character a key press drew on a text console with 8 by 16 pixel cells: the top 14
/// rows of the cell that changed between two frames, which leaves out the cursor's rows.
/// None when nothing changed or more than that cell and the cursor's next one did.
fn typed_glyph(
    before: &xenadmin_core::ConsoleFrame,
    after: &xenadmin_core::ConsoleFrame,
) -> Option<Vec<u8>> {
    let (top, left, rows, columns) = changed_area(before, after)?;
    if rows > 16 || columns > 16 {
        return None;
    }
    let (x, y, width) = (left / 8 * 8, top / 16 * 16, usize::from(after.width));
    if x + 8 > width || y + 14 > usize::from(after.height) {
        return None;
    }
    let mut glyph = Vec::with_capacity(8 * 14 * 4);
    for row in y..y + 14 {
        let start = (row * width + x) * 4;
        glyph.extend_from_slice(&after.rgba[start..start + 32]);
    }
    Some(glyph)
}

/// Opens a guest's console and waits for its first frame.
async fn open_guest_console(
    c: &Connection,
    vm: xenadmin_core::VmId,
) -> Result<xenadmin_core::Console, String> {
    let console = c
        .latest()
        .observe_vm(vm)
        .map_err(|e| e.to_string())
        .and_then(|target| c.open_console(target).map_err(|e| e.to_string()))?;
    let connected = tokio::time::timeout(deadline(120), async {
        loop {
            let snapshot = console.latest();
            if snapshot.state == xenadmin_core::ConsoleState::Connected && snapshot.frame.is_some()
            {
                return true;
            }
            if matches!(snapshot.state, xenadmin_core::ConsoleState::Failed(_)) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .unwrap_or(false);
    if connected {
        Ok(console)
    } else {
        let state = format!("{:?}", console.latest().state);
        console.close();
        Err(format!("the console did not connect: {state}"))
    }
}

/// Sends `keys` to a console and reads what they drew within three seconds: the character,
/// when exactly one appeared, and the size of the screen area that changed.
async fn typed_character(
    console: &xenadmin_core::Console,
    keys: &[xenadmin_core::ConsoleInput],
) -> (Option<Vec<u8>>, Value) {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let before = console.latest().frame.clone();
    for key in keys {
        if let Err(error) = console.input(key.clone()) {
            return (None, json!({"sent": false, "error": error.to_string()}));
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = console.latest().frame.clone();
    let (Some(before), Some(after)) = (before, after) else {
        return (None, json!({"sent": true, "error": "no frame"}));
    };
    let area = changed_area(&before, &after);
    (
        typed_glyph(&before, &after),
        json!({"sent": true, "changed_rows": area.map(|a| a.2), "changed_columns": area.map(|a| a.3)}),
    )
}

/// P04-02, P04-07, V19: a console connection lost while a modifier key is held. At the tools
/// guest's login prompt the scenario first reads the character the screen shows for a plain
/// `d` and for the same key with Shift. It then presses Shift without releasing it and
/// live-migrates the guest to the other host, which ends the console connection under the
/// held key. On a new console to the migrated guest a plain `d` must again read as the
/// lower-case character: the upper-case one means the guest still holds Shift. It repeats
/// that with a restart of the toolstack on the guest's host in place of the migration,
/// where nothing suspends the guest. When a character reads wrong, Shift is released by
/// hand and the key typed again, which shows whether a release alone cures it. The guest is
/// migrated back at the end. No password is typed and nothing is submitted at the prompt.
#[tokio::test]
#[ignore]
async fn lab_console_lost_while_a_modifier_is_held_leaves_no_stuck_key() {
    use xenadmin_core::{
        ConsoleInput, DiskPlacement, NicPlacement, PowerState, VmCommand, VmMigration,
    };
    const SHIFT: u32 = 0xffe1;
    const LETTER: u32 = 0x64;
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let Some(guest) = inventory
        .guests()
        .find(|v| v.name.starts_with("lab-tools-") && !v.is_template)
        .cloned()
    else {
        eprintln!("no tools guest on shared storage; run lab_tools_guest_on_shared_storage first");
        c.close().await;
        return;
    };
    let vm = guest.id;
    assert_eq!(
        guest.power_state,
        PowerState::Running,
        "the tools guest must be running"
    );
    let home = guest.host.expect("resident host");
    let away = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != home)
        .expect("a second pool host");
    let mut steps = Vec::new();
    let migrate = |destination: HostId| {
        let c = &c;
        async move {
            let now = c.latest().inventory().unwrap().clone();
            let disks = now.details.vm_disks[&vm]
                .iter()
                .filter_map(|b| now.details.vbds.get(b))
                .filter(|b| b.kind == "Disk")
                .filter_map(|b| {
                    let vdi = b.vdi?;
                    let sr = now.details.vdis.get(&vdi)?.sr?;
                    Some(DiskPlacement {
                        vdi,
                        expected_sr: sr,
                        destination_sr: sr,
                    })
                })
                .collect::<Vec<_>>();
            let networks = now
                .details
                .vifs
                .values()
                .filter(|v| v.vm == Some(vm))
                .filter_map(|v| {
                    let network = v.network?;
                    Some(NicPlacement {
                        vif: v.id,
                        expected_network: network,
                        destination_network: network,
                    })
                })
                .collect::<Vec<_>>();
            settle(60, || {
                c.latest().inventory().is_some_and(|i| {
                    i.details.vms.get(&vm).is_some_and(|d| {
                        d.allowed_operations
                            .as_ref()
                            .is_some_and(|ops| ops.iter().any(|op| op == "pool_migrate"))
                    })
                })
            })
            .await;
            let started = std::time::Instant::now();
            let migration = c
                .submit(
                    OperationId::new(),
                    VmCommand::Migrate {
                        vm: c.latest().observe_vm(vm).unwrap(),
                        operation: VmMigration {
                            destination,
                            expected_host: now.vms[&vm].host,
                            expected_affinity: None,
                            live: true,
                            disks,
                            networks,
                            transfer_network: None,
                        },
                    },
                )
                .expect("migration accepted");
            let row = wait_vm_outcome(&migration, 900).await;
            let placed = settle(120, || {
                c.latest().inventory().is_some_and(|i| {
                    i.vms[&vm].host == Some(destination)
                        && i.vms[&vm].power_state == PowerState::Running
                })
            })
            .await;
            json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail,
                   "read_back": placed, "elapsed_ms": started.elapsed().as_millis() as u64})
        }
    };
    let key = |keysym: u32, down: bool| ConsoleInput::Key { keysym, down };
    let plain = [key(LETTER, true), key(LETTER, false)];

    // The two reference characters on the first console. The first frames of a console
    // arrive in pieces, so one measurement with no key passes before the ones that count.
    let console = open_guest_console(&c, vm).await.expect("console");
    let painting = typed_character(&console, &[]).await.1;
    let (lower, lower_screen) = typed_character(&console, &plain).await;
    let (upper, upper_screen) = typed_character(
        &console,
        &[
            key(SHIFT, true),
            key(LETTER, true),
            key(LETTER, false),
            key(SHIFT, false),
        ],
    )
    .await;
    let told_apart = lower.is_some() && upper.is_some() && lower != upper;
    note(
        &mut steps,
        json!({"step": "the screen tells a plain letter from the same letter with Shift", "read_back": told_apart,
               "initial_painting": painting, "plain_letter": lower_screen, "letter_with_shift": upper_screen}),
    );
    let read = |glyph: &Option<Vec<u8>>| {
        if glyph.is_some() && *glyph == lower {
            "lower case"
        } else if glyph.is_some() && *glyph == upper {
            "upper case"
        } else {
            "something else"
        }
    };
    // A plain letter on a new console to the guest: lower case, or upper case when the
    // guest still holds Shift.
    let plain_letter_on_a_new_console = |label: &'static str| {
        let (c, plain, read) = (&c, plain.clone(), &read);
        async move {
            let mut opened = open_guest_console(c, vm).await;
            for _ in 0..3 {
                if opened.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
                opened = open_guest_console(c, vm).await;
            }
            match opened {
                Ok(console) => {
                    let painting = typed_character(&console, &[]).await.1;
                    let (glyph, screen) = typed_character(&console, &plain).await;
                    let seen = read(&glyph);
                    let mut released = json!(null);
                    if seen != "lower case" {
                        let _ = console.input(key(SHIFT, false));
                        let (again, screen) = typed_character(&console, &plain).await;
                        released = json!({"read": read(&again), "screen": screen});
                    }
                    (
                        Some(console),
                        json!({"step": label, "read_back": seen == "lower case", "read": seen, "screen": screen,
                               "initial_painting": painting, "after_releasing_shift_by_hand": released}),
                    )
                }
                Err(error) => (
                    None,
                    json!({"step": label, "read_back": false, "error": error}),
                ),
            }
        }
    };

    if told_apart {
        // First loss: a live migration ends the connection under the held key.
        console.input(key(SHIFT, true)).expect("Shift pressed");
        tokio::time::sleep(Duration::from_millis(500)).await;
        let held_at = std::time::Instant::now();
        let mut away_step = migrate(away).await;
        away_step["step"] = json!("live-migrate the guest with Shift held in its console");
        note(&mut steps, away_step);
        let lost = settle(60, || {
            console.latest().state != xenadmin_core::ConsoleState::Connected
        })
        .await;
        note(
            &mut steps,
            json!({"step": "the migration ended the console connection under the held key", "read_back": lost,
                   "state": format!("{:?}", console.latest().state), "within_ms": held_at.elapsed().as_millis() as u64}),
        );
        console.close();
        let (console, entry) =
            plain_letter_on_a_new_console("a plain letter on a new console after the migration")
                .await;
        note(&mut steps, entry);

        // Second loss: the toolstack of the guest's host restarts. The guest and its device
        // model keep running and nothing suspends the guest; only the console's path
        // through that host ends. A member is used so that the pool's API stays up. The
        // client refuses this restart while any task is unfinished, and an open console is
        // one, so lab tooling restarts the toolstack as another administrator would.
        let on_a_member = Some(away) != inventory.coordinator;
        if let Some(console) = console.filter(|_| on_a_member) {
            let (_, fingerprint) = pinned_options(&lab.endpoint(0)).await;
            let coordinator = lab.hosts[0].ip.clone();
            let agent = |c: &Connection| {
                c.latest().inventory().and_then(|i| {
                    i.details
                        .hosts
                        .get(&away)
                        .and_then(|h| h.agent_start_time.clone())
                })
            };
            let agent_before = agent(&c).expect("agent start time");
            let pending = other_admin(&coordinator, &fingerprint, &["pending-tasks"]).await;
            console.input(key(SHIFT, true)).expect("Shift pressed");
            tokio::time::sleep(Duration::from_millis(500)).await;
            let held_at = std::time::Instant::now();
            let restart = other_admin(
                &coordinator,
                &fingerprint,
                &["restart-toolstack", &away.to_string()],
            )
            .await;
            let lost = settle(120, || {
                console.latest().state != xenadmin_core::ConsoleState::Connected
            })
            .await;
            let lost_after_ms = held_at.elapsed().as_millis() as u64;
            let restarted = settle(300, || agent(&c).is_some_and(|now| now != agent_before)).await;
            note(
                &mut steps,
                json!({"step": "another administrator restarts the toolstack of the guest's host with Shift held in its console",
                       "read_back": restart.is_ok() && lost && restarted, "error": restart.as_ref().err(),
                       "unfinished_tasks_with_the_console_open": pending.unwrap_or_else(|e| json!({"error": e})),
                       "console_state": format!("{:?}", console.latest().state), "console_lost_after_ms": lost_after_ms,
                       "toolstack_back_after_ms": held_at.elapsed().as_millis() as u64}),
            );
            console.close();
            let (console, entry) = plain_letter_on_a_new_console(
                "a plain letter on a new console after the toolstack restart",
            )
            .await;
            note(&mut steps, entry);
            if let Some(console) = console {
                console.close();
            }
            // The restarted toolstack advertises the guest's operations again a little later.
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
        let mut back = migrate(home).await;
        back["step"] = json!("live-migrate the guest back");
        note(&mut steps, back);
    } else {
        console.close();
    }
    c.close().await;
    lab.record(
        "console-lost-while-a-modifier-is-held",
        json!({"guest": guest.name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// The lab guest with a graphical desktop, by its name prefix.
fn desktop_guest(c: &Connection) -> Option<xenadmin_core::Vm> {
    c.latest().inventory().and_then(|i| {
        i.guests()
            .find(|v| v.name.starts_with("lab-desktop-") && !v.is_template)
            .cloned()
    })
}

/// P11-09, P08-02, P03-01, P04-01: a lab guest with a graphical desktop, for pointer checks.
/// `XENADMIN_LAB_DESKTOP_IMAGE` names an isohybrid live image that starts a desktop (GParted
/// Live is what the lab uses). The image is imported as a raw disk, a guest is created from
/// the generic template with that disk alone and started, and Return is sent at intervals to
/// accept the image's boot prompts. A guest from an earlier run is reused and only started.
/// The scenario keeps the console frame it ends on; whether that frame shows a desktop is
/// for whoever reads it.
#[tokio::test]
#[ignore]
async fn lab_desktop_guest_provisioning() {
    use xenadmin_core::{
        ConsoleInput, DeviceCommand, DeviceOperation, DiskImageFormat, GuestFirmware, PowerAction,
        PowerState, TransferAction, TransferFile, TransferRequest, VmCommand, VmLifecycleOperation,
    };
    let lab = Lab::load();
    let Ok(image) = std::env::var("XENADMIN_LAB_DESKTOP_IMAGE") else {
        eprintln!("XENADMIN_LAB_DESKTOP_IMAGE not set; skipping");
        return;
    };
    let image = PathBuf::from(image);
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory.coordinator.expect("coordinator");
    let mut steps = Vec::new();
    let vm = if let Some(existing) = desktop_guest(&c) {
        note(
            &mut steps,
            json!({"step": "create the desktop guest", "reused": existing.name}),
        );
        existing.id
    } else {
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
            .expect("management network");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let disk_name = format!("lab-desktop-image-{stamp}");
        let started = std::time::Instant::now();
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
                    file: TransferFile::disk_source(&image, DiskImageFormat::Raw)
                        .expect("inspect image"),
                },
            )
            .expect("import accepted");
        let outcome = wait_outcome(&import, 1800).await;
        let find_disk = || {
            c.latest().inventory().and_then(|i| {
                i.details
                    .vdis
                    .values()
                    .find(|v| v.name == disk_name && v.sr == Some(sr))
                    .map(|v| v.id)
            })
        };
        settle(120, || find_disk().is_some()).await;
        let imported = find_disk();
        note(
            &mut steps,
            json!({"step": "import the live image as a disk", "accepted": true, "state": format!("{:?}", outcome.state),
                   "detail": outcome.detail, "read_back": imported.is_some(), "elapsed_ms": started.elapsed().as_millis() as u64}),
        );
        let imported = imported.expect("imported disk visible in inventory");
        let create = c
            .submit(
                OperationId::new(),
                VmCommand::Lifecycle {
                    source: c.latest().observe_vm(template.id).unwrap(),
                    operation: VmLifecycleOperation::Create {
                        name: format!("lab-desktop-{stamp}"),
                        description:
                            "Disposable lab guest that boots a live image into a graphical desktop"
                                .into(),
                        sr,
                        network,
                        iso: None,
                        vcpus: 1,
                        memory: 1024 * 1024 * 1024,
                        disk_size: None,
                        firmware: GuestFirmware::Bios,
                        bios_host: None,
                    },
                },
            )
            .expect("creation accepted");
        let outcome = wait_vm_outcome(&create, 300).await;
        let vm = outcome.created_vm;
        let visible = settle(60, || {
            vm.is_some_and(|vm| {
                c.latest()
                    .inventory()
                    .is_some_and(|i| i.vms.contains_key(&vm))
            })
        })
        .await;
        note(
            &mut steps,
            json!({"step": "create the desktop guest", "accepted": true, "state": format!("{:?}", outcome.state),
                   "detail": outcome.detail, "read_back": visible}),
        );
        let vm = vm.expect("created VM identity");
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
        let outcome = wait_vm_outcome(&attach, 300).await;
        let attached = settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.details.vm_disks.get(&vm).is_some_and(|d| d.len() == 1))
        })
        .await;
        note(
            &mut steps,
            json!({"step": "attach the image as the guest's disk", "accepted": true, "state": format!("{:?}", outcome.state),
                   "detail": outcome.detail, "read_back": attached}),
        );
        vm
    };

    // Start it, accept the image's boot prompts and keep the frame it settles on.
    let halted = c
        .latest()
        .inventory()
        .is_some_and(|i| i.vms[&vm].power_state == PowerState::Halted);
    let mut frame_file = None;
    if halted {
        settle(60, || {
            c.latest()
                .inventory()
                .is_some_and(|i| i.vms[&vm].allowed_actions.contains(&PowerAction::Start))
        })
        .await;
        let mut start = power_to(&c, vm, PowerAction::Start, PowerState::Running).await;
        start["step"] = json!("start the desktop guest");
        note(&mut steps, start);
        match open_guest_console(&c, vm).await {
            Ok(console) => {
                // The boot menu starts its default entry by itself; the image then asks
                // for a keymap, a language and a mode, each with a usable default.
                for wait in [75, 20, 20, 20] {
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                    let _ = console.input(ConsoleInput::KeyChord(vec![0xff0d]));
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
                let frame = console.latest().frame.clone();
                frame_file = save_frame(&lab, &console, "desktop-guest");
                note(
                    &mut steps,
                    json!({"step": "the console after the boot prompts", "read_back": frame.is_some(),
                           "width": frame.as_ref().map(|f| f.width), "height": frame.as_ref().map(|f| f.height),
                           "console_frame_file": frame_file}),
                );
                console.close();
            }
            Err(error) => note(
                &mut steps,
                json!({"step": "the console after the boot prompts", "read_back": false, "error": error}),
            ),
        }
    }
    c.close().await;
    lab.record(
        "desktop-guest-provisioning",
        json!({"vm": vm.to_string(), "image": image.file_name().map(|f| f.to_string_lossy().into_owned()),
               "console_frame_file": frame_file, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P04-02, P04-04: a pointer press in the desktop guest lands where it was aimed.
/// `XENADMIN_LAB_POINTER_TARGET` is `x,y` in the guest's framebuffer, chosen by a person
/// from a saved frame as a control that reacts to a press (a menu title, say). The pointer
/// is moved there and the left button pressed and released; the frames before and after
/// are kept, and the scenario reports the area that changed and whether it reaches the
/// target. With `XENADMIN_LAB_POINTER_KEY=escape` an Escape follows, to close a menu.
#[tokio::test]
#[ignore]
async fn lab_desktop_guest_pointer_press_lands_on_its_target() {
    use xenadmin_core::ConsoleInput;
    let lab = Lab::load();
    let Some((x, y)) = std::env::var("XENADMIN_LAB_POINTER_TARGET")
        .ok()
        .and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some((x.trim().parse::<u16>().ok()?, y.trim().parse::<u16>().ok()?))
        })
    else {
        eprintln!("XENADMIN_LAB_POINTER_TARGET=x,y not set; skipping");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let Some(guest) = desktop_guest(&c) else {
        eprintln!("no desktop guest; run lab_desktop_guest_provisioning first");
        c.close().await;
        return;
    };
    let console = open_guest_console(&c, guest.id).await.expect("console");
    // Let the first frames arrive, then park the pointer away from the target.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let pointer = |x: u16, y: u16, buttons: u8| ConsoleInput::Pointer { x, y, buttons };
    console.input(pointer(5, 5, 0)).expect("pointer moved");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let before = console.latest().frame.clone().expect("frame");
    let before_file = save_frame(&lab, &console, "desktop-pointer-before");
    console.input(pointer(x, y, 0)).expect("pointer moved");
    tokio::time::sleep(Duration::from_millis(600)).await;
    let hovered = console.latest().frame.clone().expect("frame");
    console.input(pointer(x, y, 1)).expect("button pressed");
    tokio::time::sleep(Duration::from_millis(150)).await;
    console.input(pointer(x, y, 0)).expect("button released");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = console.latest().frame.clone().expect("frame");
    let after_file = save_frame(&lab, &console, "desktop-pointer-after");
    let area = |a: &xenadmin_core::ConsoleFrame, b: &xenadmin_core::ConsoleFrame| {
        changed_area(a, b).map(|(top, left, rows, columns)| {
            json!({"top": top, "left": left, "rows": rows, "columns": columns})
        })
    };
    // The press reaches its target when what changed starts within a few pixels of it.
    let near = changed_area(&hovered, &after).is_some_and(|(top, left, rows, columns)| {
        let (x, y) = (usize::from(x), usize::from(y));
        x + 24 >= left && x <= left + columns + 24 && y + 24 >= top && y <= top + rows + 24
    });
    if std::env::var("XENADMIN_LAB_POINTER_KEY").as_deref() == Ok("escape") {
        let _ = console.input(ConsoleInput::KeyChord(vec![0xff1b]));
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let cursor = console
        .latest()
        .cursor
        .as_ref()
        .map(|c| json!({"width": c.width, "height": c.height}));
    console.close();
    c.close().await;
    lab.record(
        "desktop-guest-pointer-press",
        json!({"guest": guest.name, "target": {"x": x, "y": y}, "frame": {"width": after.width, "height": after.height},
               "changed_by_moving_there": area(&before, &hovered), "changed_by_the_press": area(&hovered, &after),
               "change_reaches_the_target": near, "guest_cursor_shape": cursor,
               "before_file": before_file, "after_file": after_file}),
    );
    assert!(near, "the press changed nothing at its target");
}

/// P04-06, P04-02: a text typed as key presses reaches a guest exactly. In the plain guest's
/// shell one request types three lines: a `read` that takes the next line as it comes, a
/// line holding every printable ASCII character, and a command that reports that line's
/// bytes in hexadecimal to the workstation over the isolated network. The report must be
/// the bytes that were typed.
#[tokio::test]
#[ignore]
async fn lab_typed_text_reaches_the_guest_exactly() {
    use xenadmin_core::{ConsoleInput, PowerState, TypedText};
    let lab = Lab::load();
    let gateway = std::env::var("XENADMIN_LAB_GATEWAY").unwrap_or_else(|_| "192.168.254.1".into());
    let port: u16 = 8000;
    let guest_ip =
        std::env::var("XENADMIN_LAB_GUEST_IP").unwrap_or_else(|_| "192.168.254.100".into());
    let reports = report_listener(&format!("{gateway}:{port}")).await;
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let guest = c
        .latest()
        .inventory()
        .unwrap()
        .guests()
        .find(|v| v.name.starts_with("lab-guest-") && v.power_state == PowerState::Running)
        .expect("running lab guest from lab_guest_provisioning")
        .clone();
    let console = open_guest_console(&c, guest.id).await.expect("console");

    // Log in and bring the interface up, as the other guest scenarios do.
    stable_frame(&console, 240).await;
    type_line(&console, "").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    stable_frame(&console, 60).await;
    type_line(&console, "root").await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    type_line(&console, "ip link set eth0 up").await;
    type_line(&console, &format!("ip addr add {guest_ip}/24 dev eth0")).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Every printable ASCII character, with the space inside so that `read` keeps it.
    let payload: String = ('!'..='O').chain([' ']).chain('P'..='~').collect();
    let expected: String = payload.bytes().map(|b| format!("{b:02x}")).collect();
    let text = format!(
        "read -r t\n{payload}\nwget -q -O- http://{gateway}:{port}/report/$(printf %s \"$t\" | hexdump -v -e '1/1 \"%02x\"')\n"
    );
    let review = TypedText::review(&text).expect("the text can be typed");
    let started = std::time::Instant::now();
    console
        .input(ConsoleInput::TypeText(text))
        .expect("typing accepted");
    let began = settle(10, || console.latest().typing > 0).await;
    let finished = settle(120, || console.latest().typing == 0).await;
    let typed_ms = started.elapsed().as_millis() as u64;
    let reported = wait_report(&reports, &expected, 90).await;
    let received = reports.received.lock().unwrap().clone();
    let frame_file = save_frame(&lab, &console, "typed-text");
    console.close();
    c.close().await;
    lab.record(
        "typed-text-reaches-the-guest",
        json!({"guest": guest.name, "characters": review.characters, "line_breaks": review.line_breaks,
               "payload_characters": payload.len(), "typing_began": began, "typing_finished": finished,
               "typed_in_ms": typed_ms, "guest_reported_the_typed_bytes": reported,
               "expected_report": expected, "received_reports": received, "console_frame_file": frame_file}),
    );
    assert!(began && finished, "the text was not typed out");
    assert!(reported, "the guest did not report the typed bytes");
}

/// P13-13: the pool's HTTPS-only setting, which XAPI keeps on each host. The client shows
/// the hosts' common value, turns it on through the reviewed pool operation, reads it back
/// from the hosts and turns it off again. A plain HTTP connection to the coordinator from
/// the workstation is tried at each stage as a check from outside the client.
#[tokio::test]
#[ignore]
async fn lab_pool_https_only_round_trip() {
    use xenadmin_core::{PoolOperation, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let pool = c.latest().inventory().unwrap().pool_id;
    let shown = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .pool
                .as_ref()
                .and_then(|p| p.administration.https_only)
        })
    };
    let plain_http = |address: String| async move {
        tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let Ok(socket) = format!("{address}:80").parse::<std::net::SocketAddr>() else {
                return json!({"error": "bad address"});
            };
            match std::net::TcpStream::connect_timeout(&socket, Duration::from_secs(5)) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = stream.write_all(b"GET / HTTP/1.0\r\n\r\n");
                    let mut reply = [0_u8; 64];
                    let read = stream.read(&mut reply).unwrap_or(0);
                    let line = String::from_utf8_lossy(&reply[..read]);
                    json!({"connected": true, "status": line.lines().next().unwrap_or_default()})
                }
                Err(error) => json!({"connected": false, "error": error.to_string()}),
            }
        })
        .await
        .unwrap_or_else(|e| json!({"error": e.to_string()}))
    };
    let coordinator = lab.hosts[0].ip.clone();
    let mut steps = Vec::new();
    let before = shown(&c);
    note(
        &mut steps,
        json!({"step": "the client shows the hosts' common setting", "read_back": before == Some(false),
               "https_only": before, "plain_http": plain_http(coordinator.clone()).await}),
    );
    if before == Some(false) {
        for (label, expected, enabled) in [
            ("turn HTTPS-only access on", false, true),
            ("turn HTTPS-only access off again", true, false),
        ] {
            let mut entry = resource_op(
                &c,
                label,
                ResourceCommand::Pool {
                    target: c
                        .latest()
                        .observe_resource(ResourceTarget::Pool(pool))
                        .unwrap(),
                    operation: PoolOperation::SetHttpsOnly { expected, enabled },
                },
                300,
            )
            .await;
            entry["read_back"] = json!(settle(60, || shown(&c) == Some(enabled)).await);
            entry["https_only"] = json!(shown(&c));
            entry["plain_http"] = plain_http(coordinator.clone()).await;
            note(&mut steps, entry);
        }
    }
    c.close().await;
    lab.record("pool-https-only-round-trip", json!({"steps": steps}));
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// The pool's security overview through a fresh inspection.
async fn security_overview(
    c: &Connection,
    pool: xenadmin_core::PoolId,
) -> Result<xenadmin_core::SecurityOverview, String> {
    use xenadmin_core::{PoolSecurityQuery, PoolSecurityResult};
    let target = c
        .latest()
        .observe_resource(ResourceTarget::Pool(pool))
        .map_err(|e| e.to_string())?;
    let discovery = c
        .inspect_pool_security(target, PoolSecurityQuery::Overview)
        .map_err(|e| e.to_string())?;
    match security_result(&discovery, 120).await? {
        PoolSecurityResult::Overview(o) => Ok(o),
        other => Err(format!("unexpected result {other:?}")),
    }
}

/// Waits for a pool security inspection to answer.
async fn security_result(
    discovery: &xenadmin_core::PoolSecurityDiscovery,
    seconds: u64,
) -> Result<xenadmin_core::PoolSecurityResult, String> {
    tokio::time::timeout(deadline(seconds), async {
        loop {
            if let Some(result) = discovery.latest() {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap_or_else(|_| Err("security inspection timed out".into()))
}

/// Lab tooling standing in for another administrator, through `lab/other_admin.py`: an
/// action the client cannot take itself in the situation under test. The script reads the
/// lab password file itself and checks the coordinator's fingerprint before sending anything.
async fn other_admin(
    coordinator: &str,
    fingerprint: &CertificateFingerprint,
    action: &[&str],
) -> Result<Value, String> {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lab/other_admin.py");
    let mut arguments = vec![coordinator.to_owned(), fingerprint.to_string()];
    arguments.extend(action.iter().map(|a| (*a).to_owned()));
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("python3")
            .arg(script)
            .args(arguments)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// P13-09, P13-10, P13-11, P10-11: point both pool hosts' management interfaces at the lab
/// domain controller for DNS, join the pool to the lab Active Directory with transient
/// administrator credentials, resolve the test user and group, grant them access, make the
/// user read-only, connect as that user and confirm a write is refused, then remove the
/// subjects, leave the directory removing the machine accounts, and restore the DNS. Leftover
/// subjects or a joined pool from an earlier attempt are cleaned up first.
///
/// P01-03, V05: the user's sessions are closed on the host twice while the user is connected,
/// with no outage. First by lab tooling acting as another administrator, while the user
/// keeps its access: the user's client must sign in again by itself and follow the pool's
/// events on the new session. Then by the removal of the user's subject through the client:
/// signing in again is refused, so the user's client must stop with the reason and stay
/// stopped.
#[tokio::test]
#[ignore]
async fn lab_directory_join_subjects_and_roles() {
    use xenadmin_core::{
        ConnectionState, Credentials, ManagementInterface, ManagementNetworkOperation,
        PoolOperation, PoolSecurityOperation, PoolSecurityQuery, PoolSecurityResult,
        PoolSensitiveChange, ResourceCommand, VmCommand, VmSettingEdit, VmSettingsPatch,
    };
    let lab = Lab::load();
    let Some(dir) = lab.directory.as_ref() else {
        eprintln!("no domain controller in the manifest; skipping");
        return;
    };
    let Some((admin_password, user_password)) = dir.passwords.clone() else {
        eprintln!("XENADMIN_LAB_DC_FILE not set; skipping");
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let mut steps = Vec::new();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let summary = |o: &xenadmin_core::SecurityOverview| {
        json!({"directory_hosts": o.directory_hosts.iter().map(|h| json!({"host": inventory.hosts.get(&h.host).map(|x| x.name.clone()), "auth_type": h.auth_type, "service_name": h.service_name})).collect::<Vec<_>>(),
               "subjects": o.subjects.iter().map(|s| json!({"name": s.name, "roles": s.roles.iter().map(|r| o.roles.iter().find(|x| x.id == *r).map(|x| x.name.clone())).collect::<Vec<_>>()})).collect::<Vec<_>>(),
               "roles": o.roles.iter().map(|r| r.name.clone()).collect::<Vec<_>>(), "rbac_available": o.rbac_available, "errors": o.errors})
    };

    // 1. DNS: each host's management interface points at the controller (restored at the end).
    let management = |c: &Connection| {
        c.latest()
            .inventory()
            .map(|i| {
                i.details
                    .pifs
                    .values()
                    .filter(|p| p.management == Some(true))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let dns_of = |c: &Connection, pif: xenadmin_core::PifId| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pifs.get(&pif).map(|p| p.dns.clone()))
    };
    let mut restore = Vec::new();
    for pif in management(&c) {
        let Some(host) = pif.host else { continue };
        let host_name = inventory.hosts[&host].name.clone();
        let interface = ManagementInterface::from_inventory(&pif).expect("management interface");
        if interface.ipv4.dns == dir.ip {
            note(
                &mut steps,
                json!({"step": format!("point {host_name} at the controller for DNS"), "skipped": "already set"}),
            );
            restore.push((host, interface));
            continue;
        }
        let mut configuration = interface.ipv4.clone();
        configuration.dns = dir.ip.clone();
        let mut entry = resource_op(
            &c,
            &format!("point {host_name} at the controller for DNS"),
            ResourceCommand::ManagementNetwork {
                host: c
                    .latest()
                    .observe_resource(ResourceTarget::Host(host))
                    .unwrap(),
                operation: Box::new(ManagementNetworkOperation::ConfigureIpv4 {
                    interface: interface.clone(),
                    configuration,
                }),
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(180, || dns_of(&c, pif.id).as_deref()
                == Some(dir.ip.as_str()))
            .await
        );
        entry["dns_before"] = json!(interface.ipv4.dns);
        note(&mut steps, entry);
        restore.push((host, interface));
    }

    // 2. Leftovers: subjects or a joined pool from an earlier attempt.
    let before = security_overview(&c, pool)
        .await
        .expect("security overview");
    note(
        &mut steps,
        json!({"step": "security before", "overview": summary(&before)}),
    );
    for subject in &before.subjects {
        let mut entry = resource_op(
            &c,
            &format!("remove leftover subject {}", subject.name),
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::Security(PoolSecurityOperation::RemoveSubject {
                    subject: subject.id,
                    expected_identifier: subject.identifier.clone(),
                    expected_roles: subject.roles.clone(),
                }),
            },
            300,
        )
        .await;
        entry["read_back"] = json!(true);
        note(&mut steps, entry);
    }
    let before = security_overview(&c, pool)
        .await
        .expect("security overview");
    if before.directory_hosts.iter().any(|h| h.auth_type == "AD") {
        let entry = resource_op(
            &c,
            "leave the directory joined by an earlier attempt",
            ResourceCommand::PoolSensitive {
                target: pool_target(&c),
                change: PoolSensitiveChange::LeaveDirectory {
                    expected_hosts: before.directory_hosts.clone(),
                    credentials: Credentials::new("Administrator".into(), admin_password.clone()),
                },
            },
            600,
        )
        .await;
        note(&mut steps, entry);
    }

    // 3. Join with transient administrator credentials.
    let before = security_overview(&c, pool)
        .await
        .expect("security overview");
    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        "join the lab Active Directory",
        ResourceCommand::PoolSensitive {
            target: pool_target(&c),
            change: PoolSensitiveChange::JoinDirectory {
                domain: dir.realm.clone(),
                expected_hosts: before.directory_hosts.clone(),
                credentials: Credentials::new("Administrator".into(), admin_password.clone()),
            },
        },
        900,
    )
    .await;
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    let joined = security_overview(&c, pool)
        .await
        .expect("security overview");
    entry["read_back"] = json!(
        !joined.directory_hosts.is_empty()
            && joined
                .directory_hosts
                .iter()
                .all(|h| h.auth_type == "AD" && h.service_name.eq_ignore_ascii_case(&dir.realm))
    );
    entry["overview"] = summary(&joined);
    let joined_ok = entry["state"] == "Succeeded" && entry["read_back"] == true;
    note(&mut steps, entry);

    let mut user_subject = None;
    if joined_ok {
        // 4. Resolve and add the user and the group, then make the user read-only.
        let read_only = joined
            .roles
            .iter()
            .find(|r| r.name == "read-only")
            .map(|r| r.id);
        let pool_admin = joined
            .roles
            .iter()
            .find(|r| r.name == "pool-admin")
            .map(|r| r.id);
        for (kind, name) in [("user", dir.user.clone()), ("group", dir.group.clone())] {
            let query = format!("{}\\{}", dir.netbios, name);
            let resolved = match c
                .inspect_pool_security(
                    pool_target(&c),
                    PoolSecurityQuery::ResolveSubject {
                        name: query.clone(),
                    },
                )
                .map_err(|e| e.to_string())
            {
                Ok(d) => security_result(&d, 120).await,
                Err(e) => Err(e),
            };
            let (identifier, resolved_name) = match &resolved {
                Ok(PoolSecurityResult::Subject {
                    identifier, name, ..
                }) => (Some(identifier.clone()), Some(name.clone())),
                _ => (None, None),
            };
            note(
                &mut steps,
                json!({"step": format!("resolve the {kind} {query}"), "read_back": identifier.is_some(), "name": resolved_name, "identifier": identifier, "error": resolved.as_ref().err()}),
            );
            let Some(identifier) = identifier else {
                continue;
            };
            let mut entry = resource_op(
                &c,
                &format!("grant the {kind} access"),
                ResourceCommand::Pool {
                    target: pool_target(&c),
                    operation: PoolOperation::Security(PoolSecurityOperation::AddSubject {
                        identifier: identifier.clone(),
                    }),
                },
                300,
            )
            .await;
            let now = security_overview(&c, pool)
                .await
                .expect("security overview");
            let subject = now
                .subjects
                .iter()
                .find(|s| s.identifier == identifier)
                .cloned();
            entry["read_back"] = json!(subject.is_some());
            entry["roles_after_creation"] = json!(subject.as_ref().map(|s| {
                s.roles
                    .iter()
                    .map(|r| {
                        now.roles
                            .iter()
                            .find(|x| x.id == *r)
                            .map(|x| x.name.clone())
                    })
                    .collect::<Vec<_>>()
            }));
            note(&mut steps, entry);
            if kind == "user" {
                user_subject = subject;
            }
        }
        if let (Some(subject), Some(read_only), Some(pool_admin)) =
            (user_subject.clone(), read_only, pool_admin)
        {
            let mut roles = subject.roles.clone();
            for (label, role, grant) in [
                ("grant the user the read-only role", read_only, true),
                ("remove the user's pool-admin role", pool_admin, false),
            ] {
                if grant == roles.contains(&role) {
                    note(
                        &mut steps,
                        json!({"step": label, "skipped": "already in that state"}),
                    );
                    continue;
                }
                let mut entry = resource_op(
                    &c,
                    label,
                    ResourceCommand::Pool {
                        target: pool_target(&c),
                        operation: PoolOperation::Security(
                            PoolSecurityOperation::ChangeSubjectRole {
                                subject: subject.id,
                                expected_identifier: subject.identifier.clone(),
                                expected_roles: roles.clone(),
                                role,
                                grant,
                            },
                        ),
                    },
                    300,
                )
                .await;
                let now = security_overview(&c, pool)
                    .await
                    .expect("security overview");
                let after = now
                    .subjects
                    .iter()
                    .find(|s| s.id == subject.id)
                    .map(|s| s.roles.clone())
                    .unwrap_or_default();
                entry["read_back"] = json!(after.contains(&role) == grant);
                entry["roles_after"] = json!(
                    after
                        .iter()
                        .map(|r| now
                            .roles
                            .iter()
                            .find(|x| x.id == *r)
                            .map(|x| x.name.clone()))
                        .collect::<Vec<_>>()
                );
                note(&mut steps, entry);
                roles = after;
            }
        }

        // 5. Connect as the directory user from a second backend (one authenticated owner per
        // pool and backend): Ready with read-only permissions, a write refused.
        let (mut options, fingerprint) = pinned_options(&lab.endpoint(0)).await;
        options.request_timeout = Duration::from_secs(60);
        let user_login = format!("{}\\{}", dir.netbios, dir.user);
        let user_state = tempfile::tempdir().unwrap();
        let user_backend = Backend::with_state_directory(user_state.path().into()).unwrap();
        let as_user = user_backend.connect(
            options,
            Credentials::new(user_login.clone(), user_password.clone()),
        );
        let ready = as_user
            .wait_ready(deadline(180))
            .await
            .map_err(|e| e.to_string());
        let snapshot = as_user.latest();
        let state = format!("{:?}", snapshot.state());
        let error = snapshot.error().map(str::to_owned);
        let permissions = {
            let p = snapshot.session_permissions();
            json!({"roles": p.roles(), "permission_count": p.permission_count()})
        };
        let guest = snapshot.inventory().and_then(|i| {
            i.guests()
                .find(|v| v.name.starts_with("lab-guest-"))
                .map(|v| (v.id, v.description.clone()))
        });
        drop(snapshot);
        // The read-only session's write must not reach the host: refused at submission or
        // rejected by the client's permission preflight before any request.
        let refused = if let Some((id, description)) = guest.clone() {
            match as_user.latest().observe_vm(id) {
                Ok(vm) => match as_user.submit(
                    OperationId::new(),
                    VmCommand::Configure {
                        vm,
                        patch: VmSettingsPatch::new(vec![VmSettingEdit::Description {
                            previous: description.clone(),
                            value: format!("{description} (must not apply)"),
                        }])
                        .unwrap(),
                    },
                ) {
                    Ok(op) => {
                        let row = wait_vm_outcome(&op, 120).await;
                        let unchanged = settle(30, || {
                            as_user.latest().inventory().is_some_and(|i| {
                                i.vms.get(&id).is_some_and(|v| v.description == description)
                            })
                        })
                        .await;
                        json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail, "refused": row.state != OperationState::Succeeded && unchanged})
                    }
                    Err(e) => json!({"accepted": false, "error": e.to_string(), "refused": true}),
                },
                Err(e) => json!({"accepted": false, "error": e.to_string(), "refused": true}),
            }
        } else {
            json!({"skipped": "no lab guest", "refused": false})
        };
        note(
            &mut steps,
            json!({"step": "connect as the directory user", "read_back": ready.is_ok() && state == "Ready" && refused["refused"] == true,
                   "login": user_login, "state": state, "error": error, "permissions": permissions, "write_attempt": refused}),
        );

        // 5b. A session lost with no outage: lab tooling, as another administrator, closes
        // the user's sessions while the user keeps its access. The client must sign in again
        // by itself and follow the pool's events on the new session.
        let connected_subject = user_subject.clone().filter(|_| ready.is_ok());
        if let Some(subject) = &connected_subject {
            let before = as_user.latest().generation();
            let logout = other_admin(
                &lab.hosts[0].ip,
                &fingerprint,
                &["logout-subject", &subject.identifier],
            )
            .await;
            let (timeline, recovered) = state_timeline(&as_user, 180, |x| {
                let s = x.latest();
                s.state() == ConnectionState::Ready && s.generation() > before
            })
            .await;
            let permissions = {
                let snapshot = as_user.latest();
                let p = snapshot.session_permissions();
                json!({"roles": p.roles(), "permission_count": p.permission_count()})
            };
            // Events on the new session: the administrator edits the guest's description and
            // restores it, and the user's connection must show each value.
            let mut followed = Vec::new();
            if let (true, Some((id, description))) = (recovered, guest.clone()) {
                let marker = format!("{description} (session check)");
                for (previous, value) in [
                    (description.clone(), marker.clone()),
                    (marker, description.clone()),
                ] {
                    let edit = c
                        .submit(
                            OperationId::new(),
                            VmCommand::Configure {
                                vm: c.latest().observe_vm(id).unwrap(),
                                patch: VmSettingsPatch::new(vec![VmSettingEdit::Description {
                                    previous,
                                    value: value.clone(),
                                }])
                                .unwrap(),
                            },
                        )
                        .expect("description edit accepted");
                    let row = wait_vm_outcome(&edit, 120).await;
                    let applied = settle(60, || {
                        c.latest()
                            .inventory()
                            .is_some_and(|i| i.vms.get(&id).is_some_and(|v| v.description == value))
                    })
                    .await;
                    let seen = settle(60, || {
                        as_user
                            .latest()
                            .inventory()
                            .is_some_and(|i| i.vms.get(&id).is_some_and(|v| v.description == value))
                    })
                    .await;
                    followed.push(json!({"state": format!("{:?}", row.state), "applied": applied, "seen_by_the_user": seen}));
                }
            }
            let follows = followed.len() == 2
                && followed
                    .iter()
                    .all(|f| f["state"] == "Succeeded" && f["seen_by_the_user"] == true);
            note(
                &mut steps,
                json!({"step": "the user's sessions are closed by another administrator; the user keeps its access",
                       "read_back": logout.is_ok() && recovered && follows, "logout": logout.as_ref().err(),
                       "generation_before": before, "generation_after": as_user.latest().generation(),
                       "timeline": timeline, "permissions": permissions, "events_on_the_new_session": followed}),
            );
        }

        // 6. Remove the subjects and leave the directory, removing the machine accounts. The
        // connected user's subject goes last, so that no group still gives the user access
        // when its removal closes its sessions.
        let now = security_overview(&c, pool)
            .await
            .expect("security overview");
        let mut subjects = now.subjects.clone();
        subjects.sort_by_key(|s| connected_subject.as_ref().is_some_and(|u| u.id == s.id));
        let generation_before = as_user.latest().generation();
        for subject in &subjects {
            let mut entry = resource_op(
                &c,
                &format!("remove the subject {}", subject.name),
                ResourceCommand::Pool {
                    target: pool_target(&c),
                    operation: PoolOperation::Security(PoolSecurityOperation::RemoveSubject {
                        subject: subject.id,
                        expected_identifier: subject.identifier.clone(),
                        expected_roles: subject.roles.clone(),
                    }),
                },
                300,
            )
            .await;
            let after = security_overview(&c, pool)
                .await
                .expect("security overview");
            entry["read_back"] = json!(!after.subjects.iter().any(|s| s.id == subject.id));
            note(&mut steps, entry);
        }

        // 6b. The same loss once the user's access is gone. Removing the subject through the
        // client closes the user's sessions and signing in again is refused, so the user's
        // client must stop with the reason, and stay stopped.
        if let Some(subject) = &connected_subject {
            let stops = |x: &Connection| {
                matches!(
                    x.latest().state(),
                    ConnectionState::Failed | ConnectionState::Disconnected
                )
            };
            let (mut timeline, mut stopped) = state_timeline(&as_user, 60, stops).await;
            let closed_by_the_removal = stopped;
            let mut tooling = json!(null);
            if !stopped {
                // The removal left the session open. Close it as another administrator
                // would, so that the refused sign-in is still observed.
                let logout = other_admin(
                    &lab.hosts[0].ip,
                    &fingerprint,
                    &["logout-subject", &subject.identifier],
                )
                .await;
                tooling = json!({"closed": logout.is_ok(), "error": logout.err()});
                let (more, later) = state_timeline(&as_user, 180, stops).await;
                timeline.extend(more);
                stopped = later;
            }
            let (later, returned) = state_timeline(&as_user, 30, |x| {
                x.latest().state() == ConnectionState::Ready
            })
            .await;
            let snapshot = as_user.latest();
            note(
                &mut steps,
                json!({"step": "the user's access is removed while the user is connected",
                       "read_back": closed_by_the_removal && stopped && !returned && snapshot.error().is_some(),
                       "sessions_closed_by_the_removal": closed_by_the_removal, "lab_tooling_logout": tooling,
                       "generation_before": generation_before, "generation": snapshot.generation(),
                       "state": format!("{:?}", snapshot.state()), "error": snapshot.error(),
                       "timeline": timeline, "next_30_s": later}),
            );
        }
        as_user.close().await;
        let now = security_overview(&c, pool)
            .await
            .expect("security overview");
        let started = std::time::Instant::now();
        let mut entry = resource_op(
            &c,
            "leave the directory and remove the machine accounts",
            ResourceCommand::PoolSensitive {
                target: pool_target(&c),
                change: PoolSensitiveChange::LeaveDirectory {
                    expected_hosts: now.directory_hosts.clone(),
                    credentials: Credentials::new("Administrator".into(), admin_password.clone()),
                },
            },
            900,
        )
        .await;
        entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
        let left = security_overview(&c, pool)
            .await
            .expect("security overview");
        entry["read_back"] = json!(left.directory_hosts.iter().all(|h| h.auth_type.is_empty()));
        entry["overview"] = summary(&left);
        note(&mut steps, entry);
    }

    // 7. Restore the hosts' DNS.
    for (host, interface) in restore {
        if interface.ipv4.dns == dir.ip {
            continue;
        }
        let host_name = inventory.hosts[&host].name.clone();
        let current = management(&c)
            .into_iter()
            .find(|p| p.id == interface.pif)
            .and_then(|p| ManagementInterface::from_inventory(&p));
        let Some(current) = current else { continue };
        let mut entry = resource_op(
            &c,
            &format!("restore {host_name}'s DNS"),
            ResourceCommand::ManagementNetwork {
                host: c
                    .latest()
                    .observe_resource(ResourceTarget::Host(host))
                    .unwrap(),
                operation: Box::new(ManagementNetworkOperation::ConfigureIpv4 {
                    interface: current,
                    configuration: interface.ipv4.clone(),
                }),
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(180, || dns_of(&c, interface.pif)
                == Some(interface.ipv4.dns.clone()))
            .await
        );
        note(&mut steps, entry);
    }
    c.close().await;
    lab.record(
        "directory-join-subjects-and-roles",
        json!({"controller": dir.ip, "realm": dir.realm, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Lab repair, P12-14 and P09-09: after a coordinator that cannot rejoin, recover through
/// the surviving coordinator: disable HA, delete the records of guests whose disks live on
/// the dead host's local storage (those disks are unreachable), forget the dead host once its
/// domain is verified off at the hypervisor, and forget its local repositories. The lab
/// tooling then reinstalls that host and the join scenario brings it back.
#[tokio::test]
#[ignore]
async fn lab_forget_dead_coordinator_through_survivor() {
    use std::collections::BTreeSet;
    use xenadmin_core::{
        HostOperation, PoolOperation, ResourceCommand, StorageOperation, VmCommand,
        VmLifecycleOperation,
    };
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 1, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let survivor = inventory.coordinator.expect("coordinator");
    assert_eq!(
        inventory.hosts[&survivor].address, lab.hosts[1].ip,
        "the second manifest host must be the surviving coordinator"
    );
    let Some(dead) = inventory
        .hosts
        .iter()
        .find(|(_, h)| h.address == lab.hosts[0].ip)
        .map(|(id, _)| *id)
    else {
        eprintln!("no record of the first manifest host remains; nothing to forget");
        return;
    };
    let dead_name = inventory.hosts[&dead].name.clone();
    let mut steps = Vec::new();
    let pool_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Pool(pool))
            .unwrap()
    };
    let administration = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.pool.as_ref().map(|p| p.administration.clone()))
    };
    let live = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.details
                .hosts
                .get(&dead)
                .and_then(|h| h.metrics)
                .and_then(|m| i.details.host_metrics.get(&m))
                .and_then(|m| m.live)
        })
    };
    note(
        &mut steps,
        json!({"step": "before", "dead": dead_name, "live": live(&c), "enabled": inventory.hosts[&dead].enabled, "ha_enabled": administration(&c).and_then(|a| a.ha_enabled)}),
    );
    if administration(&c).and_then(|a| a.ha_enabled) == Some(true) {
        let mut entry = resource_op(
            &c,
            "disable HA",
            ResourceCommand::Pool {
                target: pool_target(&c),
                operation: PoolOperation::DisableHa,
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(180, || administration(&c)
                .is_some_and(|a| a.ha_enabled == Some(false)))
            .await
        );
        note(&mut steps, entry);
    }
    // The dead host's domain is verified off at the hypervisor; it is reinstalled afterwards.
    let domain = lab.hosts[0].name.clone();
    if virsh(&["domstate", &domain]) != "shut off" {
        virsh(&["destroy", &domain]);
    }
    let off = settle(60, || virsh(&["domstate", &domain]) == "shut off").await;
    note(
        &mut steps,
        json!({"step": "dead coordinator powered off at the hypervisor", "read_back": off}),
    );
    // Guests whose disks live on the dead host's local repositories: their records only.
    let dead_srs = inventory
        .details
        .pbds
        .values()
        .filter(|p| p.host == Some(dead))
        .filter_map(|p| p.sr)
        .filter(|sr| inventory.details.storage.get(sr).and_then(|d| d.shared) != Some(true))
        .collect::<BTreeSet<_>>();
    let on_dead = |vm: xenadmin_core::VmId| {
        inventory.details.vm_disks.get(&vm).is_some_and(|vbds| {
            vbds.iter()
                .filter_map(|b| inventory.details.vbds.get(b))
                .filter(|b| b.kind == "Disk")
                .filter_map(|b| b.vdi)
                .filter_map(|v| inventory.details.vdis.get(&v))
                .filter_map(|d| d.sr)
                .any(|sr| dead_srs.contains(&sr))
        })
    };
    for vm in inventory
        .vms
        .values()
        .filter(|v| !v.is_control_domain && on_dead(v.id))
        .cloned()
        .collect::<Vec<_>>()
    {
        let snapshots = inventory
            .snapshots
            .for_vm(vm.id)
            .map(|s| s.id)
            .collect::<Vec<_>>();
        let mut entry = match c.submit(
            OperationId::new(),
            VmCommand::Lifecycle {
                source: c.latest().observe_vm(vm.id).unwrap(),
                operation: VmLifecycleOperation::Delete {
                    expected_template: vm.is_template,
                    disks: Vec::new(),
                    snapshots: snapshots
                        .iter()
                        .map(|id| xenadmin_core::SnapshotDeletionSelection {
                            snapshot: *id,
                            disks: Vec::new(),
                        })
                        .collect(),
                },
            },
        ) {
            Ok(op) => {
                let row = wait_vm_outcome(&op, 300).await;
                json!({"accepted": true, "state": format!("{:?}", row.state), "detail": row.detail})
            }
            Err(e) => json!({"accepted": false, "error": e.to_string()}),
        };
        entry["step"] = json!(format!(
            "delete the record of {} whose disks are on the dead host",
            vm.name
        ));
        entry["snapshots"] = json!(snapshots.len());
        entry["read_back"] = json!(
            settle(120, || c
                .latest()
                .inventory()
                .is_some_and(|i| !i.vms.contains_key(&vm.id)))
            .await
        );
        note(&mut steps, entry);
    }
    let mut entry = resource_op(
        &c,
        &format!("forget the dead coordinator {dead_name}"),
        ResourceCommand::Host {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Host(dead))
                .unwrap(),
            operation: HostOperation::ForgetDeadHost {
                verified_power_off: off,
            },
        },
        600,
    )
    .await;
    entry["read_back"] = json!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(|i| !i.hosts.contains_key(&dead)))
        .await
    );
    note(&mut steps, entry);
    for sr in dead_srs {
        let Some(now) = c.latest().inventory().cloned() else {
            break;
        };
        let Some(summary) = now
            .storage
            .iter()
            .find(|s| xenadmin_core::SrId::from(s.uuid) == sr)
            .cloned()
        else {
            continue;
        };
        let attached = now
            .details
            .pbds
            .values()
            .filter(|p| p.sr == Some(sr))
            .count();
        let disks = now.details.sr_disks.get(&sr).cloned().unwrap_or_default();
        // XAPI recomputes a repository's advertised operations lazily after its host is
        // destroyed; give it a moment before the reviewed forget (observation 38).
        let advertised = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .storage
                    .get(&sr)
                    .and_then(|d| d.allowed_operations.as_ref())
                    .is_some_and(|ops| ops.iter().any(|o| o == "forget"))
            })
        })
        .await;
        let mut entry = resource_op(
            &c,
            &format!("forget the dead host's repository {}", summary.name),
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Storage(sr))
                    .unwrap(),
                operation: StorageOperation::Forget {
                    expected_name: summary.name.clone(),
                    expected_disks: disks.clone(),
                },
            },
            300,
        )
        .await;
        entry["attachments_left"] = json!(attached);
        entry["disks"] = json!(disks.len());
        entry["forget_advertised"] = json!(advertised);
        entry["read_back"] = json!(
            settle(120, || c.latest().inventory().is_some_and(|i| {
                !i.storage
                    .iter()
                    .any(|s| xenadmin_core::SrId::from(s.uuid) == sr)
            }))
            .await
        );
        note(&mut steps, entry);
    }
    c.close().await;
    lab.record(
        "forget-dead-coordinator-through-survivor",
        json!({"dead": dead_name, "survivor": inventory.hosts[&survivor].name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Lab utility, P12-11: hand the coordinator role to the first manifest host when another
/// host holds it after a recovery, through the reviewed handover, and follow it.
#[tokio::test]
#[ignore]
async fn lab_designate_first_host_coordinator() {
    use xenadmin_core::{ConnectionState, PoolOperation, ResourceCommand};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let current = inventory.coordinator.expect("coordinator");
    let first = inventory
        .hosts
        .iter()
        .find(|(_, h)| h.address == lab.hosts[0].ip)
        .map(|(id, _)| *id)
        .expect("the first manifest host is in the pool");
    let mut steps = Vec::new();
    if current == first {
        note(
            &mut steps,
            json!({"step": "hand the coordinator role to the first manifest host", "skipped": "already the coordinator"}),
        );
    } else {
        let started = std::time::Instant::now();
        let mut entry = resource_op(
            &c,
            "hand the coordinator role to the first manifest host",
            ResourceCommand::Pool {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .unwrap(),
                operation: PoolOperation::DesignateCoordinator {
                    expected: current,
                    host: first,
                },
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            settle(600, || {
                let s = c.latest();
                s.state() == ConnectionState::Ready
                    && s.inventory().is_some_and(|i| i.coordinator == Some(first))
            })
            .await
        );
        entry["followed_after_ms"] = json!(started.elapsed().as_millis() as u64);
        entry["endpoint_after"] = json!(c.latest().active_endpoint().map(|e| e.to_string()));
        // The handover restarts both toolstacks and can close the connection before the
        // reply; the operator path rechecks the durable record once the pool is visible.
        if entry["state"] == "OutcomeUnknown" {
            let id = c
                .resource_history()
                .iter()
                .find(|r| r.state == OperationState::OutcomeUnknown)
                .map(|r| r.id);
            if let Some(id) = id
                && c.control_resource(id, xenadmin_core::OperationControl::Recheck)
                    .is_ok()
            {
                settle(300, || {
                    c.resource_history()
                        .iter()
                        .any(|r| r.id == id && r.state.terminal())
                })
                .await;
                if let Some(row) = c.resource_history().iter().find(|r| r.id == id) {
                    entry["recheck_state"] = json!(format!("{:?}", row.state));
                    entry["recheck_detail"] = json!(row.detail);
                    if row.state == OperationState::Succeeded {
                        entry["state"] = json!("Succeeded");
                    }
                }
            }
        }
        note(&mut steps, entry);
    }
    c.close().await;
    lab.record("designate-first-host-coordinator", json!({"steps": steps}));
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Lab utility, P09-06: attach every shared block or NFS repository that a host has left
/// detached (for example after a host rejoined the pool), waiting for the host to advertise
/// plug first.
#[tokio::test]
#[ignore]
async fn lab_reattach_shared_repositories() {
    use xenadmin_core::{ResourceCommand, StorageOperation};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let mut steps = Vec::new();
    let detached = inventory
        .details
        .pbds
        .values()
        .filter(|p| p.currently_attached == Some(false))
        .filter_map(|p| {
            let sr = p.sr?;
            let kind = inventory
                .storage
                .iter()
                .find(|s| xenadmin_core::SrId::from(s.uuid) == sr)
                .map(|s| s.kind.clone())?;
            (matches!(kind.as_str(), "nfs" | "lvmoiscsi" | "lvmohba")
                && inventory.details.storage.get(&sr).and_then(|d| d.shared) == Some(true))
            .then_some((p.id, p.host?, sr, kind))
        })
        .collect::<Vec<_>>();
    for (pbd, host, sr, kind) in detached {
        let name = inventory
            .storage
            .iter()
            .find(|s| xenadmin_core::SrId::from(s.uuid) == sr)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let advertised = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.details
                    .storage
                    .get(&sr)
                    .and_then(|d| d.allowed_operations.as_ref())
                    .is_some_and(|ops| ops.iter().any(|o| o == "plug"))
            })
        })
        .await;
        let mut entry = resource_op(
            &c,
            &format!("attach {name} ({kind}) on {}", inventory.hosts[&host].name),
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pbd(pbd))
                    .unwrap(),
                operation: StorageOperation::SetAttached {
                    sr,
                    host,
                    expected_attached: false,
                    attached: true,
                },
            },
            600,
        )
        .await;
        entry["plug_advertised"] = json!(advertised);
        entry["read_back"] = json!(
            entry["state"] == "Succeeded"
                && settle(180, || c.latest().inventory().is_some_and(|i| {
                    i.details
                        .pbds
                        .get(&pbd)
                        .is_some_and(|p| p.currently_attached == Some(true))
                }))
                .await
        );
        note(&mut steps, entry);
    }
    c.close().await;
    lab.record("reattach-shared-repositories", json!({"steps": steps}));
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// Lab utility: cleanly shut down every running guest whose disks live on a shared
/// repository, before the appliance that serves them is restarted; `lab_start_lab_guests`
/// starts the lab guests again afterwards.
#[tokio::test]
#[ignore]
async fn lab_shut_down_shared_storage_guests() {
    use std::collections::BTreeSet;
    use xenadmin_core::{PowerAction, PowerState};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let shared = inventory
        .details
        .storage
        .iter()
        .filter(|(_, d)| d.shared == Some(true))
        .map(|(id, _)| *id)
        .collect::<BTreeSet<_>>();
    let on_shared = |vm: xenadmin_core::VmId| {
        inventory.details.vm_disks.get(&vm).is_some_and(|vbds| {
            vbds.iter()
                .filter_map(|b| inventory.details.vbds.get(b))
                .filter(|b| b.kind == "Disk")
                .filter_map(|b| b.vdi)
                .filter_map(|v| inventory.details.vdis.get(&v))
                .filter_map(|d| d.sr)
                .any(|sr| shared.contains(&sr))
        })
    };
    // `XENADMIN_LAB_SHUTDOWN_ALL=1` includes guests on local storage, before a host power cycle.
    let all = std::env::var("XENADMIN_LAB_SHUTDOWN_ALL").is_ok_and(|v| v == "1");
    let mut power = Vec::new();
    for vm in inventory
        .guests()
        .filter(|v| v.power_state == PowerState::Running && (all || on_shared(v.id)))
        .cloned()
        .collect::<Vec<_>>()
    {
        let mut entry = power_to(&c, vm.id, PowerAction::CleanShutdown, PowerState::Halted).await;
        if entry["reached"] != true {
            entry = power_to(&c, vm.id, PowerAction::HardShutdown, PowerState::Halted).await;
        }
        entry["vm"] = json!(vm.name);
        note(&mut power, entry);
    }
    c.close().await;
    lab.record("shut-down-shared-storage-guests", json!({"power": power}));
    for entry in &power {
        assert_eq!(entry["reached"], true, "{entry}");
    }
}

/// Lab utility: start every halted lab guest (the provisioned guest, the tools guest and the
/// Xen Orchestra appliance).
#[tokio::test]
#[ignore]
async fn lab_start_lab_guests() {
    use xenadmin_core::{PowerAction, PowerState};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let mut power = Vec::new();
    for vm in inventory
        .guests()
        .filter(|v| {
            v.power_state == PowerState::Halted
                && (v.name.starts_with("lab-guest-")
                    || v.name.starts_with("lab-tools-")
                    || v.name == "XOA")
        })
        .cloned()
        .collect::<Vec<_>>()
    {
        let mut entry = power_to(&c, vm.id, PowerAction::Start, PowerState::Running).await;
        entry["vm"] = json!(vm.name);
        note(&mut power, entry);
    }
    c.close().await;
    lab.record("start-lab-guests", json!({"power": power}));
    for entry in &power {
        assert_eq!(entry["reached"], true, "{entry}");
    }
}

/// Lab utility, P11-01: live-migrate the running Xen Orchestra appliance, whose disk is on
/// shared storage, to the coordinator, so that the member holds no guest for the scenarios
/// that put it into maintenance; `XENADMIN_LAB_XO_HOST=member` moves it back to the member.
/// An appliance already on the wanted host is only recorded.
#[tokio::test]
#[ignore]
async fn lab_move_xo_appliance() {
    use xenadmin_core::{DiskPlacement, NicPlacement, PowerState, VmCommand, VmMigration};
    let lab = Lab::load();
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory.coordinator.expect("coordinator");
    let to_member = std::env::var("XENADMIN_LAB_XO_HOST").is_ok_and(|v| v == "member");
    let destination = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| (*h != coordinator) == to_member && inventory.hosts[h].enabled)
        .expect("an enabled destination host");
    let appliance = inventory
        .guests()
        .find(|v| v.name == "XOA" && v.power_state == PowerState::Running)
        .cloned()
        .expect("the running Xen Orchestra appliance");
    let vm = appliance.id;
    let mut steps = Vec::new();
    if appliance.host != Some(destination) {
        let disks = inventory.details.vm_disks[&vm]
            .iter()
            .filter_map(|b| inventory.details.vbds.get(b))
            .filter(|b| b.kind == "Disk")
            .filter_map(|b| {
                let vdi = b.vdi?;
                let sr = inventory.details.vdis.get(&vdi)?.sr?;
                Some(DiskPlacement {
                    vdi,
                    expected_sr: sr,
                    destination_sr: sr,
                })
            })
            .collect::<Vec<_>>();
        let networks = inventory
            .details
            .vifs
            .values()
            .filter(|v| v.vm == Some(vm))
            .filter_map(|v| {
                let network = v.network?;
                Some(NicPlacement {
                    vif: v.id,
                    expected_network: network,
                    destination_network: network,
                })
            })
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let migration = c
            .submit(
                OperationId::new(),
                VmCommand::Migrate {
                    vm: c.latest().observe_vm(vm).unwrap(),
                    operation: VmMigration {
                        destination,
                        expected_host: appliance.host,
                        expected_affinity: inventory.details.vms.get(&vm).and_then(|d| d.affinity),
                        live: true,
                        disks,
                        networks,
                        transfer_network: None,
                    },
                },
            )
            .expect("migration accepted");
        let row = wait_vm_outcome(&migration, 900).await;
        let placed = settle(120, || {
            c.latest().inventory().is_some_and(|i| {
                i.vms.get(&vm).is_some_and(|v| {
                    v.power_state == PowerState::Running && v.host == Some(destination)
                })
            })
        })
        .await;
        note(
            &mut steps,
            json!({"step": "live-migrate the appliance without moving storage", "accepted": true,
                "state": format!("{:?}", row.state), "detail": row.detail, "read_back": placed,
                "elapsed_ms": started.elapsed().as_millis() as u64}),
        );
    }
    c.close().await;
    lab.record(
        "move-xo-appliance",
        json!({"from": appliance.host.map(|h| inventory.hosts[&h].name.clone()),
            "to": inventory.hosts[&destination].name, "steps": steps}),
    );
    for step in &steps {
        assert_eq!(step["state"], "Succeeded", "{step}");
        assert_eq!(step["read_back"], true, "{step}");
    }
}

/// P09-04, P09-06, P09-08, P09-09: create an ISO library over NFS on the lab appliance's
/// `/srv/nfs/iso` export through the coordinator, confirm it is listed as a shared ISO
/// repository attached on both hosts, rescan it and read back its image list (the images
/// the appliance seeds must be listed), then detach it everywhere, coordinator last, and
/// forget it. A library left by an earlier attempt is forgotten first.
#[tokio::test]
#[ignore]
async fn lab_iso_library_over_nfs() {
    iso_library_round_trip("nfs").await;
}

/// P09-04, P09-06, P09-08, P09-09: the same round trip over the appliance's guest-readable
/// SMB 3 share `//APPLIANCE/iso`, created through the explicit SMB 3.0 path without
/// credentials.
#[tokio::test]
#[ignore]
async fn lab_iso_library_over_smb() {
    iso_library_round_trip("smb").await;
}

async fn iso_library_round_trip(transport: &str) {
    use xenadmin_core::{NfsVersion, ResourceCommand, StorageLocation, StorageOperation};
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("storage network in the manifest");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let pool = inventory.pool_id;
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut steps = Vec::new();
    let leftovers = inventory
        .storage
        .iter()
        .filter(|s| s.name.starts_with("lab-iso-"))
        .map(|s| (xenadmin_core::SrId::from(s.uuid), s.name.clone()))
        .collect::<Vec<_>>();
    for (sr, leftover) in leftovers {
        note(
            &mut steps,
            json!({"step": "forget a library left by an earlier attempt", "repository": leftover}),
        );
        remove_repository(&c, &inventory, sr, &leftover, false, &mut steps).await;
    }
    let name = format!("lab-iso-{transport}-{stamp}");
    let find_sr = |c: &Connection| {
        c.latest().inventory().and_then(|i| {
            i.storage
                .iter()
                .find(|s| s.name == name)
                .map(|s| xenadmin_core::SrId::from(s.uuid))
        })
    };
    let location = if transport == "smb" {
        StorageLocation::IsoSmb {
            server: storage.nas_ip.clone(),
            share: "iso".into(),
            path: String::new(),
        }
    } else {
        StorageLocation::IsoNfs {
            server: storage.nas_ip.clone(),
            path: "/srv/nfs/iso".into(),
            version: NfsVersion::Automatic,
        }
    };
    let started = std::time::Instant::now();
    let mut entry = resource_op(
        &c,
        &format!("create the ISO library over {}", transport.to_uppercase()),
        ResourceCommand::Storage {
            target: c
                .latest()
                .observe_resource(ResourceTarget::Pool(pool))
                .unwrap(),
            operation: StorageOperation::Create {
                host: coordinator,
                name: name.clone(),
                description: format!("ISO library on the lab appliance over {transport}"),
                location,
                hba_device: None,
                local_device: None,
            },
        },
        600,
    )
    .await;
    entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
    entry["read_back"] = json!(settle(180, || find_sr(&c).is_some()).await);
    note(&mut steps, entry);
    let Some(sr) = find_sr(&c) else {
        lab.record(
            &format!("iso-library-over-{transport}"),
            json!({"appliance": storage.nas_ip, "failed": "library not listed", "steps": steps}),
        );
        panic!("library not listed: {steps:?}");
    };
    let library = |c: &Connection| {
        c.latest().inventory().map(|i| {
            let d = i.details.storage.get(&sr);
            json!({
                "kind": i.storage.iter().find(|s| xenadmin_core::SrId::from(s.uuid) == sr).map(|s| s.kind.clone()),
                "content_type": d.map(|d| d.content_type.clone()),
                "shared": d.and_then(|d| d.shared),
                "attachments": i.details.pbds.values().filter(|p| p.sr == Some(sr)).map(|p| json!({"host": p.host.and_then(|h| i.hosts.get(&h)).map(|h| h.name.clone()), "attached": p.currently_attached})).collect::<Vec<_>>(),
                "images": i.details.sr_disks.get(&sr).map_or(0, Vec::len),
                "image_names": i.details.sr_disks.get(&sr).map(|v| v.iter().filter_map(|d| i.details.vdis.get(d).map(|x| x.name.clone())).collect::<Vec<_>>()),
            })
        })
    };
    let listed = settle(240, || {
        c.latest().inventory().is_some_and(|i| {
            i.details
                .storage
                .get(&sr)
                .is_some_and(|d| d.shared == Some(true) && d.content_type == "iso")
                && {
                    let a = i
                        .details
                        .pbds
                        .values()
                        .filter(|p| p.sr == Some(sr))
                        .collect::<Vec<_>>();
                    a.len() == i.hosts.len() && a.iter().all(|p| p.currently_attached == Some(true))
                }
        })
    })
    .await;
    note(
        &mut steps,
        json!({"step": "shared ISO library attached on every host", "read_back": listed, "library": library(&c)}),
    );
    let mut entry = resource_op(
        &c,
        "rescan the library",
        ResourceCommand::ScanStorage {
            sr: c
                .latest()
                .observe_resource(ResourceTarget::Storage(sr))
                .unwrap(),
        },
        300,
    )
    .await;
    let seeded = settle(120, || {
        c.latest().inventory().is_some_and(|i| {
            let names = i
                .details
                .sr_disks
                .get(&sr)
                .map(|v| {
                    v.iter()
                        .filter_map(|d| i.details.vdis.get(d).map(|x| x.name.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            storage
                .seed_isos
                .iter()
                .all(|seed| names.iter().any(|n| n == seed))
        })
    })
    .await;
    entry["library"] = json!(library(&c));
    entry["read_back"] = json!(seeded);
    entry["expected_images"] = json!(storage.seed_isos);
    note(&mut steps, entry);
    remove_repository(&c, &inventory, sr, &name, false, &mut steps).await;
    c.close().await;
    lab.record(
        &format!("iso-library-over-{transport}"),
        json!({"appliance": storage.nas_ip, "repository": name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}

/// P09-12, P12-03: with the member empty and in maintenance, enable standard multipathing on
/// it through the reviewed complete-host flow (detach every attachment, set the mode and the
/// dmp handler, replug), confirm the host reports multipathing with the dmp handler and that
/// the iSCSI attachment reports its paths across the appliance's two portals, then disable
/// multipathing the same way and leave maintenance.
#[tokio::test]
#[ignore]
async fn lab_member_multipath_round_trip() {
    use xenadmin_core::{
        HostOperation, ResourceCommand, ReviewedPbd, StorageGraph, StorageOperation,
    };
    let lab = Lab::load();
    let storage = lab
        .storage
        .as_ref()
        .expect("storage network in the manifest");
    let iscsi = storage
        .iscsi
        .as_ref()
        .expect("iSCSI targets in the manifest");
    let state = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(state.path().into()).unwrap();
    let c = connect(&lab, &backend, 0, false).await;
    let inventory = c.latest().inventory().unwrap().clone();
    let coordinator = inventory
        .details
        .pool
        .as_ref()
        .and_then(|p| p.administration.coordinator)
        .expect("coordinator");
    let member = inventory
        .hosts
        .keys()
        .copied()
        .find(|h| *h != coordinator)
        .expect("member");
    let member_name = inventory.hosts[&member].name.clone();
    assert!(
        !inventory
            .vms
            .values()
            .any(|v| v.host == Some(member) && !v.is_control_domain),
        "the member must not host guests"
    );
    let mut steps = Vec::new();
    let host_target = |c: &Connection| {
        c.latest()
            .observe_resource(ResourceTarget::Host(member))
            .unwrap()
    };
    let details = |c: &Connection| {
        c.latest()
            .inventory()
            .and_then(|i| i.details.hosts.get(&member).cloned())
    };
    let iscsi_paths = |c: &Connection| {
        c.latest().inventory().map(|i| {
            i.details
                .pbds
                .values()
                .filter(|p| p.host == Some(member))
                .filter(|p| p.sr.is_some_and(|sr| i.details.storage.contains_key(&sr)))
                .filter_map(|p| {
                    let sr = i.storage.iter().find(|s| Some(xenadmin_core::SrId::from(s.uuid)) == p.sr)?;
                    (sr.kind == "lvmoiscsi").then(|| json!({"repository": sr.name, "multipathed": p.multipathed, "paths": p.paths.iter().map(|x| json!({"device": x.device, "active": x.active, "maximum": x.maximum})).collect::<Vec<_>>()}))
                })
                .collect::<Vec<_>>()
        })
    };
    note(
        &mut steps,
        json!({"step": "multipath before", "multipathing": details(&c).and_then(|h| h.multipathing), "standard_handler": details(&c).and_then(|h| h.standard_multipath_handler), "portals": [storage.nas_ip.clone(), storage.nas_lab_ip.clone()], "port": iscsi.port, "iscsi_attachments": iscsi_paths(&c)}),
    );

    // Recover from an earlier attempt: reattach anything left detached on the member before
    // the round trip, which expects every member attachment attached.
    let detached = c
        .latest()
        .inventory()
        .map(|i| {
            i.details
                .pbds
                .values()
                .filter(|p| p.host == Some(member) && p.currently_attached == Some(false))
                .filter_map(|p| Some((p.id, p.sr?)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (pbd, sr) in detached {
        let mut entry = resource_op(
            &c,
            &format!("reattach {sr} left detached by an earlier attempt"),
            ResourceCommand::Storage {
                target: c
                    .latest()
                    .observe_resource(ResourceTarget::Pbd(pbd))
                    .unwrap(),
                operation: StorageOperation::SetAttached {
                    sr,
                    host: member,
                    expected_attached: false,
                    attached: true,
                },
            },
            600,
        )
        .await;
        entry["read_back"] = json!(
            entry["state"] == "Succeeded"
                && settle(120, || c.latest().inventory().is_some_and(|i| {
                    i.details
                        .pbds
                        .get(&pbd)
                        .is_some_and(|p| p.currently_attached == Some(true))
                }))
                .await
        );
        note(&mut steps, entry);
    }
    if details(&c).and_then(|h| h.maintenance_mode) == Some(true) {
        let entry = resource_op(
            &c,
            "clear the leftover maintenance marker",
            ResourceCommand::Host {
                target: host_target(&c),
                operation: HostOperation::ExitMaintenance,
            },
            300,
        )
        .await;
        note(&mut steps, entry);
    }
    let enabled = c.latest().inventory().unwrap().hosts[&member].enabled;
    let mut entry = resource_op(
        &c,
        "enter maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::EnterMaintenance {
                expected_enabled: enabled,
                resident_vms: Vec::new(),
            },
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(60, || details(&c).and_then(|h| h.maintenance_mode)
            == Some(true))
        .await
    );
    note(&mut steps, entry);

    // If an earlier attempt left multipathing enabled, disable it first so the round trip
    // starts from the host's normal state.
    let mut toggles = Vec::new();
    if details(&c).and_then(|h| h.multipathing) == Some(true) {
        toggles.push((
            "disable multipathing left enabled by an earlier attempt",
            false,
        ));
    }
    toggles.push(("enable standard multipathing on the member", true));
    toggles.push(("disable multipathing on the member", false));
    for (label, enable) in toggles {
        let now = c.latest().inventory().unwrap().clone();
        let attachments = now
            .details
            .pbds
            .values()
            .filter(|p| p.host == Some(member))
            .filter_map(|p| ReviewedPbd::from_inventory(&now, p.id))
            .collect::<Vec<_>>();
        let host_details = now.details.hosts[&member].clone();
        let started = std::time::Instant::now();
        let mut entry = resource_op(
            &c,
            label,
            ResourceCommand::StorageGraph {
                target: host_target(&c),
                operation: StorageGraph::Multipath {
                    host: member,
                    expected: host_details.multipathing.unwrap_or(false),
                    expected_handler: host_details.standard_multipath_handler.unwrap_or(false),
                    enabled: enable,
                    attachments: attachments.clone(),
                },
            },
            1200,
        )
        .await;
        entry["elapsed_ms"] = json!(started.elapsed().as_millis() as u64);
        entry["attachments_reviewed"] = json!(attachments.len());
        entry["kept_attached"] = json!(
            attachments
                .iter()
                .filter(|a| a.local_cache)
                .map(|a| a.sr.to_string())
                .collect::<Vec<_>>()
        );
        entry["read_back"] = json!(
            settle(300, || {
                details(&c).is_some_and(|h| h.multipathing == Some(enable))
                    && c.latest().inventory().is_some_and(|i| {
                        i.details
                            .pbds
                            .values()
                            .filter(|p| p.host == Some(member))
                            .all(|p| p.currently_attached == Some(true))
                    })
            })
            .await
        );
        entry["multipathing_after"] = json!(details(&c).and_then(|h| h.multipathing));
        entry["standard_handler_after"] =
            json!(details(&c).and_then(|h| h.standard_multipath_handler));
        tokio::time::sleep(Duration::from_secs(20)).await;
        entry["iscsi_attachments_after"] = json!(iscsi_paths(&c));
        note(&mut steps, entry);
    }

    let mut entry = resource_op(
        &c,
        "exit maintenance",
        ResourceCommand::Host {
            target: host_target(&c),
            operation: HostOperation::ExitMaintenance,
        },
        300,
    )
    .await;
    entry["read_back"] = json!(
        settle(120, || c
            .latest()
            .inventory()
            .is_some_and(|i| i.hosts[&member].enabled))
        .await
    );
    note(&mut steps, entry);
    c.close().await;
    lab.record(
        "member-multipath-round-trip",
        json!({"member": member_name, "steps": steps}),
    );
    for step in &steps {
        if step.get("accepted").is_some() {
            assert_eq!(step["accepted"], true, "{step}");
            assert_eq!(step["state"], "Succeeded", "{step}");
        }
        if step.get("read_back").is_some() {
            assert_eq!(step["read_back"], true, "{step}");
        }
    }
}
