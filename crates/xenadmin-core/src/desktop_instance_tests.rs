use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream as StdStream,
    sync::mpsc,
};
fn socket(path: &Path) -> PathBuf {
    path.join(SOCKET)
}
#[test]
fn desktop_owner_receives_private_activation_and_releases_lock_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = DesktopInstance::acquire(dir.path().into())
        .unwrap()
        .unwrap();
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_none()
    );
    let (tx, rx) = mpsc::channel();
    owner
        .start_activation(Arc::new(move |token| {
            tx.send(token).unwrap();
        }))
        .unwrap();
    assert_eq!(
        fs::metadata(socket(dir.path())).unwrap().mode() & 0o777,
        0o600
    );
    activate_desktop(dir.path(), Some("synthetic-launch-token".into())).unwrap();
    assert_eq!(
        rx.recv_timeout(TIMEOUT).unwrap(),
        Some("synthetic-launch-token".into())
    );
    activate_desktop(dir.path(), None).unwrap();
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), None);
    assert!(owner.start_activation(Arc::new(|_| {})).is_err());
    drop(owner);
    assert!(!socket(dir.path()).exists());
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_some()
    );
}
#[test]
fn desktop_activation_reclaims_only_stale_owned_socket_and_preserves_replacements() {
    let dir = tempfile::tempdir().unwrap();
    let target = socket(dir.path());
    let old = std::os::unix::net::UnixListener::bind(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    drop(old);
    let mut owner = DesktopInstance::acquire(dir.path().into())
        .unwrap()
        .unwrap();
    owner.start_activation(Arc::new(|_| {})).unwrap();
    fs::remove_file(&target).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&target).unwrap();
    let inode = fs::symlink_metadata(&target).unwrap().ino();
    drop(owner);
    assert_eq!(fs::symlink_metadata(&target).unwrap().ino(), inode);
    drop(replacement);
    fs::remove_file(&target).unwrap();
    fs::write(&target, b"retained file").unwrap();
    let mut owner = DesktopInstance::acquire(dir.path().into())
        .unwrap()
        .unwrap();
    assert!(owner.start_activation(Arc::new(|_| {})).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"retained file");
}
#[test]
fn desktop_owner_drop_unlocks_even_while_an_inherited_description_remains_open() {
    let dir = tempfile::tempdir().unwrap();
    let owner = DesktopInstance::acquire(dir.path().into())
        .unwrap()
        .unwrap();
    // A duplicate shares the same open file description, as an inherited fd does between
    // fork and CLOEXEC. No sleeps or actual subprocess scheduling are needed for this race.
    let inherited = owner.file.try_clone().unwrap();
    drop(owner);
    let next = DesktopInstance::acquire(dir.path().into()).unwrap();
    assert!(next.is_some());
    drop(inherited);
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_none()
    );
}
#[test]
fn desktop_ownership_refuses_symlink_directory_or_lock_and_hardlinked_lock() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    symlink(&real, &link).unwrap();
    assert!(DesktopInstance::acquire(link).is_err());
    let lock = real.join("desktop.lock");
    let unrelated = dir.path().join("keep");
    fs::write(&unrelated, b"owned data").unwrap();
    symlink(&unrelated, &lock).unwrap();
    assert!(DesktopInstance::acquire(real.clone()).is_err());
    assert_eq!(fs::read(&unrelated).unwrap(), b"owned data");
    fs::remove_file(&lock).unwrap();
    fs::hard_link(&unrelated, &lock).unwrap();
    assert!(DesktopInstance::acquire(real).is_err());
}
#[test]
fn desktop_activation_rejects_host_commands_invalid_versions_and_unbounded_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = DesktopInstance::acquire(dir.path().into())
        .unwrap()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    owner
        .start_activation(Arc::new(move |_| {
            tx.send(()).unwrap();
        }))
        .unwrap();
    let payloads = [
        serde_json::json!({"version":2,"command":"activate","token":null}),
        serde_json::json!({"version":1,"command":"host.reboot","token":null}),
        serde_json::json!({"version":1,"command":"activate","token":"a\nb"}),
        serde_json::json!({"version":1,"command":"activate","token":"x".repeat(4097)}),
        serde_json::json!({"version":1,"command":"activate","token":null,"vm":"foreign"}),
    ];
    for p in payloads {
        let mut stream = StdStream::connect(socket(dir.path())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = serde_json::to_vec(&p).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).unwrap();
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
        assert!(response.is_empty());
    }
    let mut oversized = StdStream::connect(socket(dir.path())).unwrap();
    oversized
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut frame = vec![b'x'; LIMIT + 100];
    frame.push(b'\n');
    let _ = oversized.write_all(&frame);
    let mut response = Vec::new();
    let _ = oversized.read_to_end(&mut response);
    assert!(response.is_empty());
    assert!(rx.try_recv().is_err());
    assert!(activate_desktop(dir.path(), Some("x".repeat(4097))).is_err());
    assert!(rx.try_recv().is_err());
    let mut slow = StdStream::connect(socket(dir.path())).unwrap();
    slow.write_all(b"{").unwrap();
    activate_desktop(dir.path(), None).unwrap();
    assert!(rx.recv_timeout(TIMEOUT).is_ok());
}
#[test]
fn desktop_activation_waits_for_initial_listener_but_does_not_accept_false_ack() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = DesktopInstance::acquire(dir.path().into())
        .unwrap()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        owner
            .start_activation(Arc::new(move |_| {
                tx.send(()).unwrap();
            }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(250));
    });
    activate_desktop(dir.path(), None).unwrap();
    assert!(rx.recv_timeout(TIMEOUT).is_ok());
    worker.join().unwrap();
    let listener = std::os::unix::net::UnixListener::bind(socket(dir.path())).unwrap();
    fs::set_permissions(socket(dir.path()), fs::Permissions::from_mode(0o600)).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0; 1024];
        let _ = stream.read(&mut buffer);
        stream
            .write_all(b"{\"version\":1,\"accepted\":false}\n")
            .unwrap();
    });
    assert!(activate_desktop(dir.path(), None).is_err());
    worker.join().unwrap();
}
#[test]
fn activation_child_helper() {
    let Some(path) = std::env::var_os("XENADMIN_TEST_ACTIVATION_DIR").map(PathBuf::from) else {
        return;
    };
    let mut owner = DesktopInstance::acquire(path.clone()).unwrap().unwrap();
    let (tx, rx) = mpsc::channel();
    owner
        .start_activation(Arc::new(move |token| {
            tx.send(token).unwrap();
        }))
        .unwrap();
    let token = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let retained = std::env::var_os("XENADMIN_TEST_RETAIN_LOCK").is_some();
    if retained {
        owner.retain_lock_until_exit().unwrap();
        owner.retain_lock_until_exit().unwrap();
    }
    drop(owner);
    fs::write(path.join("received"), token.unwrap_or_default()).unwrap();
    if retained {
        let mut release = [0u8; 1];
        std::io::stdin().read_exact(&mut release).unwrap();
    }
}
#[test]
fn another_process_activates_owner_without_taking_its_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop_instance::tests::activation_child_helper",
            "--nocapture",
        ])
        .env("XENADMIN_TEST_ACTIVATION_DIR", dir.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while !socket(dir.path()).exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_none()
    );
    activate_desktop(dir.path(), Some("cross-process-fixture".into())).unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(dir.path().join("received")).unwrap(),
        "cross-process-fixture"
    );
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_some()
    );
}

#[test]
fn separate_state_directories_keep_activation_ownership_independent() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut first = DesktopInstance::acquire(a.path().into()).unwrap().unwrap();
    let mut second = DesktopInstance::acquire(b.path().into()).unwrap().unwrap();
    let (at, ar) = mpsc::channel();
    let (bt, br) = mpsc::channel();
    first
        .start_activation(Arc::new(move |_| {
            at.send(()).unwrap();
        }))
        .unwrap();
    second
        .start_activation(Arc::new(move |_| {
            bt.send(()).unwrap();
        }))
        .unwrap();
    activate_desktop(b.path(), None).unwrap();
    assert!(br.recv_timeout(TIMEOUT).is_ok());
    assert!(ar.try_recv().is_err());
    drop(second);
    assert!(DesktopInstance::acquire(a.path().into()).unwrap().is_none());
    activate_desktop(a.path(), None).unwrap();
    assert!(ar.recv_timeout(TIMEOUT).is_ok());
}

#[test]
fn desktop_lock_remains_held_after_guard_drop_until_process_exit() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop_instance::tests::activation_child_helper",
            "--nocapture",
        ])
        .env("XENADMIN_TEST_ACTIVATION_DIR", dir.path())
        .env("XENADMIN_TEST_RETAIN_LOCK", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while !socket(dir.path()).exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    activate_desktop(dir.path(), None).unwrap();
    while !dir.path().join("received").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !socket(dir.path()).exists(),
        "listener guard has already dropped"
    );
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_none(),
        "remaining backend threads must keep process ownership"
    );
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(
        DesktopInstance::acquire(dir.path().into())
            .unwrap()
            .is_some()
    );
}
