use super::*;
use std::{
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Executable {
    pub path: PathBuf,
    identity: Identity,
}
impl Executable {
    pub fn inspect(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err(fail("Choose an absolute trusted tool path"));
        }
        let path = path
            .canonicalize()
            .map_err(|_| fail("Required local tool is not installed"))?;
        let m = path
            .metadata()
            .map_err(|_| fail("Cannot inspect the local tool"))?;
        if !m.is_file() || m.mode() & 0o022 != 0 || m.mode() & 0o111 == 0 {
            return Err(fail(
                "Local tools must be executable regular files without group/world write permission",
            ));
        }
        Ok(Self {
            path,
            identity: Identity::from(&m),
        })
    }
    fn verify(&self) -> Result<()> {
        let m = self
            .path
            .metadata()
            .map_err(|_| fail("Local tool disappeared"))?;
        if Identity::from(&m) != self.identity {
            return Err(fail("Local tool changed; inspect capabilities again"));
        }
        Ok(())
    }
}

pub(super) struct Access<'a> {
    pub input: Option<&'a Path>,
    pub output: Option<&'a Path>,
    pub write_output: bool,
    pub file_limit: u64,
    pub deadline: Duration,
}
pub(super) fn run(
    tools: &Tools,
    args: &[String],
    access: Access<'_>,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    stopped(cancel)?;
    for tool in [&tools.qemu, &tools.bwrap, &tools.prlimit] {
        tool.verify()?;
    }
    let mut command = Command::new(&tools.prlimit.path);
    command
        .args(["--as=2147483648", "--cpu=14400", "--nofile=64", "--core=0"])
        .arg(format!("--fsize={}", access.file_limit))
        .arg("--")
        .arg(&tools.bwrap.path)
        .args([
            "--unshare-all",
            "--unshare-user",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
            "--setenv",
            "LC_ALL",
            "C",
            "--setenv",
            "QEMU_MODULE_DIR",
            "/nonexistent",
            "--dir",
            "/tool",
            "--dir",
            "/input",
            "--dir",
            "/output",
            "--dir",
            "/etc",
            "--ro-bind",
        ])
        .arg(&tools.qemu.path)
        .arg("/tool/qemu-img");
    for path in [
        "/usr/lib",
        "/usr/lib64",
        "/lib",
        "/lib64",
        "/etc/ld.so.cache",
    ] {
        if Path::new(path).exists() {
            command.arg("--ro-bind").arg(path).arg(path);
        }
    }
    // No host procfs, devices, home, sockets or arbitrary input directory is mounted.
    if let Some(input) = access.input {
        command.arg("--ro-bind").arg(input).arg("/input/source");
    }
    if let Some(output) = access.output {
        command
            .arg(if access.write_output {
                "--bind"
            } else {
                "--ro-bind"
            })
            .arg(output)
            .arg("/output");
    }
    command
        .args(["--chdir", "/", "--seccomp", "0", "--", "/tool/qemu-img"])
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let filter = filter()?;
    xenadmin_process::close_inherited_on_exec(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| fail("Cannot start the restricted conversion process"))?;
    let feed = child
        .stdin
        .take()
        .ok_or_else(|| fail("Cannot create sandbox policy input"))?;
    let policy = std::thread::spawn(move || {
        use std::io::Write;
        let mut feed = feed;
        feed.write_all(&filter)
    });
    let overflow = Arc::new(AtomicBool::new(false));
    fn reader(
        mut pipe: impl Read + Send + 'static,
        overflow: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut chunk = [0; 8192];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if out.len() + n > OUTPUT_LIMIT {
                    overflow.store(true, Ordering::Relaxed);
                } else {
                    out.extend_from_slice(&chunk[..n]);
                }
            }
            out
        })
    }
    let stdout = reader(
        child
            .stdout
            .take()
            .ok_or_else(|| fail("Missing tool output"))?,
        overflow.clone(),
    );
    let stderr = reader(
        child
            .stderr
            .take()
            .ok_or_else(|| fail("Missing tool diagnostics"))?,
        overflow.clone(),
    );
    let start = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed)
            || overflow.load(Ordering::Relaxed)
            || start.elapsed() > access.deadline
        {
            if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            break Err(fail(if cancel.load(Ordering::Relaxed) {
                "Conversion cancelled"
            } else {
                "Local tool exceeded its time or output limit"
            }));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(fail(
                        "Local tool or required sandbox refused the operation; no fallback was used",
                    ))
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(fail("Cannot observe the local conversion process"));
            }
        }
    };
    let _ = policy.join();
    let output = stdout
        .join()
        .map_err(|_| fail("Tool output worker stopped"))?;
    let _ = stderr.join();
    result?;
    stopped(cancel)?;
    Ok(output)
}

// The desktop distribution target is Linux x86-64. Deny process creation while
// allowing QEMU's worker threads; clone3 falls back to the checked clone ABI.
fn filter() -> Result<Vec<u8>> {
    if !cfg!(target_arch = "x86_64") {
        return Err(fail("Conversion sandbox is qualified only on Linux x86-64"));
    }
    let mut code: Vec<(u16, u8, u8, u32)> = vec![
        (0x20, 0, 0, 4),
        (0x15, 1, 0, 0xc000003e),
        (0x06, 0, 0, 0x80000000),
        (0x20, 0, 0, 0),
        (0x35, 0, 1, 0x40000000),
        (0x06, 0, 0, 0x80000000),
    ];
    for call in [
        57, 58, 101, 165, 166, 169, 175, 176, 246, 248, 249, 250, 272, 298, 304, 308, 321, 323,
    ] {
        code.extend([(0x15, 0, 1, call), (0x06, 0, 0, 0x00050001)]);
    }
    code.extend([(0x15, 0, 1, 435), (0x06, 0, 0, 0x00050026)]); // ENOSYS: libc thread fallback.
    code.extend([
        (0x15, 0, 4, 56),
        (0x20, 0, 0, 16),
        (0x54, 0, 0, 0x10000),
        (0x15, 1, 0, 0x10000),
        (0x06, 0, 0, 0x00050001),
        (0x06, 0, 0, 0x7fff0000),
    ]);
    Ok(code
        .into_iter()
        .flat_map(|(op, jt, jf, k)| {
            let mut b = Vec::from(op.to_ne_bytes());
            b.extend([jt, jf]);
            b.extend(k.to_ne_bytes());
            b
        })
        .collect())
}
