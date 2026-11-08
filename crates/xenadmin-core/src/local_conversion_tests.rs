use super::*;
use serde_json::json;

#[test]
fn metadata_rejects_external_data_snapshots_and_wrong_formats() {
    let good = json!({"format":"qcow2","virtual-size":1048576,"format-specific":{"type":"qcow2","data":{"compat":"1.1","refcount-bits":16,"compression-type":"zlib"}}});
    assert_eq!(
        inspect_metadata(
            &serde_json::to_vec(&good).unwrap(),
            ConversionFormat::Qcow2,
            DiskImageFormat::Raw
        )
        .unwrap(),
        1048576
    );
    for value in [
        "backing-filename",
        "full-backing-filename",
        "backing-filename-format",
    ] {
        let mut v = good.clone();
        v[value] = json!("private/source");
        assert!(
            inspect_metadata(
                &serde_json::to_vec(&v).unwrap(),
                ConversionFormat::Qcow2,
                DiskImageFormat::Raw
            )
            .is_err()
        );
    }
    for key in ["data-file", "encrypt"] {
        let mut v = good.clone();
        v["format-specific"]["data"][key] = json!("private");
        assert!(
            inspect_metadata(
                &serde_json::to_vec(&v).unwrap(),
                ConversionFormat::Qcow2,
                DiskImageFormat::Raw
            )
            .is_err()
        );
    }
    let mut v = good;
    v["snapshots"] = json!([{"id":"1"}]);
    assert!(
        inspect_metadata(
            &serde_json::to_vec(&v).unwrap(),
            ConversionFormat::Qcow2,
            DiskImageFormat::Raw
        )
        .is_err()
    );
    let mut v = json!({"format":"vmdk","virtual-size":1048576,"format-specific":{"type":"vmdk","data":{"create-type":"monolithicSparse","parent-cid":4294967295u64,"extents":[{"filename":"/input/source","virtual-size":1048576}]}}});
    assert!(
        inspect_metadata(
            &serde_json::to_vec(&v).unwrap(),
            ConversionFormat::Vmdk,
            DiskImageFormat::Raw
        )
        .is_ok()
    );
    v["format-specific"]["data"]["extents"][0]["filename"] = json!("/etc/shadow");
    assert!(
        inspect_metadata(
            &serde_json::to_vec(&v).unwrap(),
            ConversionFormat::Vmdk,
            DiskImageFormat::Raw
        )
        .is_err()
    );
}
#[test]
fn staging_is_distinct_read_only_and_source_or_destination_changes_fail() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("original.raw");
    std::fs::write(&source, vec![7; 4096]).unwrap();
    let dest = d.path().join("new.raw");
    let (_, _, working) = prepare_paths(&source, &dest, uuid::Uuid::new_v4()).unwrap();
    let staged = working.join("source");
    let stop = AtomicBool::new(false);
    let (id, hash) = stage(&source, &staged, &stop).unwrap();
    assert_ne!(id.inode, source.metadata().unwrap().ino());
    assert_eq!(staged.metadata().unwrap().mode() & 0o777, 0o400);
    std::fs::write(&source, vec![9; 4096]).unwrap();
    verify_file(&staged, Some(&id), Some(&hash), &stop).unwrap();
    std::fs::remove_file(&staged).unwrap();
    std::fs::write(&staged, vec![7; 4096]).unwrap();
    assert!(verify_file(&staged, Some(&id), Some(&hash), &stop).is_err());
    std::fs::write(&dest, b"preserve").unwrap();
    assert!(prepare_paths(&source, &dest, uuid::Uuid::new_v4()).is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"preserve");
}
#[test]
fn worker_lock_is_exclusive_and_public_history_has_no_paths() {
    let d = tempfile::tempdir().unwrap();
    let lock1 = lock(d.path()).unwrap();
    assert!(lock(d.path()).is_err());
    drop(lock1);
    assert!(lock(d.path()).is_ok());
    let row = ConversionSummary {
        id: uuid::Uuid::new_v4(),
        source: ConversionFormat::Raw,
        output: DiskImageFormat::Vhd,
        virtual_size: Some(4096),
        state: ConversionState::Completed,
        detail: "Done".into(),
    };
    let value = serde_json::to_value(&row).unwrap();
    assert!(value.get("original").is_none());
    assert!(value.get("destination").is_none());
}
fn tools() -> ConversionTools {
    ConversionTools {
        qemu: std::env::var_os("XENADMIN_TEST_QEMU")
            .expect("Set XENADMIN_TEST_QEMU to the trusted project-local qemu-img build")
            .into(),
    }
}

#[test]
#[ignore = "Requires explicitly selected trusted local QEMU and working user-namespace sandbox"]
fn real_qemu_converts_generated_raw_and_vhd_without_touching_original_or_overwriting() {
    let d = tempfile::tempdir().unwrap();
    let original = d.path().join("original.raw");
    let mut data = vec![0; 2 * 1024 * 1024];
    data[..512].fill(37);
    data[1024 * 1024..1024 * 1024 + 512].fill(91);
    std::fs::write(&original, &data).unwrap();
    let manager = LocalConversions::new(d.path().join("state"));
    let cancel = AtomicBool::new(false);
    let review = manager
        .prepare(
            ConversionRequest {
                tools: tools(),
                source: original.clone(),
                source_format: ConversionFormat::Raw,
                output: DiskImageFormat::Vhd,
                destination: d.path().join("converted.vhd"),
            },
            &cancel,
            |_| {},
        )
        .unwrap();
    assert_eq!(review.summary().state, ConversionState::Ready);
    assert!(
        manager
            .prepare(
                ConversionRequest {
                    tools: tools(),
                    source: original.clone(),
                    source_format: ConversionFormat::Raw,
                    output: DiskImageFormat::Raw,
                    destination: d.path().join("second.raw")
                },
                &cancel,
                |_| {}
            )
            .is_err()
    );
    assert_eq!(
        review.convert(&cancel, |_| {}).unwrap().state,
        ConversionState::Completed
    );
    assert_eq!(std::fs::read(&original).unwrap(), data);
    let review = manager
        .prepare(
            ConversionRequest {
                tools: tools(),
                source: d.path().join("converted.vhd"),
                source_format: ConversionFormat::Vhd,
                output: DiskImageFormat::Raw,
                destination: d.path().join("roundtrip.raw"),
            },
            &cancel,
            |_| {},
        )
        .unwrap();
    review.convert(&cancel, |_| {}).unwrap();
    assert_eq!(std::fs::read(d.path().join("roundtrip.raw")).unwrap(), data);
    let review = manager
        .prepare(
            ConversionRequest {
                tools: tools(),
                source: original.clone(),
                source_format: ConversionFormat::Raw,
                output: DiskImageFormat::Raw,
                destination: d.path().join("raced.raw"),
            },
            &cancel,
            |_| {},
        )
        .unwrap();
    std::fs::write(d.path().join("raced.raw"), b"keep").unwrap();
    assert!(review.convert(&cancel, |_| {}).is_err());
    assert_eq!(std::fs::read(d.path().join("raced.raw")).unwrap(), b"keep");
    assert_eq!(manager.history().unwrap().len(), 3);
}

#[test]
#[ignore = "Requires explicitly selected trusted local QEMU and working user-namespace sandbox"]
fn real_qemu_self_contained_formats_roundtrip_and_backing_file_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let raw = d.path().join("source.raw");
    let mut content = vec![0; 2 * 1024 * 1024];
    content[512..1024].fill(73);
    std::fs::write(&raw, &content).unwrap();
    let output = d.path().join("generated");
    std::fs::create_dir(&output).unwrap();
    let cancel = AtomicBool::new(false);
    let binaries = Tools::inspect(&tools()).unwrap();
    for (index, (format, options)) in [
        (ConversionFormat::Qcow2, None),
        (ConversionFormat::Vmdk, None),
        (ConversionFormat::Vmdk, Some("subformat=streamOptimized")),
        (ConversionFormat::Vhdx, None),
        (ConversionFormat::Vhd, Some("subformat=fixed,force_size=on")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut args = vec![
            "convert".into(),
            "-f".into(),
            "raw".into(),
            "-O".into(),
            format.wire().into(),
        ];
        if let Some(options) = options {
            args.extend(["-o".into(), options.into()]);
        }
        args.extend(["/input/source".into(), "/output/image".into()]);
        binaries
            .run(
                args,
                Some(&raw),
                Some(&output),
                true,
                32 * 1024 * 1024,
                Duration::from_secs(30),
                &cancel,
            )
            .unwrap();
        let manager = LocalConversions::new(d.path().join(format!("state-{index}")));
        let destination = d.path().join(format!("roundtrip-{index}.raw"));
        let review = manager
            .prepare(
                ConversionRequest {
                    tools: tools(),
                    source: output.join("image"),
                    source_format: format,
                    output: DiskImageFormat::Raw,
                    destination: destination.clone(),
                },
                &cancel,
                |_| {},
            )
            .unwrap();
        review.convert(&cancel, |_| {}).unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), content);
        std::fs::remove_file(output.join("image")).unwrap();
    }
    binaries
        .run(
            vec![
                "create".into(),
                "-u".into(),
                "-f".into(),
                "qcow2".into(),
                "-F".into(),
                "raw".into(),
                "-b".into(),
                "/private/unselected-base.raw".into(),
                "/output/image".into(),
                "2M".into(),
            ],
            None,
            Some(&output),
            true,
            32 * 1024 * 1024,
            Duration::from_secs(30),
            &cancel,
        )
        .unwrap();
    let manager = LocalConversions::new(d.path().join("backing-state"));
    let rejected = manager.prepare(
        ConversionRequest {
            tools: tools(),
            source: output.join("image"),
            source_format: ConversionFormat::Qcow2,
            output: DiskImageFormat::Raw,
            destination: d.path().join("must-not-exist.raw"),
        },
        &cancel,
        |_| {},
    );
    assert!(matches!(rejected,Err(ref message) if message.contains("Backing")));
    assert!(!d.path().join("must-not-exist.raw").exists());
}

#[test]
#[ignore = "Requires working user-namespace sandbox and trusted system utilities"]
fn sandbox_hides_unselected_files_and_environment_and_bounds_process_output() {
    let d = tempfile::tempdir().unwrap();
    let secret = d.path().join("unselected-canary");
    std::fs::write(&secret, b"synthetic private canary").unwrap();
    let cancel = AtomicBool::new(false);
    let mut tool = Tools {
        qemu: process::Executable::inspect(Path::new("/usr/bin/cat")).unwrap(),
        bwrap: process::Executable::inspect(Path::new("/usr/bin/bwrap")).unwrap(),
        prlimit: process::Executable::inspect(Path::new("/usr/bin/prlimit")).unwrap(),
    };
    assert!(
        tool.run(
            vec![secret.display().to_string()],
            None,
            None,
            false,
            0,
            Duration::from_secs(5),
            &cancel
        )
        .is_err()
    );
    assert_eq!(
        tool.run(
            vec!["/input/source".into()],
            Some(&secret),
            None,
            false,
            0,
            Duration::from_secs(5),
            &cancel
        )
        .unwrap(),
        b"synthetic private canary"
    );
    tool.qemu = process::Executable::inspect(Path::new("/usr/bin/env")).unwrap();
    let environment = String::from_utf8(
        tool.run(
            vec![],
            None,
            None,
            false,
            0,
            Duration::from_secs(5),
            &cancel,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!environment.contains("HOME="));
    assert!(!environment.contains("DISPLAY="));
    assert!(!environment.contains("DBUS"));
    assert!(!environment.contains("SSH"));
    tool.qemu = process::Executable::inspect(Path::new("/usr/bin/yes")).unwrap();
    assert!(
        tool.run(
            vec![],
            None,
            None,
            false,
            0,
            Duration::from_secs(5),
            &cancel
        )
        .is_err()
    );
}

#[test]
#[ignore = "Requires explicitly selected trusted local QEMU and working user-namespace sandbox"]
fn cancellation_and_abandoned_review_recovery_do_not_resume_or_remove_unrelated_files() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("source.raw");
    std::fs::write(&source, vec![3; 4096]).unwrap();
    let manager = LocalConversions::new(d.path().join("state"));
    let cancel = AtomicBool::new(false);
    let review = manager
        .prepare(
            ConversionRequest {
                tools: tools(),
                source: source.clone(),
                source_format: ConversionFormat::Raw,
                output: DiskImageFormat::Raw,
                destination: d.path().join("cancelled.raw"),
            },
            &cancel,
            |_| {},
        )
        .unwrap();
    let stopped = review.convert(&cancel, |row| {
        if row.state == ConversionState::Converting {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert!(stopped.is_err());
    assert!(!d.path().join("cancelled.raw").exists());
    assert_eq!(std::fs::read(&source).unwrap(), vec![3; 4096]);
    cancel.store(false, Ordering::Relaxed);
    let review = manager
        .prepare(
            ConversionRequest {
                tools: tools(),
                source,
                source_format: ConversionFormat::Raw,
                output: DiskImageFormat::Raw,
                destination: d.path().join("interrupted.raw"),
            },
            &cancel,
            |_| {},
        )
        .unwrap();
    let id = review.summary().id;
    let working = review.record.working.clone();
    let original_record = review.record.clone();
    let record_path = manager.directory.join(format!("{id}.json"));
    for changed in 0..3 {
        let mut corrupt = original_record.clone();
        match changed {
            0 => corrupt.working = d.path().join("unrelated"),
            1 => corrupt.destination = d.path().join("elsewhere").join("image.raw"),
            _ => corrupt.summary.state = ConversionState::Completed,
        }
        save(&manager.directory, &corrupt).unwrap();
        assert!(load(&record_path).is_err());
        assert!(manager.discard_temporary_files(id).is_err());
        assert!(working.join("source").exists());
    }
    save(&manager.directory, &original_record).unwrap();
    drop(review);
    std::fs::write(working.join("unattributed"), b"preserve").unwrap();
    let rows = manager.history().unwrap();
    assert!(
        rows.iter()
            .any(|r| r.id == id && r.state == ConversionState::Interrupted)
    );
    manager.discard_temporary_files(id).unwrap();
    assert_eq!(
        std::fs::read(working.join("unattributed")).unwrap(),
        b"preserve"
    );
    assert!(!working.join("source").exists());
    assert!(!d.path().join("interrupted.raw").exists());
}

#[test]
#[ignore = "Requires trusted C compiler and working user-namespace sandbox"]
fn sandbox_closes_inherited_descriptors_and_cancellation_terminates_new_session_payload() {
    use std::os::fd::AsRawFd;
    let d = tempfile::tempdir().unwrap();
    let code = d.path().join("probe.c");
    let executable = d.path().join("probe");
    std::fs::write(
        &code,
        br#"
#include <fcntl.h>
#include <unistd.h>
#include <sys/file.h>
#include <stdlib.h>
#include <errno.h>
#include <string.h>
int main(int argc,char**argv) {
 if(argc==2) { errno=0; return fcntl(atoi(argv[1]),F_GETFD)==-1 && errno==EBADF ? 0 : 11; }
 if(argc==3) { errno=0; return fork()==-1 && errno==EPERM ? 0 : 12; }
 int f=open("/output/held",O_CREAT|O_RDWR,0600); if(f<0||flock(f,LOCK_EX))return 20;
 int ready=open("/output/ready",O_CREAT|O_WRONLY,0600); if(ready<0)return 21;
 if(write(ready,"ready",5)!=5)return 22; close(ready);
 for(;;)pause();
}
"#,
    )
    .unwrap();
    assert!(
        std::process::Command::new("/usr/bin/cc")
            .args(["-O2", "-o"])
            .arg(&executable)
            .arg(&code)
            .status()
            .unwrap()
            .success()
    );
    let binaries = Tools {
        qemu: process::Executable::inspect(&executable).unwrap(),
        bwrap: process::Executable::inspect(Path::new("/usr/bin/bwrap")).unwrap(),
        prlimit: process::Executable::inspect(Path::new("/usr/bin/prlimit")).unwrap(),
    };
    let canary = d.path().join("canary");
    std::fs::write(&canary, b"synthetic descriptor canary").unwrap();
    let file = File::open(canary).unwrap();
    let inherited = rustix::io::fcntl_dupfd_cloexec(&file, 100).unwrap();
    rustix::io::fcntl_setfd(&inherited, rustix::io::FdFlags::empty()).unwrap();
    binaries
        .run(
            vec![inherited.as_raw_fd().to_string()],
            None,
            None,
            false,
            0,
            Duration::from_secs(5),
            &AtomicBool::new(false),
        )
        .unwrap();
    binaries
        .run(
            vec!["fork".into(), "probe".into()],
            None,
            None,
            false,
            0,
            Duration::from_secs(5),
            &AtomicBool::new(false),
        )
        .unwrap();
    drop(inherited);
    let output = d.path().join("output");
    std::fs::create_dir(&output).unwrap();
    let child_output = output.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let child_cancel = cancel.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = binaries.run(
            vec![],
            None,
            Some(&child_output),
            true,
            1024 * 1024,
            Duration::from_secs(10),
            &child_cancel,
        );
        sender.send(result).unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::fs::read(output.join("ready")).ok().as_deref() != Some(b"ready") {
        assert!(
            std::time::Instant::now() < deadline,
            "sandbox payload did not reach its readiness gate"
        );
        std::thread::yield_now();
    }
    let held = File::open(output.join("held")).unwrap();
    assert!(
        rustix::fs::flock(&held, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err()
    );
    cancel.store(true, Ordering::Relaxed);
    assert!(
        receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap_err()
            .contains("cancelled")
    );
    rustix::fs::flock(&held, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .expect("cancelled new-session payload must release its held file lock");
}

#[test]
fn history_limit_is_explicit_and_corrupted_private_identity_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    for i in 0..1025 {
        std::fs::write(d.path().join(format!("entry-{i}")), b"").unwrap();
    }
    assert!(history(d.path()).unwrap_err().contains("1,024"));
    let d = tempfile::tempdir().unwrap();
    let id = uuid::Uuid::new_v4();
    let path = d.path().join(format!("{id}.json"));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    f.write_all(b"{\"summary\":{}}").unwrap();
    assert!(load(&path).is_err());
}
