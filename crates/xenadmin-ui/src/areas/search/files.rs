//! Saved-search files and the report file: the Rune `search_files.rs`.
//!
//! Each of the three actions asks the front end for a path
//! (`Intent::ChooseFile`), then reads or writes on a worker thread whose
//! result `poll` collects. At most one file job runs at a time. A reply or a
//! result that arrives after the job was abandoned (the filters were
//! cleared, the report dialog was closed) is ignored: the area's `token`
//! changed meanwhile. The report is encoded before the prompt opens, so the
//! file holds the results as they were when the button was pressed.

use std::path::Path;
use std::sync::mpsc;

use xenadmin_core::SearchCatalog;

use super::{Dialog, Import, report};
use crate::files::{ReadError, read_regular, write_private};
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

/// Reads saved-search definitions: a regular file only, never through a
/// symbolic link, at most `SearchCatalog::MAX_BYTES`
/// ([`crate::files::read_regular`]), in the words of the Rune reader.
pub(super) fn read(path: &Path) -> Result<SearchCatalog, String> {
    let bytes =
        read_regular(path, SearchCatalog::MAX_BYTES).map_err(|error| match error {
            ReadError::NotRegular => "Choose a regular JSON file",
            ReadError::Open => "Cannot open search file",
            ReadError::Inspect => "Cannot inspect search file",
            ReadError::Read => "Cannot read search file",
            // What `SearchCatalog::decode` says of more bytes than it takes.
            ReadError::TooLarge => "Saved searches exceed 1 MiB",
        })?;
    SearchCatalog::decode(&bytes)
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
mod tests {
    use super::*;
    use crate::testing::TempDir;

    // Rune: search_files.rs, moved. What is about the file alone is tested
    // in `crate::files`; this is the reader's own wording and its limit.
    #[test]
    fn local_search_reader_is_bounded_and_refuses_symlinks_and_special_files() {
        let d = TempDir::new("search");
        let file = d.path().join("scope.json");
        std::fs::write(&file, SearchCatalog::default().encode().unwrap()).unwrap();
        assert!(read(&file).is_ok());
        let link = d.path().join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read(&link).unwrap_err(), "Choose a regular JSON file");
        assert_eq!(read(d.path()).unwrap_err(), "Choose a regular JSON file");
        assert_eq!(
            read(Path::new("/dev/null")).unwrap_err(),
            "Choose a regular JSON file"
        );
        assert_eq!(
            read(&d.path().join("missing.json")).unwrap_err(),
            "Cannot open search file"
        );
        std::fs::write(&file, vec![b' '; SearchCatalog::MAX_BYTES + 1]).unwrap();
        assert_eq!(read(&file).unwrap_err(), "Saved searches exceed 1 MiB");
        assert_eq!(
            SearchCatalog::decode(&vec![b' '; SearchCatalog::MAX_BYTES + 1]).unwrap_err(),
            "Saved searches exceed 1 MiB"
        );
    }
}
