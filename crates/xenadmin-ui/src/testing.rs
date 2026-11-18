//! Test support shared by the kernel, every area and the tests of the GPUI
//! front end. It is compiled for this crate's own tests and, for another
//! crate's tests, with the feature `test-support` (a dev-dependency turns
//! it on); no released binary has it.
//!
//! - [`TempDir`]: a private directory under the system's temporary
//!   directory, removed on drop (the crate has no `tempfile`).
//! - [`Fixture`]: a model with a writable, empty profile store in a private
//!   directory (the Rune `profiles_view::tests::Fixture`).
//! - [`Server`]: a local JSON-RPC server. [`Server::new`] serves one pool;
//!   [`Server::with_vm`] a pool, a host and a halted VM that can be started
//!   once, with the task of that start. With [`connected`] (read-only) or
//!   [`connected_as`] a fixture gets a real `Backend` and a live, current
//!   session against it, so a test can have a current snapshot, an
//!   `ObservedVm` and a submission that succeeds, without a host.
//! - [`Live`]: all of it in one value, for a test of a surface that needs
//!   an observed VM:
//!
//! ```ignore
//! let mut live = Live::with_vm();                 // writable, current, the scope session
//! live.h.select(Target::one(live.vm()));
//! live.h.invoke("vm.start");
//! poll_until(&mut live.h, "the start succeeded", |h| {
//!     h.model.operation_rows.iter().any(|r| r.state == OperationState::Succeeded)
//! });
//! assert_eq!(live.server.count("Async.VM.start"), 1);
//! ```
//!
//! - [`custom_templates`]: the demo inventory has no custom template; this
//!   makes some of a model's guests into them.
//!
//! The parts, for a test that wants a read-only session or several pools:
//!
//! ```ignore
//! let pool = PoolId::from(uuid::Uuid::new_v4());
//! let server = Server::new(pool);
//! let mut fixture = Fixture::new();
//! let profile = connected(&mut fixture, &server, pool);
//! let mut h = Harness::with(std::mem::take(&mut fixture.model));
//! poll_until(&mut h, "the pool is shown", |h| h.model.inventory.is_some());
//! // keep `fixture` and `server` alive until the end of the test
//! // a second pool: connect(&mut h.model, &other_server, other_pool, "Other", false)
//! ```
//!
//! What the server answers: `session.login_with_password`,
//! `session.get_record` (a local superuser unless [`Server::session`] says
//! otherwise), `event.from` (the inventory first, then what changed),
//! `session.logout`; with a VM also `host.get_all_records`,
//! `Async.VM.start`, `task.get_record` and `task.destroy`. Any other
//! request panics in its thread and the client sees a closed connection
//! ([`Server::calls`] still lists it); with [`Server::hold_unserved`] it
//! waits instead, which keeps a resource operation unfinished. A test that needs more of the API
//! extends `respond` here.

use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use xenadmin_core::{
    Backend, Connection, ConnectionProfile, CredentialMode, Credentials, Endpoint, HostId, PoolId,
    ProfileStore, VmId,
};

use crate::Harness;
use crate::model::Model;
use crate::object::ObjectRef;
use crate::session::{self, Workspace};

/// Gives a fixture custom templates, which the demo inventory lacks: the
/// last `count` guests of the scope session's inventory become halted
/// custom templates (`is_default_template == Some(false)`, no current or
/// blocked operation), named "Custom template 1", "Custom template 2", …
/// Returns them as tree objects, in that order. The first guests, which
/// most tests use, stay guests.
///
/// For a model a test owns (`Harness::with`, the GPUI tests'
/// `open_scene_with`): the plain demo keeps its object counts, which
/// kernel tests rely on.
pub fn custom_templates(m: &mut Model, count: usize) -> Vec<ObjectRef> {
    let Some(inventory) = m.inventory.take() else {
        return Vec::new();
    };
    let mut inventory = inventory.as_ref().clone();
    let pool = inventory.pool_id;
    let mut ids: Vec<VmId> = inventory
        .vms
        .values()
        .filter(|vm| !vm.is_template && !vm.is_control_domain)
        .map(|vm| vm.id)
        .collect();
    let first = ids.len().saturating_sub(count);
    let ids = ids.split_off(first);
    for (index, id) in ids.iter().enumerate() {
        if let Some(vm) = inventory.vms.get_mut(id) {
            vm.is_template = true;
            vm.power_state = xenadmin_core::PowerState::Halted;
            vm.name = format!("Custom template {}", index + 1);
        }
        if let Some(details) = inventory.details.vms.get_mut(id) {
            details.is_default_template = Some(false);
            details.current_operations.clear();
            details.blocked_operations.clear();
        }
    }
    inventory.revision += 1;
    m.inventory = Some(Arc::new(inventory));
    ids.into_iter()
        .map(|id| ObjectRef::template(pool, id))
        .collect()
}

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

/// What a [`Server`] serves and what it was asked.
struct World {
    pool: PoolId,
    /// The host and the VM of [`Server::with_vm`].
    guest: Option<(HostId, VmId)>,
    /// The answer to `session.get_record`.
    session: Value,
    /// Every method asked, in order.
    calls: Vec<String>,
    /// The VM was started and its task has been read as a success.
    running: bool,
    /// `Async.VM.start` is received and not answered.
    lose_replies: bool,
    /// `task.get_record` says the task is pending.
    hold_tasks: bool,
    /// A request the server does not serve waits instead of failing.
    hold_unserved: bool,
    /// Events for the next `event.from`.
    pending: Vec<Value>,
}

impl World {
    fn vm_record(&self, vm: VmId) -> Value {
        let (state, allowed) = if self.running {
            (
                "Running",
                json!([
                    "clean_shutdown",
                    "hard_shutdown",
                    "clean_reboot",
                    "hard_reboot"
                ]),
            )
        } else {
            ("Halted", json!(["start", "start_on"]))
        };
        json!({
            "uuid": vm.to_string(), "name_label": "Fixture guest", "power_state": state,
            "is_a_snapshot": false, "is_a_template": false, "is_control_domain": false,
            "VCPUs_at_startup": "2", "memory_static_max": "2147483648",
            "allowed_operations": allowed,
        })
    }

    fn host_record(host: HostId) -> Value {
        json!({
            "uuid": host.to_string(), "name_label": "Fixture host",
            "address": "192.0.2.10", "enabled": true,
        })
    }

    fn inventory(&self) -> Vec<Value> {
        let event = |class: &str, reference: &str, record: Value| json!({"class": class, "operation": "add", "ref": reference, "snapshot": record});
        let mut pool = json!({"uuid": self.pool.to_string(), "name_label": "Fixture pool"});
        let mut events = Vec::new();
        if let Some((host, vm)) = self.guest {
            pool["master"] = json!("OpaqueRef:host");
            events.push(event("host", "OpaqueRef:host", Self::host_record(host)));
            events.push(event("VM", "OpaqueRef:vm", self.vm_record(vm)));
        }
        events.insert(0, event("pool", "OpaqueRef:pool", pool));
        events
    }
}

/// A local JSON-RPC server that answers a login, the session's record, the
/// event stream of one pool and a logout (the Rune fixture); with
/// [`Server::with_vm`] also one start of its VM.
pub struct Server {
    pub address: std::net::SocketAddr,
    pub endpoint: Endpoint,
    stop: Arc<AtomicBool>,
    logins: Arc<Mutex<Vec<(String, String)>>>,
    world: Arc<Mutex<World>>,
}
impl Server {
    /// A pool without hosts or VMs.
    pub fn new(pool: PoolId) -> Self {
        Self::on(std::net::TcpListener::bind("127.0.0.1:0").unwrap(), pool)
    }

    /// A pool "Fixture pool" with one enabled host "Fixture host" (its
    /// coordinator) and one halted VM "Fixture guest" that allows `start`:
    /// [`Server::host`], [`Server::vm`]. `Async.VM.start` is accepted, its
    /// task succeeds, and the event stream then says the VM is running.
    pub fn with_vm(pool: PoolId) -> Self {
        let server = Self::new(pool);
        server.world.lock().unwrap().guest = Some((
            HostId::from(uuid::Uuid::new_v4()),
            VmId::from(uuid::Uuid::new_v4()),
        ));
        server
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
        let world = Arc::new(Mutex::new(World {
            pool,
            guest: None,
            session: json!({"is_local_superuser": true}),
            calls: Vec::new(),
            running: false,
            lose_replies: false,
            hold_tasks: false,
            hold_unserved: false,
            pending: Vec::new(),
        }));
        let signal = stop.clone();
        let seen = logins.clone();
        let shared = world.clone();
        std::thread::spawn(move || {
            while !signal.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (logins, world) = (seen.clone(), shared.clone());
                        std::thread::spawn(move || serve(stream, &world, &logins));
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
            world,
        }
    }

    pub fn logins(&self) -> Vec<(String, String)> {
        self.logins.lock().unwrap().clone()
    }

    /// The VM of [`Server::with_vm`].
    pub fn vm(&self) -> VmId {
        self.world.lock().unwrap().guest.expect("Server::with_vm").1
    }

    /// The host of [`Server::with_vm`].
    pub fn host(&self) -> HostId {
        self.world.lock().unwrap().guest.expect("Server::with_vm").0
    }

    /// Every method the server was asked, in order, also one it refused.
    pub fn calls(&self) -> Vec<String> {
        self.world.lock().unwrap().calls.clone()
    }

    /// How often a method was asked: `server.count("Async.VM.start")`.
    pub fn count(&self, method: &str) -> usize {
        let world = self.world.lock().unwrap();
        world.calls.iter().filter(|call| *call == method).count()
    }

    /// What `session.get_record` answers at the next login: the default is
    /// `{"is_local_superuser": true}` (everything granted). A session that
    /// may not start a VM: `{"is_local_superuser": false, "rbac_permissions":
    /// ["session.logout"]}`. Call it before the session connects.
    pub fn session(&self, record: Value) {
        self.world.lock().unwrap().session = record;
    }

    /// While set, `Async.VM.start` is received and never answered: the
    /// client cannot know whether the VM started, and its operation ends as
    /// an unknown outcome.
    pub fn lose_replies(&self, lose: bool) {
        self.world.lock().unwrap().lose_replies = lose;
    }

    /// While set, the task of a start stays pending, so its operation stays
    /// unfinished with a host task; cleared, the next read of the task is a
    /// success.
    pub fn hold_tasks(&self, hold: bool) {
        self.world.lock().unwrap().hold_tasks = hold;
    }

    /// While set, a request the server does not serve (the first call of a
    /// resource operation, `pool.get_by_uuid` for one on the pool) is kept
    /// waiting, so the operation stays unfinished; cleared, the request is
    /// dropped without a reply and the operation ends as the core ends one
    /// whose host cannot be reached (`Rejected`). Held for ten seconds at
    /// most. Without it such a request panics in its thread, with the same
    /// end for the operation.
    pub fn hold_unserved(&self, hold: bool) {
        self.world.lock().unwrap().hold_unserved = hold;
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release)
    }
}

/// Answers one request of one connection.
fn serve(stream: std::net::TcpStream, world: &Mutex<World>, logins: &Mutex<Vec<(String, String)>>) {
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
    let request: Value = serde_json::from_slice(&bytes).unwrap();
    let method = request["method"].as_str().unwrap();
    world.lock().unwrap().calls.push(method.to_owned());
    if method == "session.login_with_password" {
        logins.lock().unwrap().push((
            request["params"][0].as_str().unwrap().into(),
            request["params"][1].as_str().unwrap().into(),
        ));
    }
    let Some(result) = respond(world, method, &request["params"]) else {
        // The reply is lost: the connection closes without one.
        return;
    };
    let body =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
            .unwrap();
    let mut stream = input.into_inner();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
}

/// The result of one call; `None` when the reply is to be lost.
fn respond(world: &Mutex<World>, method: &str, params: &Value) -> Option<Value> {
    let has_vm = world.lock().unwrap().guest.is_some();
    Some(match method {
        "session.login_with_password" => json!("OpaqueRef:fixture-session"),
        "session.get_record" => world.lock().unwrap().session.clone(),
        "session.logout" => Value::Null,
        "event.from" if params[2] == "" => {
            let mut world = world.lock().unwrap();
            world.pending.clear();
            json!({"token": "fixture", "events": world.inventory()})
        }
        "event.from" => {
            // The long poll: what changed, as soon as something has.
            let start = Instant::now();
            let events = loop {
                let events = std::mem::take(&mut world.lock().unwrap().pending);
                if !events.is_empty() || start.elapsed() >= Duration::from_millis(100) {
                    break events;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            json!({"token": "fixture", "events": events})
        }
        "host.get_all_records" if has_vm => {
            let host = world.lock().unwrap().guest.unwrap().0;
            json!({"OpaqueRef:host": World::host_record(host)})
        }
        "Async.VM.start" if has_vm => {
            assert_eq!(params[1], "OpaqueRef:vm", "the VM of the fixture");
            if world.lock().unwrap().lose_replies {
                return None;
            }
            json!("OpaqueRef:task")
        }
        "task.get_record" if has_vm => {
            let mut world = world.lock().unwrap();
            if world.hold_tasks {
                json!({"uuid": TASK, "status": "pending", "progress": 0.5,
                    "allowed_operations": []})
            } else {
                if !world.running {
                    world.running = true;
                    let vm = world.guest.unwrap().1;
                    let record = world.vm_record(vm);
                    world.pending.push(json!({
                        "class": "VM", "operation": "mod", "ref": "OpaqueRef:vm",
                        "snapshot": record,
                    }));
                }
                json!({"uuid": TASK, "status": "success", "progress": 1.0,
                    "allowed_operations": []})
            }
        }
        "task.destroy" if has_vm => Value::Null,
        _ if world.lock().unwrap().hold_unserved => {
            let start = Instant::now();
            while world.lock().unwrap().hold_unserved && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            return None;
        }
        _ => panic!("Unexpected fixture request: {method}"),
    })
}

/// The UUID of the one task the server knows.
const TASK: &str = "44444444-4444-4444-4444-444444444444";

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
    connected_as(f, server, pool, "Fixture", true)
}

/// [`connected`] under a name and writable or not. A writable session can
/// submit: its journal is in the fixture's private directory. The backend
/// is started once; a second call adds a second session to it (and makes
/// that one the scope session).
pub fn connected_as(
    f: &mut Fixture,
    server: &Server,
    pool: PoolId,
    name: &str,
    read_only: bool,
) -> ConnectionProfile {
    let m = &mut f.model;
    // `Fixture::new` leaves the connect form open, in front.
    session::cancel_editor(m);
    m.workspace.show_connect = false;
    m.workspace.password.clear();
    m.workspace.restore_started = true;
    if m.backend.is_err() {
        m.backend = Ok(Backend::with_state_directory(f.directory.clone()).unwrap());
    }
    connect(m, server, pool, name, read_only)
}

/// Adds a saved connection to a model that has a backend and a writable
/// profile store (a fixture after [`connected`], [`Live`]): connected to
/// the server, current, and the scope session, with the tree selection on
/// its pool. For a test of several pools; select the object you want next.
pub fn connect(
    m: &mut Model,
    server: &Server,
    pool: PoolId,
    name: &str,
    read_only: bool,
) -> ConnectionProfile {
    let mut profile =
        ConnectionProfile::new(name.into(), server.endpoint.clone(), "operator".into());
    profile.pool = Some(pool);
    profile.read_only = read_only;
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

/// A harness over one writable, current session against a [`Server`] with
/// a VM: what a test of a surface that captures an `ObservedVm`, or of a
/// submission that succeeds, starts from. The pool is the scope and
/// `h.model.can_mutate()` holds.
pub struct Live {
    pub h: Harness,
    pub server: Server,
    pub pool: PoolId,
    pub profile: ConnectionProfile,
    // After the harness: the model's backend writes into its directory.
    _fixture: Fixture,
}

impl Live {
    #[allow(clippy::new_without_default)]
    pub fn with_vm() -> Self {
        let pool = PoolId::from(uuid::Uuid::new_v4());
        Self::over(Server::with_vm(pool), pool, false)
    }

    /// Over a server the test prepared ([`Server::session`], a pool
    /// without a VM), writable or read-only.
    pub fn over(server: Server, pool: PoolId, read_only: bool) -> Self {
        let mut fixture = Fixture::new();
        let profile = connected_as(&mut fixture, &server, pool, "Fixture", read_only);
        let mut h = Harness::with(std::mem::take(&mut fixture.model));
        poll_until(&mut h, "the fixture pool is shown and current", |h| {
            h.model.inventory.is_some() && h.model.can_mutate_pool(pool) == !read_only
        });
        Self {
            h,
            server,
            pool,
            profile,
            _fixture: fixture,
        }
    }

    /// The server's VM as an object of the tree.
    pub fn vm(&self) -> ObjectRef {
        ObjectRef::vm(self.pool, self.server.vm())
    }

    /// The server's host as an object of the tree.
    pub fn host(&self) -> ObjectRef {
        ObjectRef::host(self.pool, self.server.host())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Target;
    use xenadmin_core::{OperationState, PermissionDecision, PowerAction, PowerState, VmCommand};

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
        // The session's record is answered: nothing was refused.
        assert_eq!(server.count("session.get_record"), 1);
        assert!(
            server.calls().iter().all(|call| matches!(
                call.as_str(),
                "session.login_with_password" | "session.get_record" | "event.from"
            )),
            "{:?}",
            server.calls()
        );
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

    // The server with a VM gives the core what it needs to observe the VM,
    // accepts one start, and says afterwards that the VM runs.
    #[test]
    fn the_server_with_a_vm_serves_an_observed_vm_and_one_start_that_succeeds() {
        let mut live = Live::with_vm();
        let (pool, vm, host) = (live.pool, live.server.vm(), live.server.host());
        let inventory = live.h.model.inventory.clone().expect("an inventory");
        assert_eq!(inventory.pool_id, pool);
        assert_eq!(inventory.pool_name, "Fixture pool");
        assert!(inventory.hosts[&host].enabled);
        let guest = &inventory.vms[&vm];
        assert_eq!(
            (guest.name.as_str(), &guest.power_state),
            ("Fixture guest", &PowerState::Halted)
        );
        assert!(guest.allowed_actions.contains(&PowerAction::Start));
        assert!(live.h.model.can_mutate() && live.h.model.can_mutate_pool(pool));
        assert_eq!(
            live.h.model.scope(),
            &Target::one(ObjectRef::pool(pool)),
            "the session's pool is the scope"
        );
        // A local superuser: the permission is known and granted.
        assert_eq!(
            crate::permissions::power_for(&live.h.model, pool, PowerAction::Start),
            Some(PermissionDecision::Granted)
        );

        let observed = live
            .h
            .model
            .snapshot_for(pool)
            .and_then(|snapshot| snapshot.observe_vm(vm).ok())
            .expect("an observed VM");
        let id = live
            .h
            .model
            .submit_tracked_command_for(
                pool,
                VmCommand::Power {
                    vm: observed,
                    action: PowerAction::Start,
                },
            )
            .expect("the start is submitted");
        poll_until(&mut live.h, "the start succeeded", |h| {
            h.model
                .operation_rows
                .iter()
                .any(|r| r.id == id && r.state == OperationState::Succeeded)
        });
        assert_eq!(live.server.count("Async.VM.start"), 1);
        // The core destroys its task after it has recorded the outcome, so
        // the request can arrive a moment after the record says Succeeded
        // (it did on a slow runner).
        poll_until(&mut live.h, "the finished task is destroyed", |_| {
            live.server.count("task.destroy") == 1
        });
        poll_until(&mut live.h, "the VM is shown running", |h| {
            h.model
                .inventory_for(pool)
                .is_some_and(|i| i.vms[&vm].power_state == PowerState::Running)
        });
        assert_eq!(live.server.logins().len(), 1);
    }
}
