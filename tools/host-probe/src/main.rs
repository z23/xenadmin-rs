use std::{path::PathBuf, time::Duration};

use clap::Parser;
use serde_json::{Value, json};
use xenadmin_core::{
    ActionAvailability, Backend, CertificateFingerprint, Connection, ConnectionOptions,
    Credentials, Endpoint, InspectionClass, InspectionRequest, Inventory, PowerAction, ProfileId,
    VmId,
};

#[derive(Parser)]
#[command(about = "Read-only direct XCP-ng XAPI inventory probe. Never calls Xen Orchestra.")]
struct Args {
    /// XCP-ng host URL. HTTPS is required except numeric loopback addresses for tests.
    #[arg(required_unless_present = "live", conflicts_with = "live")]
    endpoint: Option<String>,
    /// Inspect an opted-in running desktop through its private Unix socket; no host login.
    #[arg(long)]
    live: Option<PathBuf>,
    /// Live connection UUID, as listed by --live SOCKET.
    #[arg(long, requires = "live", value_parser = parse_profile)]
    connection: Option<ProfileId>,
    /// List VM inventory from the running desktop.
    #[arg(long, requires = "live", conflicts_with_all = ["history", "diagnose_vm"])]
    inventory: bool,
    /// Live inventory class: vms, hosts, storage, networks, snapshots, disks, interfaces, tasks, messages.
    #[arg(long, requires = "inventory", default_value = "vms", value_parser = parse_class)]
    class: InspectionClass,
    /// Restrict live inventory to one object UUID.
    #[arg(long, requires = "inventory", value_parser = VmId::parse)]
    object: Option<VmId>,
    /// List operation summaries from the running desktop.
    #[arg(long, requires = "live", conflicts_with_all = ["inventory", "diagnose_vm"])]
    history: bool,
    /// Filter live inventory or history to a VM UUID.
    #[arg(long, requires = "live", value_parser = VmId::parse, conflicts_with = "diagnose_vm")]
    vm: Option<VmId>,
    #[arg(long, requires = "live", default_value_t = 0)]
    offset: usize,
    #[arg(long, requires = "live", default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: u16,
    #[arg(short, long, default_value = "root", conflicts_with = "live")]
    username: String,
    /// PEM CA certificate used to verify this host.
    #[arg(long, conflicts_with = "live")]
    ca_cert: Option<PathBuf>,
    /// Verified SHA-256 fingerprint of this endpoint's certificate.
    #[arg(long, conflicts_with = "live")]
    cert_sha256: Option<CertificateFingerprint>,
    /// Disable certificate validation for the initial host, coordinator redirects and consoles.
    #[arg(long, conflicts_with = "live")]
    insecure: bool,
    /// Additional trusted host certificate, as https://host=SHA256. May be repeated.
    #[arg(long, conflicts_with = "live")]
    host_cert: Vec<String>,
    /// Read the password from stdin instead of a terminal prompt.
    #[arg(long, conflicts_with = "live")]
    password_stdin: bool,
    #[arg(long)]
    json: bool,
    /// Read one guest VM's advertised and freshly checked power-action availability.
    #[arg(long, value_parser = VmId::parse, conflicts_with = "watch")]
    diagnose_vm: Option<VmId>,
    /// Observe inventory updates for this many seconds after initial synchronization.
    #[arg(long, default_value_t = 0)]
    watch: u64,
}

fn parse_profile(value: &str) -> Result<ProfileId, String> {
    serde_json::from_value(json!(value)).map_err(|_| "Expected a connection UUID".into())
}
fn parse_class(value: &str) -> Result<InspectionClass, String> {
    serde_json::from_value(json!(value)).map_err(|_| "Unknown inspection class".into())
}

async fn run_live(path: &std::path::Path, args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let request = if let Some(vm) = args.diagnose_vm {
        InspectionRequest::DiagnosePower {
            connection: args.connection,
            vm,
        }
    } else if args.history {
        InspectionRequest::History {
            connection: args.connection,
            vm: args.vm,
            limit: usize::from(args.limit),
        }
    } else if args.inventory || args.vm.is_some() {
        InspectionRequest::Inventory {
            connection: args.connection,
            vm: args.vm,
            class: args.class,
            object: args.object.map(VmId::uuid),
            offset: args.offset,
            limit: usize::from(args.limit),
        }
    } else {
        InspectionRequest::Status
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.watch);
    loop {
        let result = xenadmin_core::inspect_live(path, &request).await?;
        if args.json {
            println!("{}", serde_json::to_string(&result)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        if result.get("complete") == Some(&Value::Bool(false)) {
            return Err("Some action checks were unavailable; see the diagnostic report".into());
        }
        if args.watch == 0 || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(
            Duration::from_secs(1)
                .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
        )
        .await;
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if let Some(path) = &args.live {
        return run_live(path, &args).await;
    }
    let mut options = ConnectionOptions::new(Endpoint::parse(
        args.endpoint.as_deref().ok_or("Host endpoint required")?,
    )?);
    options.read_only = true;
    options.ca_certificate = args.ca_cert;
    options.certificate_fingerprint = args.cert_sha256;
    options.accept_invalid_certificates = args.insecure;
    if args.insecure {
        eprintln!(
            "Certificate validation is disabled for this connection and coordinator redirects"
        );
    }
    if !args.insecure {
        options.add_host_certificates(&args.host_cert.join("\n"))?;
    }
    let password = if args.password_stdin {
        let mut password = String::new();
        std::io::stdin().read_line(&mut password)?;
        password.truncate(password.trim_end_matches(['\r', '\n']).len());
        password
    } else {
        rpassword::prompt_password("XCP-ng password: ")?
    };
    let backend = Backend::start()?;
    let connection = backend.connect(options, Credentials::new(args.username, password));
    let result = async {
        let inventory = connection.wait_ready(Duration::from_secs(60)).await?;
        if let Some(vm) = args.diagnose_vm {
            let report = diagnose_vm(&connection, vm).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_diagnostic(&report);
            }
            return if report["complete"] == true {
                Ok(())
            } else {
                Err("Some action checks were unavailable; see the diagnostic report".into())
            };
        }
        if args.json {
            println!("{}", serde_json::to_string_pretty(&*inventory)?);
        } else {
            println!("Pool: {} ({})", inventory.pool_name, inventory.pool_id);
            println!(
                "{} hosts, {} guests, {} storage repositories, {} networks",
                inventory.hosts.len(),
                inventory.guests().count(),
                inventory.storage.len(),
                inventory.networks.len()
            );
            for vm in inventory.guests() {
                println!("{}  {:<12}  {}", vm.id, vm.power_state, vm.name);
            }
        }
        if args.watch > 0 {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(args.watch);
            let mut receiver = connection.watch();
            while tokio::time::timeout_at(deadline, receiver.changed())
                .await
                .is_ok_and(|r| r.is_ok())
            {
                let snapshot = receiver.borrow_and_update();
                if args.json {
                    if snapshot.is_current() {
                        println!(
                            "{}",
                            serde_json::to_string(snapshot.inventory().map(|i| &**i).unwrap())?
                        );
                    }
                } else {
                    println!(
                        "State: {:?}, generation {}",
                        snapshot.state(),
                        snapshot.generation()
                    );
                }
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    connection.close().await;
    result
}

fn vm_report(inventory: &Inventory, vm: VmId) -> Result<Value, Box<dyn std::error::Error>> {
    let vm = inventory
        .guests()
        .find(|record| record.id == vm)
        .ok_or("The requested guest VM was not found in current inventory")?;
    let details = inventory.details.vms.get(&vm.id);
    Ok(json!({
        "schema_version": 1,
        "vm": {"uuid": vm.id, "name": vm.name, "power_state": vm.power_state.to_string()},
        "observed": {
            "allowed_operations": details.and_then(|record| record.allowed_operations.as_ref()),
            "current_operations": details.map(|record| record.current_operations.iter()
                .map(|operation| operation.operation.as_str()).collect::<Vec<_>>()),
            "blocked_operations": details.map(|record| &record.blocked_operations),
        },
    }))
}

async fn diagnose_vm(
    connection: &Connection,
    vm: VmId,
) -> Result<Value, Box<dyn std::error::Error>> {
    let snapshot = connection.latest();
    let target = snapshot.observe_vm(vm)?;
    let mut report = vm_report(
        snapshot
            .inventory()
            .ok_or("Current inventory is unavailable")?,
        vm,
    )?;
    let checks = PowerAction::ALL
        .into_iter()
        .map(|action| {
            connection
                .diagnose_power(target, action)
                .map(|check| (action, check))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if connection.latest().observe_vm(vm).ok() != Some(target) {
            return Err(
                "Connection or VM identity changed during diagnostics; reconnect and retry".into(),
            );
        }
        if checks.iter().all(|(_, check)| check.latest().is_some())
            || tokio::time::Instant::now() >= deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let results = checks
        .into_iter()
        .map(|(action, check)| {
            (
                action,
                check
                    .latest()
                    .unwrap_or_else(|| Err("Action check timed out".into())),
            )
        })
        .collect::<Vec<_>>();
    add_results(&mut report, results);
    Ok(report)
}

fn add_results(
    report: &mut Value,
    results: Vec<(PowerAction, Result<ActionAvailability, String>)>,
) {
    report["complete"] = json!(results.iter().all(|(_, result)| {
        matches!(result, Ok(availability) if !matches!(availability,
            ActionAvailability::Stale | ActionAvailability::Unknown))
    }));
    report["power_actions"] = json!(
        results
            .into_iter()
            .map(|(action, result)| {
                match result {
                    Ok(availability) => json!({"action": action, "label": action.label(),
                "explanation": availability.explanation(), "availability": availability}),
                    Err(error) => {
                        json!({"action": action, "label": action.label(), "error": error})
                    }
                }
            })
            .collect::<Vec<_>>()
    );
}

fn print_diagnostic(report: &Value) {
    fn text(value: &Value) -> String {
        value
            .as_str()
            .unwrap_or("not provided")
            .escape_debug()
            .to_string()
    }
    println!(
        "VM: {} ({})",
        text(&report["vm"]["name"]),
        text(&report["vm"]["uuid"])
    );
    println!("Power state: {}", text(&report["vm"]["power_state"]));
    println!("Advertised inventory values at the start of this check:");
    for (label, key) in [
        ("Allowed operations", "allowed_operations"),
        ("Current operations", "current_operations"),
        ("Blocked operations", "blocked_operations"),
    ] {
        println!("  {label}: {}", report["observed"][key]);
    }
    println!("Fresh read-only checks (advisory; no power actions submitted):");
    for action in report["power_actions"].as_array().into_iter().flatten() {
        let explanation = action.get("explanation").or_else(|| action.get("error"));
        println!(
            "  {}: {}",
            text(&action["label"]),
            text(explanation.unwrap_or(&Value::Null))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VM: &str = "00000000-0000-0000-0000-000000000002";

    #[test]
    fn diagnostic_arguments_require_a_uuid_and_disallow_watch() {
        assert!(
            Args::try_parse_from(["host-probe", "https://host.example", "--diagnose-vm", VM])
                .is_ok()
        );
        assert!(
            Args::try_parse_from([
                "host-probe",
                "https://host.example",
                "--diagnose-vm",
                "guest-name"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "host-probe",
                "https://host.example",
                "--diagnose-vm",
                VM,
                "--watch",
                "1"
            ])
            .is_err()
        );
    }

    #[test]
    fn live_mode_needs_no_endpoint_and_refuses_login_options() {
        let args = Args::try_parse_from([
            "host-probe",
            "--live",
            "/tmp/fixture.sock",
            "--diagnose-vm",
            VM,
        ])
        .unwrap();
        assert!(args.endpoint.is_none());
        assert!(args.live.is_some());
        for flag in ["--password-stdin", "--insecure"] {
            assert!(
                Args::try_parse_from(["host-probe", "--live", "/tmp/fixture.sock", flag]).is_err()
            );
        }
        assert!(
            Args::try_parse_from([
                "host-probe",
                "https://host.example",
                "--live",
                "/tmp/fixture.sock"
            ])
            .is_err()
        );
        assert!(
            Args::try_parse_from([
                "host-probe",
                "--live",
                "/tmp/fixture.sock",
                "--inventory",
                "--limit",
                "101"
            ])
            .is_err()
        );
    }

    #[test]
    fn targeted_report_excludes_unrelated_inventory_and_task_references() {
        let inventory: Inventory = serde_json::from_value(json!({
            "pool_id": "00000000-0000-0000-0000-000000000001", "pool_name": "Private pool",
            "coordinator": null, "hosts": {}, "storage": [], "networks": [], "revision": 1,
            "vms": {
                VM: {"id": VM, "name": "Diagnostic guest", "description": "Private description",
                    "power_state": "Running", "host": null, "vcpus": 2, "memory_bytes": 1024,
                    "is_template": false, "is_control_domain": false},
                "00000000-0000-0000-0000-000000000003": {
                    "id": "00000000-0000-0000-0000-000000000003", "name": "Unrelated guest",
                    "description": "", "power_state": "Halted", "host": null, "vcpus": 1,
                    "memory_bytes": 1024, "is_template": false, "is_control_domain": false}
            },
        }))
        .unwrap();
        let mut inventory = inventory;
        let vm = VmId::parse(VM).unwrap();
        inventory.details.vms.insert(
            vm,
            xenadmin_core::VmDetails {
                allowed_operations: Some(vec!["snapshot".into()]),
                current_operations: vec![xenadmin_core::RunningOperation {
                    task: Some(
                        xenadmin_core::TaskId::parse("00000000-0000-0000-0000-000000000004")
                            .unwrap(),
                    ),
                    operation: "snapshot".into(),
                }],
                blocked_operations: [("clean_shutdown".into(), "Do not stop during backup".into())]
                    .into(),
                ..Default::default()
            },
        );
        let report = vm_report(&inventory, vm).unwrap();
        assert_eq!(
            report["observed"]["current_operations"],
            json!(["snapshot"])
        );
        assert_eq!(
            report["observed"]["blocked_operations"]["clean_shutdown"],
            "Do not stop during backup"
        );
        let output = report.to_string();
        for excluded in [
            "Private pool",
            "Private description",
            "Unrelated guest",
            "00000000-0000-0000-0000-000000000004",
        ] {
            assert!(!output.contains(excluded));
        }
        assert!(
            vm_report(
                &inventory,
                VmId::parse("00000000-0000-0000-0000-000000000005").unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn blocked_actions_are_diagnosed_but_missing_results_are_incomplete() {
        let mut report = json!({});
        add_results(
            &mut report,
            vec![(
                PowerAction::Pause,
                Ok(ActionAvailability::Blocked {
                    reason: "Maintenance in progress".into(),
                }),
            )],
        );
        assert_eq!(report["complete"], true);
        assert_eq!(
            report["power_actions"][0]["availability"]["Blocked"]["reason"],
            "Maintenance in progress"
        );
        for result in [
            Ok(ActionAvailability::Unknown),
            Ok(ActionAvailability::Stale),
            Err("Action check timed out".into()),
        ] {
            add_results(&mut report, vec![(PowerAction::Pause, result)]);
            assert_eq!(report["complete"], false);
        }
    }
}
