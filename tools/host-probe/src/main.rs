use std::{path::PathBuf, time::Duration};

use clap::Parser;
use xenadmin_core::{Backend, CertificateFingerprint, ConnectionOptions, Credentials, Endpoint};

#[derive(Parser)]
#[command(about = "Read-only direct XCP-ng XAPI inventory probe. Never calls Xen Orchestra.")]
struct Args {
    /// XCP-ng host URL. HTTPS is required except numeric loopback addresses for tests.
    endpoint: String,
    #[arg(short, long, default_value = "root")]
    username: String,
    /// PEM CA certificate used to verify this host.
    #[arg(long)]
    ca_cert: Option<PathBuf>,
    /// Verified SHA-256 fingerprint of this endpoint's certificate.
    #[arg(long)]
    cert_sha256: Option<CertificateFingerprint>,
    /// Disable certificate validation for the initial host, coordinator redirects and consoles.
    #[arg(long)]
    insecure: bool,
    /// Additional trusted host certificate, as https://host=SHA256. May be repeated.
    #[arg(long)]
    host_cert: Vec<String>,
    /// Read the password from stdin instead of a terminal prompt.
    #[arg(long)]
    password_stdin: bool,
    #[arg(long)]
    json: bool,
    /// Observe inventory updates for this many seconds after initial synchronization.
    #[arg(long, default_value_t = 0)]
    watch: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut options = ConnectionOptions::new(Endpoint::parse(&args.endpoint)?);
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
