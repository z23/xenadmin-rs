//! Saved-search files and the report file: the Rune `search_files.rs`.
//!
//! Each of the three actions asks the front end for a path
//! (`Intent::ChooseFile`), then reads or writes on a worker thread whose
//! result `poll` collects. At most one file job runs at a time. A reply or a
//! result that arrives after the job was abandoned (the filters were
//! cleared, the report dialog was closed) is ignored: the area's `token`
//! changed meanwhile. The report is encoded before the prompt opens, so the
//! file holds the results as they were when the button was pressed.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;

use xenadmin_core::SearchCatalog;

use super::{Dialog, Import, report};
use crate::prelude::*;

/// Why a file command or button is disabled while a job runs.
pub const BUSY: &str = "A search file is already being read or written.";

#[derive(Default)]
pub struct Pending {
    pub busy: bool,
    receiver: Option<mpsc::Receiver<Result<Outcome, String>>>,
    job: Option<Job>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    Import,
    Export,
    Report,
}

enum Outcome {
    Import(SearchCatalog),
    Export,
    ExportResults,
}

fn begin(m: &mut Model, job: Job) -> Option<uuid::Uuid> {
    let state = &mut m.areas.search;
    if state.files.busy {
        return None;
    }
    state.files.busy = true;
    state.files.job = Some(job);
    state.sheet.notice = None;
    state.sheet.error = None;
    if let Some(Dialog::Report(panel)) = &mut state.dialog {
        panel.notice = None;
        panel.error = None;
    }
    Some(state.token)
}

/// Whether a reply belongs to the job that asked: the token is the one the
/// prompt was opened with.
fn current(m: &Model, token: uuid::Uuid) -> bool {
    m.areas.search.token == token && m.areas.search.files.busy
}

pub fn import(m: &mut Model) {
    let Some(token) = begin(m, Job::Import) else {
        return;
    };
    m.choose_file(
        FileRequest::open("Import saved search definitions", move |m, path| {
            if !current(m, token) {
                return;
            }
            match path {
                Some(path) => worker(m, move || read(&path).map(Outcome::Import)),
                None => m.areas.search.files = Pending::default(),
            }
        })
        .filter("XenAdmin RS search JSON", ["json"]),
    );
}

pub fn export(m: &mut Model) {
    let Some(token) = begin(m, Job::Export) else {
        return;
    };
    let catalog = m.preferences.saved_searches.clone();
    m.choose_file(
        FileRequest::save(
            "Export saved search definitions",
            "xenadmin-searches.json",
            move |m, path| {
                if !current(m, token) {
                    return;
                }
                match path {
                    Some(path) => {
                        let catalog = catalog.clone();
                        worker(m, move || {
                            catalog
                                .encode()
                                .and_then(|bytes| write_private(&path, &bytes))
                                .map(|()| Outcome::Export)
                        })
                    }
                    None => m.areas.search.files = Pending::default(),
                }
            },
        )
        .filter("JSON", ["json"]),
    );
}

/// Captures the current results as a report and asks where to save it.
pub fn export_results(m: &mut Model) {
    if m.areas.search.files.busy {
        return;
    }
    super::sync(m);
    let bytes = match report::encode(&m.areas.search.sheet) {
        Ok(bytes) => bytes,
        Err(error) => {
            fail(m, Job::Report, error);
            return;
        }
    };
    let Some(token) = begin(m, Job::Report) else {
        return;
    };
    m.choose_file(
        FileRequest::save(
            "Export captured resource results",
            "xenadmin-resource-report.json",
            move |m, path| {
                if !current(m, token) {
                    return;
                }
                match path {
                    Some(path) => {
                        let bytes = bytes.clone();
                        worker(m, move || {
                            write_private(&path, &bytes).map(|()| Outcome::ExportResults)
                        })
                    }
                    None => m.areas.search.files = Pending::default(),
                }
            },
        )
        .filter("JSON", ["json"]),
    );
}

/// The identity of a file: device and inode.
fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

/// `O_NOFOLLOW | O_NONBLOCK` where this crate knows their values. The crate
/// has no `libc`; the check of the file's identity in [`read`] holds on
/// every target, and these flags add what the Rune reader had on the
/// targets the client is built for.
fn no_follow_flags() -> i32 {
    const O_NONBLOCK: i32 = 0o4000;
    if cfg!(all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "x86",
            target_arch = "riscv64"
        )
    )) {
        0o400000 | O_NONBLOCK
    } else if cfg!(all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "arm")
    )) {
        0o100000 | O_NONBLOCK
    } else {
        0
    }
}

/// Reads saved-search definitions: a regular file only, never through a
/// symbolic link, at most `SearchCatalog::MAX_BYTES`.
pub(super) fn read(path: &Path) -> Result<SearchCatalog, String> {
    use std::os::unix::fs::OpenOptionsExt;
    // The path itself must name a regular file: not a link, a directory or
    // a device. The opened file must then be that same file.
    let named = path
        .symlink_metadata()
        .map_err(|_| "Cannot open search file")?;
    if !named.is_file() {
        return Err("Choose a regular JSON file".into());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(no_follow_flags())
        .open(path)
        .map_err(|_| "Cannot open search file")?;
    let opened = file.metadata().map_err(|_| "Cannot inspect search file")?;
    if !opened.is_file() || identity(&opened) != identity(&named) {
        return Err("Choose a regular JSON file".into());
    }
    let mut bytes = Vec::new();
    file.take((SearchCatalog::MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read search file")?;
    SearchCatalog::decode(&bytes)
}

/// Writes an export: a private temporary file (mode 0600) renamed over the
/// destination. The Rune `activity_export::write_private`, which the search
/// files shared.
pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("Export exceeds 32 MiB; narrow the selection.".into());
    }
    if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
        return Err("Choose a regular file destination.".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = parent.join(format!(".xenadmin-search-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn worker(m: &mut Model, work: impl FnOnce() -> Result<Outcome, String> + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    let wake = m.waker.clone();
    let spawned = std::thread::Builder::new()
        .name("saved-search-file".into())
        .spawn(move || {
            let _ = tx.send(work());
            if let Some(wake) = wake {
                wake();
            }
        });
    if spawned.is_err() {
        let job = m.areas.search.files.job;
        m.areas.search.files = Pending::default();
        if let Some(job) = job {
            fail(m, job, "Cannot start saved search file worker".into());
        }
    } else {
        m.areas.search.files.receiver = Some(rx);
        // A fixture has no waker: ask for a poll.
        m.request_tick(0.05);
    }
}

/// Shows the failure of a job where the job was started: the import in its
/// own dialog, the report in the report dialog, the export on the tab.
fn fail(m: &mut Model, job: Job, error: String) {
    let state = &mut m.areas.search;
    match (job, &mut state.dialog) {
        (Job::Import, _) => {
            state.dialog = Some(Dialog::Import(Import {
                catalog: None,
                error: Some(error),
            }))
        }
        (Job::Report, Some(Dialog::Report(panel))) => panel.error = Some(error),
        _ => state.sheet.error = Some(error),
    }
}

/// Collects the result of the worker.
pub fn poll(m: &mut Model) -> bool {
    let result = {
        let Some(rx) = &m.areas.search.files.receiver else {
            return false;
        };
        match rx.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err("Search file worker stopped".into())),
        }
    };
    let Some(result) = result else {
        m.request_tick(0.05);
        return false;
    };
    let job = m.areas.search.files.job.unwrap_or(Job::Export);
    m.areas.search.files = Pending::default();
    let state = &mut m.areas.search;
    match result {
        Ok(Outcome::Import(catalog)) => {
            state.dialog = Some(Dialog::Import(Import {
                catalog: Some(catalog),
                error: None,
            }))
        }
        Ok(Outcome::ExportResults) => {
            let notice =
                "Captured resource results exported; later inventory changes are not included";
            match &mut state.dialog {
                Some(Dialog::Report(panel)) => panel.notice = Some(notice.into()),
                _ => state.sheet.notice = Some(notice.into()),
            }
        }
        Ok(Outcome::Export) => {
            state.sheet.notice =
                Some("Saved search definitions exported; no result inventory was included".into())
        }
        Err(error) => fail(m, job, error),
    }
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A private directory under the system's temporary directory, removed
    /// on drop (the crate has no `tempfile`).
    pub(crate) struct Scratch(pub std::path::PathBuf);
    impl Scratch {
        pub(crate) fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("xenadmin-search-ui-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // Rune: search_files.rs, moved.
    #[test]
    fn local_search_reader_is_bounded_and_refuses_symlinks_and_special_files() {
        let d = Scratch::new();
        let file = d.path().join("scope.json");
        std::fs::write(&file, SearchCatalog::default().encode().unwrap()).unwrap();
        assert!(read(&file).is_ok());
        let link = d.path().join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read(&link).is_err());
        assert!(read(d.path()).is_err());
        assert!(read(Path::new("/dev/null")).is_err());
        std::fs::write(&file, vec![b' '; SearchCatalog::MAX_BYTES + 1]).unwrap();
        assert!(read(&file).is_err());
    }

    #[test]
    fn exports_are_private_files_and_refuse_a_destination_that_is_not_a_regular_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = Scratch::new();
        let file = d.path().join("out.json");
        write_private(&file, b"{}").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"{}");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_private(d.path(), b"{}").is_err());
        // Nothing temporary is left behind.
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }
}
