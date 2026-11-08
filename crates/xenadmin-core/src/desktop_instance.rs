//! Local desktop ownership and activation. This channel never accepts host commands.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;
const LIMIT: usize = 16 * 1024;
const TIMEOUT: Duration = Duration::from_millis(500);
const SOCKET: &str = "desktop-activate.sock";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    command: String,
    token: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u8,
    accepted: bool,
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn uid() -> io::Result<u32> {
    Ok(fs::metadata("/proc/self")?.uid())
}
fn directory(path: &Path, owner: u32) -> io::Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != owner || m.mode() & 0o777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Desktop state directory must be owned by this user with mode 0700",
        ));
    }
    Ok(())
}
fn socket_metadata(path: &Path, owner: u32) -> io::Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_socket() || m.uid() != owner || m.mode() & 0o777 != 0o600 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Desktop activation path must be a private user-owned socket",
        ));
    }
    Ok(m)
}
fn validate_token(token: Option<&str>) -> io::Result<()> {
    if token.is_some_and(|t| t.len() > 4096 || t.chars().any(char::is_control)) {
        return Err(invalid("Invalid desktop activation token"));
    }
    Ok(())
}
struct SocketFile {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl Drop for SocketFile {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| {
            m.file_type().is_socket() && m.dev() == self.device && m.ino() == self.inode
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
struct Service {
    cancel: CancellationToken,
    worker: Option<std::thread::JoinHandle<()>>,
    _socket: SocketFile,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
/// Holds the exclusive desktop lock and the optional activation listener for this process.
pub struct DesktopInstance {
    file: File,
    process_lock_retained: bool,
    directory: PathBuf,
    service: Option<Service>,
}
impl DesktopInstance {
    /// `None` means another instance holds the lock; no backend should be started in that case.
    pub fn acquire(path: PathBuf) -> io::Result<Option<Self>> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)?;
        let owner = uid()?;
        let m = fs::symlink_metadata(&path)?;
        if !m.is_dir() || m.uid() != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Desktop state directory is not owned by this user",
            ));
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        directory(&path, owner)?;
        let fd = rustix::fs::open(
            path.join("desktop.lock"),
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )?;
        let file = File::from(fd);
        let m = file.metadata()?;
        if !m.is_file() || m.uid() != owner || m.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Desktop lock is not a private regular file",
            ));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                file,
                process_lock_retained: false,
                directory: path,
                service: None,
            })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }
    /// Pins one close-on-exec descriptor until this process exits. The desktop uses this
    /// before creating its backend so asynchronous runtime teardown cannot release ownership early.
    /// Listener cleanup still happens when this guard drops; the OS releases the pinned lock.
    pub fn retain_lock_until_exit(&mut self) -> io::Result<()> {
        if !self.process_lock_retained {
            let retained = self.file.try_clone()?;
            rustix::io::fcntl_setfd(&retained, rustix::io::FdFlags::CLOEXEC)?;
            std::mem::forget(retained);
            self.process_lock_retained = true;
        }
        Ok(())
    }
    /// Installs a nonblocking callback that queues activation on the desktop event loop.
    /// Call before starting the native event loop; the callback must not run host operations.
    pub fn start_activation(
        &mut self,
        activate: Arc<dyn Fn(Option<String>) + Send + Sync>,
    ) -> io::Result<()> {
        if self.service.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Desktop activation is already running",
            ));
        }
        let owner = uid()?;
        directory(&self.directory, owner)?;
        let path = self.directory.join(SOCKET);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        if fs::symlink_metadata(&path).is_ok() {
            let old = socket_metadata(&path, owner)?;
            if !matches!(runtime.block_on(async {tokio::time::timeout(TIMEOUT,UnixStream::connect(&path)).await}),Ok(Err(e)) if e.kind()==io::ErrorKind::ConnectionRefused)
            {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "Desktop activation socket is already active",
                ));
            }
            let current = socket_metadata(&path, owner)?;
            if current.dev() != old.dev() || current.ino() != old.ino() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "Desktop activation socket changed",
                ));
            }
            fs::remove_file(&path)?;
        }
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        let m = fs::symlink_metadata(&path)?;
        let socket = SocketFile {
            path,
            device: m.dev(),
            inode: m.ino(),
        };
        fs::set_permissions(&socket.path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        let worker=std::thread::Builder::new().name("desktop-activation".into()).spawn(move||runtime.block_on(async move{
            let Ok(listener)=UnixListener::from_std(listener)else{return;};let slots=Arc::new(Semaphore::new(4));
            loop {tokio::select!{_ = stopped.cancelled()=>break,accepted=listener.accept()=>{let Ok((stream,_))=accepted else{continue;};if !stream.peer_cred().is_ok_and(|c|c.uid()==owner){continue;}let Ok(permit)=slots.clone().try_acquire_owned()else{continue;};let activate=activate.clone();tokio::spawn(async move{let _permit=permit;let _=tokio::time::timeout(TIMEOUT,serve(stream,activate)).await;});}}}
        }))?;
        self.service = Some(Service {
            cancel,
            worker: Some(worker),
            _socket: socket,
        });
        Ok(())
    }
}
impl Drop for DesktopInstance {
    fn drop(&mut self) {
        self.service.take();
        if !self.process_lock_retained {
            // A concurrent fork may retain this open file description until exec. Release
            // the unpinned owner's lock now rather than waiting for every inherited fd.
            let _ = self.file.unlock();
        }
    }
}
async fn line(reader: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    BufReader::new(reader.take((LIMIT + 1) as u64))
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.len() > LIMIT || !bytes.ends_with(b"\n") {
        return Err(invalid("Invalid desktop activation frame"));
    }
    Ok(bytes)
}
async fn serve(
    mut stream: UnixStream,
    activate: Arc<dyn Fn(Option<String>) + Send + Sync>,
) -> io::Result<()> {
    let request: Request = serde_json::from_slice(&line(&mut stream).await?)?;
    if request.version != 1 || request.command != "activate" {
        return Err(invalid("Unsupported desktop activation request"));
    }
    validate_token(request.token.as_deref())?;
    activate(request.token.filter(|t| !t.is_empty()));
    stream
        .write_all(b"{\"version\":1,\"accepted\":true}\n")
        .await
}
/// For a second launcher only. Acknowledgement means activation was queued, not granted by the compositor.
/// This blocking call never reads profiles, credentials or host inventory and never starts a backend.
pub fn activate_desktop(path: &Path, token: Option<String>) -> io::Result<()> {
    validate_token(token.as_deref())?;
    let owner = uid()?;
    directory(path, owner)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let target = path.join(SOCKET);
        let mut stream = loop {
            let result = match socket_metadata(&target, owner) {
                Ok(_) => tokio::time::timeout(TIMEOUT, UnixStream::connect(&target))
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Desktop activation connection timed out",
                        )
                    })?,
                Err(e) => Err(e),
            };
            match result {
                Ok(stream) => break stream,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(25)).await
                }
                Err(e) => return Err(e),
            }
        };
        if stream.peer_cred()?.uid() != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Desktop activation peer has another owner",
            ));
        }
        let mut bytes = serde_json::to_vec(&Request {
            version: 1,
            command: "activate".into(),
            token,
        })?;
        bytes.push(b'\n');
        tokio::time::timeout(TIMEOUT, async {
            stream.write_all(&bytes).await?;
            let response: Reply = serde_json::from_slice(&line(stream).await?)?;
            if response.version != 1 || !response.accepted {
                return Err(invalid("Desktop activation was not accepted"));
            }
            Ok(())
        })
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "Desktop activation acknowledgement timed out",
            )
        })?
    })
}
#[cfg(test)]
#[path = "desktop_instance_tests.rs"]
mod tests;
