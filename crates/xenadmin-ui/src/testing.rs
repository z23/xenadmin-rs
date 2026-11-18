//! Test support shared by the kernel, every area and the tests of the GPUI
//! front end. It is compiled for this crate's own tests and, for another
//! crate's tests, with the feature `test-support` (a dev-dependency turns
//! it on); no released binary has it.
//!
//! - [`TempDir`]: a private directory under the system's temporary
//!   directory, removed on drop (the crate has no `tempfile`).
//! - [`Fixture`]: a model with a writable, empty profile store in a private
//!   directory (the Rune `profiles_view::tests::Fixture`).
//! - [`Server`]: a local JSON-RPC server that answers a login, the first
//!   event batch with one pool, and a logout. With [`connected`] a fixture
//!   gets a real `Backend` and a live, current session against it, so a
//!   test can have a current snapshot without a host:
//!
//! ```ignore
//! let pool = PoolId::from(uuid::Uuid::new_v4());
//! let server = Server::new(pool);
//! let mut fixture = Fixture::new();
//! let profile = connected(&mut fixture, &server, pool);
//! let mut h = Harness::with(std::mem::take(&mut fixture.model));
//! poll_until(&mut h, "the pool is shown", |h| h.model.inventory.is_some());
//! // keep `fixture` and `server` alive until the end of the test
//! ```
//!
//! The server knows three calls; any other request panics in its thread
//! and the client sees a closed connection. A test that needs more of the
//! API extends `serve` here.

use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use xenadmin_core::{
    Backend, Connection, ConnectionProfile, CredentialMode, Credentials, Endpoint, PoolId,
    ProfileStore,
};

use crate::Harness;
use crate::model::Model;
use crate::session::{self, Workspace};

/// A private directory under the system's temporary directory, removed
/// with everything in it on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    /// `label` goes into the directory's name, to tell whose it is when a
    /// crashed run leaves one behind.
    pub fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("xenadmin-ui-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A model that owns a private, empty state directory with a writable
/// profile store. It leaves the connect form open, in front (the Rune
/// fixture did): `Harness::with(std::mem::take(&mut fixture.model))`, and
/// keep the fixture alive, because its directory goes with it.
pub struct Fixture {
    pub model: Model,
    pub directory: PathBuf,
}

impl Fixture {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("xenadmin-profiles-ui-{}", uuid::Uuid::new_v4()));
        let mut model = Model::fixture();
        model.workspace = Workspace::load(None, false);
        model.workspace.writable = true;
        model.workspace.store = Ok(ProfileStore::open(directory.clone()).unwrap());
        session::begin_new(&mut model);
        Self { model, directory }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// A local JSON-RPC server that answers a login, the first event batch
/// with one pool, and a logout (the Rune fixture).
pub struct Server {
    pub address: std::net::SocketAddr,
    pub endpoint: Endpoint,
    stop: Arc<AtomicBool>,
    logins: Arc<Mutex<Vec<(String, String)>>>,
}
impl Server {
    pub fn new(pool: PoolId) -> Self {
        Self::on(std::net::TcpListener::bind("127.0.0.1:0").unwrap(), pool)
    }

    /// A server on the address another one had: the host is back. The
    /// address is free once the thread of the dropped server has seen
    /// its stop flag.
    pub fn at(address: std::net::SocketAddr, pool: PoolId) -> Self {
        let start = Instant::now();
        loop {
            match std::net::TcpListener::bind(address) {
                Ok(listener) => return Self::on(listener, pool),
                Err(error) => {
                    assert!(start.elapsed() < Duration::from_secs(3), "{error}");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    fn on(listener: std::net::TcpListener, pool: PoolId) -> Self {
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = Endpoint::parse(&format!("http://{address}")).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let logins = Arc::new(Mutex::new(Vec::new()));
        let signal = stop.clone();
        let seen = logins.clone();
        std::thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let logins = seen.clone();
                        std::thread::spawn(move || serve(stream, pool, &logins));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            endpoint,
            stop,
            logins,
        }
    }

    pub fn logins(&self) -> Vec<(String, String)> {
        self.logins.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release)
    }
}

/// Answers one request of one connection.
fn serve(stream: std::net::TcpStream, pool: PoolId, logins: &Mutex<Vec<(String, String)>>) {
    // The listener is non-blocking; the accepted stream must not be.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut input = std::io::BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    if length > 65536 {
        return;
    }
    let mut bytes = vec![0; length];
    if input.read_exact(&mut bytes).is_err() {
        return;
    }
    let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let method = request["method"].as_str().unwrap();
    let result = match method {
        "session.login_with_password" => {
            logins.lock().unwrap().push((
                request["params"][0].as_str().unwrap().into(),
                request["params"][1].as_str().unwrap().into(),
            ));
            serde_json::json!("OpaqueRef:fixture-session")
        }
        "event.from" => {
            if request["params"][2] != "" {
                std::thread::sleep(Duration::from_millis(100));
                serde_json::json!({"token": "fixture", "events": []})
            } else {
                serde_json::json!({"token": "fixture", "events": [{
                    "class": "pool",
                    "operation": "add",
                    "ref": "OpaqueRef:pool",
                    "snapshot": {"uuid": pool.to_string(), "name_label": "Fixture pool"}
                }]})
            }
        }
        "session.logout" => serde_json::Value::Null,
        _ => panic!("Unexpected fixture request: {method}"),
    };
    let body = serde_json::to_vec(
        &serde_json::json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
    )
    .unwrap();
    let mut stream = input.into_inner();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
}

/// Polls the model, as its window does on every wake, until the
/// condition holds.
pub fn poll_until(h: &mut Harness, what: &str, condition: impl Fn(&Harness) -> bool) {
    let start = Instant::now();
    loop {
        h.tick(0.02);
        if condition(h) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn ready(c: &Connection) {
    let start = Instant::now();
    while !c.latest().is_current() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            c.latest().error()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Gives a fixture a real [`Backend`] (its state in the fixture's private
/// directory) and one saved connection, read-only, that is connected to the
/// server and is the scope session (the Rune `connected`). The model's
/// `connection`, `snapshot` and `inventory` are those of the session; the
/// snapshot is current, so `can_mutate` holds what a read-only session
/// allows. Returns the saved profile.
pub fn connected(f: &mut Fixture, server: &Server, pool: PoolId) -> ConnectionProfile {
    let m = &mut f.model;
    // `Fixture::new` leaves the connect form open, in front.
    session::cancel_editor(m);
    m.workspace.show_connect = false;
    m.workspace.password.clear();
    m.workspace.restore_started = true;
    m.backend = Ok(Backend::with_state_directory(f.directory.clone()).unwrap());
    let mut profile =
        ConnectionProfile::new("Fixture".into(), server.endpoint.clone(), "operator".into());
    profile.pool = Some(pool);
    profile.read_only = true;
    profile.credential_mode = CredentialMode::SecretService;
    profile.auto_connect = true;
    m.workspace
        .store
        .as_mut()
        .unwrap()
        .save(profile.clone())
        .unwrap();
    session::start_connection(
        m,
        &profile,
        Credentials::new("operator".into(), "synthetic-original".into()),
    )
    .unwrap();
    ready(m.workspace.active.get(&profile.id).unwrap());
    session::refresh(m);
    session::select(m, profile.id);
    m.workspace.profile(profile.id).unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_temporary_directory_is_private_to_its_guard_and_goes_with_it() {
        let (first, second) = (TempDir::new("guard"), TempDir::new("guard"));
        assert_ne!(first.path(), second.path());
        std::fs::write(first.path().join("file"), b"x").unwrap();
        let path = first.path().to_owned();
        drop(first);
        assert!(!path.exists());
        assert!(second.path().is_dir());
    }

    // One poll shows one state of the scope session: what `session::refresh`
    // stored for the connection list is what the model adopts, not a second
    // reading of the connection.
    #[test]
    fn a_poll_adopts_the_snapshot_the_connection_list_saw() {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        let server = Server::new(pool);
        let mut fixture = Fixture::new();
        let profile = connected(&mut fixture, &server, pool);
        let mut h = Harness::with(std::mem::take(&mut fixture.model));
        poll_until(&mut h, "the pool is shown", |h| {
            h.model.inventory.is_some() && h.model.can_mutate_pool(pool) == !profile.read_only
        });
        let listed = h.model.workspace.seen[&profile.id].clone();
        let adopted = h.model.snapshot.clone().expect("a snapshot");
        assert!(Arc::ptr_eq(&listed, &adopted));
        assert!(adopted.is_current());
        assert_eq!(server.logins().len(), 1);
        // The list holds an older snapshot than the connection has now (as
        // when the connection moved on between the two readings of one
        // poll): the model adopts the listed one.
        let older = Arc::new(adopted.as_ref().clone());
        h.model.workspace.seen.insert(profile.id, older.clone());
        assert!(h.model.poll_scope_session());
        assert!(Arc::ptr_eq(h.model.snapshot.as_ref().unwrap(), &older));
        // The next poll's refresh sees the connection's own snapshot again,
        // and both agree on it.
        h.tick(0.02);
        let listed = h.model.workspace.seen[&profile.id].clone();
        assert!(Arc::ptr_eq(&listed, h.model.snapshot.as_ref().unwrap()));
        assert!(!Arc::ptr_eq(&listed, &older));
    }
}
