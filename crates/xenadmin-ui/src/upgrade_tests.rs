//! The upgrade from the released client, whose window was the Rune
//! interface: a whole state directory it wrote, loaded as the start of the
//! application loads it (`main` and `start` of
//! `crates/xenadmin-desktop/src/app.rs`: `launch::prepare`,
//! `launch::attach`, `Model::live(owner, Preferences::load())`, then
//! `Model::setup`), and closed as the main window closes it
//! (`Model::close`).
//!
//! The test is ignored: it needs a directory, named by the environment
//! variable `XENADMIN_UPGRADE_STATE_DIR`.
//!
//! ```sh
//! XENADMIN_UPGRADE_STATE_DIR=/path/to/state cargo test -p xenadmin-ui --locked \
//!     a_state_directory_of_the_released_client -- --ignored --nocapture
//! ```
//!
//! What it does to be safe:
//!
//! - It copies the regular files of the directory into a temporary
//!   directory and works on the copy only. At the end it reads the
//!   original again and fails if a byte of it differs.
//! - It runs the load in a second process of this test binary, whose
//!   environment names the copy as the state directory and a temporary
//!   directory as home, and has no session bus.
//! - It never connects to a host. In a unit test of this crate a keyring
//!   job never reaches a keyring (`session::CredentialJob`), so the start-up
//!   restore of a saved connection waits for a password that never comes.
//!   The journal part connects to this crate's fixture server on the
//!   loopback interface (`testing::Server`), which stands for the pool the
//!   journal's records belong to: the core opens the journal only for a
//!   session with a pool.
//! - It prints counts and yes or no. A failed assertion names a field,
//!   never its value: no host name, address or fingerprint is printed.

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use serde_json::Value;
use xenadmin_core::PoolId;

use crate::launch::{self, Launch};
use crate::prefs::Preferences;
use crate::testing::{Server, TempDir, connect, poll_until};
use crate::{Harness, Model, session};

/// The directory to load a copy of.
const SOURCE: &str = "XENADMIN_UPGRADE_STATE_DIR";
/// Set in the second process.
const INNER: &str = "XENADMIN_UPGRADE_INNER";
const NAME: &str = "a_state_directory_of_the_released_client_loads_and_closes_without_loss";

const PREFERENCES: &str = "preferences.json";
const PROFILES: &str = "profiles.json";
const JOURNAL: &str = "operations.sqlite3";
/// The one key of the preferences file the new shell does not write back
/// (docs/GPUI-MIGRATION.md: the details pane is gone).
const DROPPED: &str = "inspector_width";

/// The regular files of a directory, by name. A socket or a link is left
/// out.
fn regular_files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(directory).expect("the state directory is readable") {
        let entry = entry.expect("directory entry");
        let kind = entry.file_type().expect("file type");
        if kind.is_file() {
            files.insert(
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).expect("a file of the state directory is readable"),
            );
        }
    }
    files
}

/// What a journal file holds, read with SQLite and not through the core.
#[derive(PartialEq, Eq)]
struct Journal {
    version: i64,
    /// id to (pool, payload) of `operations`.
    operations: BTreeMap<String, (String, String)>,
    /// id to (pool, payload) of `resource_operations`.
    resources: BTreeMap<String, (String, String)>,
    children: i64,
}

/// Reads the journal of a state directory, if it has one. SQLite makes
/// files beside a database it opens, even to read, so what is opened is a
/// copy of the journal's files in a directory of its own: the directory
/// itself is only read. Call it while no session of this process has the
/// journal open.
fn journal(directory: &Path) -> Option<Journal> {
    let files = regular_files(directory);
    files.get(JOURNAL)?;
    let probe = TempDir::new("upgrade-journal");
    // The copy holds what the journal holds (pools, VMs, the details of
    // operations): nobody else on the machine reads it.
    std::fs::set_permissions(probe.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private copy of the journal");
    for (name, bytes) in files.iter().filter(|(name, _)| name.starts_with(JOURNAL)) {
        std::fs::write(probe.path().join(name), bytes).expect("copy of the journal");
    }
    let db = rusqlite::Connection::open(probe.path().join(JOURNAL)).expect("the journal opens");
    let version = db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("journal version");
    let rows = |table: &str| -> BTreeMap<String, (String, String)> {
        let has: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .expect("table list");
        if has == 0 {
            return BTreeMap::new();
        }
        let mut statement = db
            .prepare(&format!("SELECT id, pool, payload FROM {table}"))
            .expect("journal rows");
        statement
            .query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))
            .expect("journal rows")
            .collect::<Result<_, _>>()
            .expect("journal rows")
    };
    let operations = rows("operations");
    let resources = rows("resource_operations");
    let children = db
        .query_row("SELECT count(*) FROM workflow_batch_children", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);
    Some(Journal {
        version,
        operations,
        resources,
        children,
    })
}

/// The keys under which two JSON objects differ.
fn differing(a: &Value, b: &Value) -> BTreeSet<String> {
    let empty = serde_json::Map::new();
    let (a, b) = (
        a.as_object().unwrap_or(&empty),
        b.as_object().unwrap_or(&empty),
    );
    a.keys()
        .chain(b.keys())
        .filter(|key| a.get(*key) != b.get(*key))
        .cloned()
        .collect()
}

fn yes(condition: bool) -> &'static str {
    if condition { "yes" } else { "no" }
}

#[test]
#[ignore = "needs a state directory of the released client in XENADMIN_UPGRADE_STATE_DIR"]
fn a_state_directory_of_the_released_client_loads_and_closes_without_loss() {
    assert_eq!(
        format!("{}::{NAME}", module_path!()),
        format!("{}::upgrade_tests::{NAME}", env!("CARGO_CRATE_NAME")),
        "the second process is asked for this test by name"
    );
    if std::env::var_os(INNER).is_some() {
        return load_and_close();
    }
    let source = std::env::var_os(SOURCE).unwrap_or_else(|| {
        panic!("set {SOURCE} to a state directory written by the released client")
    });
    let source = Path::new(&source);
    let original = regular_files(source);
    assert!(
        original.contains_key(PREFERENCES) || original.contains_key(PROFILES),
        "the directory holds neither {PREFERENCES} nor {PROFILES}"
    );
    let copy = TempDir::new("upgrade-state");
    std::fs::set_permissions(copy.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private copy");
    for (name, bytes) in &original {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(copy.path().join(name))
            .and_then(|mut file| file.write_all(bytes))
            .expect("copy of a file of the state directory");
    }
    let home = TempDir::new("upgrade-home");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(home.path().join("runtime"))
        .expect("runtime directory");
    let status = Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--ignored",
            "--exact",
            &format!("upgrade_tests::{NAME}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env(INNER, "1")
        .env_remove(SOURCE)
        .env("XENADMIN_STATE_DIR", copy.path())
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_RUNTIME_DIR", home.path().join("runtime"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/xenadmin-upgrade-test",
        )
        .env_remove("XENADMIN_DEV_INSPECT")
        .env_remove("XDG_ACTIVATION_TOKEN")
        .env_remove("DESKTOP_STARTUP_ID")
        .status()
        .expect("the second process starts");
    assert!(status.success(), "the load in the second process failed");
    assert!(
        regular_files(source) == original,
        "the original directory changed"
    );
    println!("upgrade: the original directory is unchanged: yes");
}

/// The second process: its state directory is the copy.
fn load_and_close() {
    let directory = crate::prefs::directory().expect("a state directory");
    assert!(
        std::env::var_os(SOURCE).is_none()
            && std::env::var_os("XENADMIN_STATE_DIR").is_some()
            && directory.starts_with(std::env::temp_dir()),
        "the second process works on the temporary copy only"
    );
    let pristine = regular_files(&directory);
    let preferences_file: Value = pristine
        .get(PREFERENCES)
        .map(|bytes| serde_json::from_slice(bytes).expect("preferences.json is JSON"))
        .unwrap_or(Value::Null);
    let saved: Vec<Value> = pristine
        .get(PROFILES)
        .map(|bytes| serde_json::from_slice::<Value>(bytes).expect("profiles.json is JSON"))
        .and_then(|file| file.get("profiles").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let journal_before = journal(&directory);
    println!(
        "upgrade: files {}; saved connections {}; preference keys {}; journal {}",
        pristine.len(),
        saved.len(),
        preferences_file.as_object().map_or(0, serde_json::Map::len),
        journal_before
            .as_ref()
            .map_or("absent".to_owned(), |j| format!(
                "format {} with {} VM operation(s), {} resource operation(s), {} batch member(s)",
                j.version,
                j.operations.len(),
                j.resources.len(),
                j.children
            )),
    );

    // The start.
    let owner = match launch::prepare() {
        Ok(Launch::Owner(owner)) => owner,
        Ok(_) => panic!("the start did not become the owner of the copy"),
        Err(error) => panic!("the start failed: {error}"),
    };
    let owner = launch::attach(owner, Arc::new(|_| {})).expect("activation and lock");
    let mut m = Model::live(owner, Preferences::load());
    m.setup(Arc::new(|| {}));
    assert!(m.owns_state());
    assert!(m.error.is_none(), "the start reports an error");

    // The saved connections: every field the file has, as the file has it.
    let store = m
        .workspace
        .store
        .as_ref()
        .unwrap_or_else(|_| panic!("the profile store of the copy did not open"));
    let loaded: Vec<Value> = store
        .profiles()
        .map(|profile| serde_json::to_value(profile).expect("profile"))
        .collect();
    assert_eq!(loaded.len(), saved.len(), "saved connections");
    let mut pins = 0;
    for entry in &saved {
        let profile = loaded
            .iter()
            .find(|profile| profile["id"] == entry["id"])
            .expect("a saved connection of the file is in the store");
        for key in [
            "name",
            "endpoint",
            "username",
            "host_certificates",
            "certificate_details",
            "verified_members",
            "pool",
            "credential_mode",
            "auto_connect",
            "accept_invalid_certificates",
            "read_only",
            "request_timeout_seconds",
        ] {
            assert!(
                entry.get(key).is_some() && profile.get(key) == entry.get(key),
                "the field {key} of a saved connection was not read as the file has it"
            );
        }
        let lost = differing(entry, profile)
            .into_iter()
            .filter(|key| entry.get(key).is_some())
            .collect::<Vec<_>>();
        assert!(
            lost.is_empty(),
            "fields of a saved connection lost: {lost:?}"
        );
        pins += entry["host_certificates"]
            .as_object()
            .map_or(0, |pins| pins.len());
    }
    // And they are listed: the Home tab's rows are these.
    let listed = session::sessions(&m);
    assert_eq!(listed.len(), saved.len(), "listed connections");
    for entry in &saved {
        assert!(
            listed.iter().any(|session| {
                serde_json::to_value(session.profile).expect("id") == entry["id"]
                    && entry["name"].as_str() == Some(session.name.as_str())
                    && session.state.is_none()
            }),
            "a saved connection is not listed under its name, disconnected"
        );
    }
    println!(
        "upgrade: saved connections listed with endpoint, user and pins as in the file: yes \
         ({} connection(s), {pins} certificate pin(s))",
        saved.len()
    );

    // The preferences: every key of the file but the dropped one.
    let read = serde_json::to_value(&m.preferences).expect("preferences");
    let mut expected = preferences_file.clone();
    if let Some(object) = expected.as_object_mut() {
        object.remove(DROPPED);
    }
    let unread = differing(&expected, &read)
        .into_iter()
        .filter(|key| expected.get(key).is_some())
        .collect::<Vec<_>>();
    assert!(
        unread.is_empty(),
        "preferences not read as the file has them: {unread:?}"
    );
    println!(
        "upgrade: preferences read as in the file: yes ({} key(s); the file has {DROPPED}: {})",
        expected.as_object().map_or(0, serde_json::Map::len),
        yes(preferences_file.get(DROPPED).is_some())
    );
    // The connect form starts from the remembered endpoint.
    for (key, held) in [
        ("endpoint", &m.workspace.endpoint),
        ("username", &m.workspace.username),
        ("fingerprint", &m.workspace.fingerprint),
        ("host_certificates", &m.workspace.host_certificates),
    ] {
        if let Some(value) = preferences_file.get(key) {
            assert!(
                value.as_str() == Some(held.as_str()),
                "the connect form does not start from the remembered {key}"
            );
        }
    }

    // The start-up restore asks the keyring for the connections that opted
    // in, and for no other; nothing connects.
    let opted = saved
        .iter()
        .filter(|entry| {
            entry["auto_connect"] == true && entry["credential_mode"] == "secret_service"
        })
        .count();
    assert_eq!(m.workspace.loads.len(), opted, "start-up restore");
    assert!(m.workspace.active.is_empty(), "nothing connects");
    println!("upgrade: connections asked to restore at start: {opted}; connected: 0");

    // The close of the main window.
    m.close();
    assert!(m.error.is_none(), "the close reports an error");
    let closed = regular_files(&directory);
    if pristine.contains_key(PREFERENCES) {
        let written: Value =
            serde_json::from_slice(&closed[PREFERENCES]).expect("preferences.json is JSON");
        let changed = differing(&expected, &written);
        assert!(
            changed.is_empty(),
            "the close changed more of preferences.json than {DROPPED}: {changed:?}"
        );
        assert!(written.get(DROPPED).is_none());
    }
    for name in [PROFILES, JOURNAL] {
        assert!(
            closed.get(name) == pristine.get(name),
            "the start and the close changed {name}"
        );
    }
    let added = closed
        .keys()
        .filter(|name| !pristine.contains_key(*name))
        .collect::<Vec<_>>();
    let removed = pristine
        .keys()
        .filter(|name| !closed.contains_key(*name))
        .collect::<Vec<_>>();
    assert!(removed.is_empty(), "files removed: {removed:?}");
    assert!(
        added.iter().all(|name| *name == "desktop.lock"),
        "files added: {added:?}"
    );
    println!(
        "upgrade: the close wrote preferences.json without {DROPPED} and nothing else: yes \
         (profiles.json and the journal are byte for byte the same)"
    );

    // The journal. The core opens it for a session with a pool; the pool
    // is the fixture server's, on the loopback interface.
    let Some(before) = journal_before else {
        println!("upgrade: no journal in this directory, nothing to open");
        return;
    };
    let pools: BTreeSet<&String> = before
        .operations
        .values()
        .chain(before.resources.values())
        .map(|(pool, _)| pool)
        .collect();
    let mut h = Harness::with(m);
    let mut servers = Vec::new();
    for (index, pool) in pools.iter().enumerate() {
        let pool: PoolId = serde_json::from_value(Value::from((*pool).clone())).expect("pool id");
        let server = Server::new(pool);
        connect(
            &mut h.model,
            &server,
            pool,
            &format!("Loopback fixture {index}"),
            false,
        );
        servers.push(server);
    }
    let wanted: BTreeSet<String> = before.operations.keys().cloned().collect();
    poll_until(&mut h, "the journal's VM operations are listed", |h| {
        let listed: BTreeSet<String> = h
            .model
            .operation_rows
            .iter()
            .filter_map(|record| {
                serde_json::to_value(record.id)
                    .ok()?
                    .as_str()
                    .map(str::to_owned)
            })
            .collect();
        wanted.is_subset(&listed)
    });
    // The wait above is the check: it ends only when every one is listed.
    let listed = h
        .model
        .operation_rows
        .iter()
        .filter(|record| {
            serde_json::to_value(record.id)
                .ok()
                .and_then(|id| id.as_str().map(|id| wanted.contains(id)))
                .unwrap_or(false)
        })
        .count();
    println!(
        "upgrade: VM operations of the journal listed by the model after a session opened: {listed} of {}",
        before.operations.len()
    );
    h.model.close();
    drop(h);
    drop(servers);
    let after = journal(&directory).expect("the journal is still there");
    assert!(after.version >= before.version, "journal format");
    for (table, was, is) in [
        ("operations", &before.operations, &after.operations),
        ("resource_operations", &before.resources, &after.resources),
    ] {
        assert_eq!(was.len(), is.len(), "rows of {table}");
        for (id, (pool, payload)) in was {
            let (pool_now, payload_now) = is.get(id).expect("a journal row is kept");
            assert!(pool == pool_now, "the pool of a row of {table} changed");
            let (was, now): (Value, Value) = (
                serde_json::from_str(payload).expect("payload"),
                serde_json::from_str(payload_now).expect("payload"),
            );
            let lost = differing(&was, &now)
                .into_iter()
                .filter(|key| was.get(key).is_some() && now.get(key).is_none())
                .collect::<Vec<_>>();
            assert!(lost.is_empty(), "fields of a row of {table} lost: {lost:?}");
            for key in ["id", "pool", "vm", "action"] {
                assert!(
                    was.get(key) == now.get(key),
                    "the field {key} of a row of {table} changed"
                );
            }
        }
    }
    assert_eq!(before.children, after.children, "batch members");
    println!(
        "upgrade: journal after the core opened it: format {} (was {}), {} VM operation(s), \
         {} resource operation(s), {} batch member(s); payloads byte for byte the same: {}",
        after.version,
        before.version,
        after.operations.len(),
        after.resources.len(),
        after.children,
        yes(before.operations == after.operations && before.resources == after.resources)
    );
}
