use super::*;
use std::{io::Read, path::Path, sync::mpsc};
#[derive(Default)]
pub(super) struct Pending {
    pub busy: bool,
    receiver: Option<mpsc::Receiver<Result<Outcome, String>>>,
}
enum Outcome {
    Import(SearchCatalog),
    Export,
}
fn begin(s: &mut State) -> Option<uuid::Uuid> {
    let w = edit(s);
    if w.files.busy {
        return None;
    }
    w.files.busy = true;
    w.notice = None;
    w.error = None;
    Some(w.token)
}
pub(super) fn import(s: &mut State, tx: &mut Tx) {
    let Some(token) = begin(s) else {
        return;
    };
    tx.open_file(
        rune::FileDialogOptions {
            title: "Import saved search definitions".into(),
            ..Default::default()
        }
        .filter("XenAdmin RS search JSON", &["json"]),
        move |s: &mut State, result, _| {
            if s.searches.as_ref().is_none_or(|w| w.token != token) {
                return;
            }
            match result {
                Ok(Some(path)) => worker(s, move || read(&path).map(Outcome::Import)),
                Ok(None) => edit(s).files.busy = false,
                Err(_) => {
                    edit(s).files.busy = false;
                    edit(s).error = Some("Cannot open search file picker".into());
                }
            }
        },
    );
}
pub(super) fn export(s: &mut State, tx: &mut Tx) {
    let Some(token) = begin(s) else {
        return;
    };
    let catalog = s.preferences.saved_searches.clone();
    tx.save_file(
        rune::FileDialogOptions {
            title: "Export saved search definitions".into(),
            file_name: Some("xenadmin-searches.json".into()),
            ..Default::default()
        }
        .filter("JSON", &["json"]),
        move |s: &mut State, result, _| {
            if s.searches.as_ref().is_none_or(|w| w.token != token) {
                return;
            }
            match result {
                Ok(Some(path)) => worker(s, move || {
                    catalog
                        .encode()
                        .and_then(|bytes| activity_export::write_private(&path, &bytes))
                        .map(|()| Outcome::Export)
                }),
                Ok(None) => edit(s).files.busy = false,
                Err(_) => {
                    edit(s).files.busy = false;
                    edit(s).error = Some("Cannot open search save picker".into());
                }
            }
        },
    );
}
fn read(path: &Path) -> Result<SearchCatalog, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "Cannot open search file")?;
    if !file
        .metadata()
        .map_err(|_| "Cannot inspect search file")?
        .is_file()
    {
        return Err("Choose a regular JSON file".into());
    }
    let mut bytes = Vec::new();
    file.take((SearchCatalog::MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read search file")?;
    SearchCatalog::decode(&bytes)
}
fn worker(s: &mut State, work: impl FnOnce() -> Result<Outcome, String> + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    let wake = s.waker.clone();
    if std::thread::Builder::new()
        .name("saved-search-file".into())
        .spawn(move || {
            let _ = tx.send(work());
            if let Some(wake) = wake {
                wake();
            }
        })
        .is_err()
    {
        edit(s).files.busy = false;
        edit(s).error = Some("Cannot start saved search file worker".into());
    } else {
        edit(s).files.receiver = Some(rx);
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(rx) = &edit(s).files.receiver else {
        return false;
    };
    let result = match rx.try_recv() {
        Ok(value) => value,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("Search file worker stopped".into()),
    };
    let w = edit(s);
    w.files = Default::default();
    match result {
        Ok(Outcome::Import(catalog)) => w.importing = Some(catalog),
        Ok(Outcome::Export) => {
            w.notice =
                Some("Saved search definitions exported; no result inventory was included".into())
        }
        Err(error) => w.error = Some(error),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_search_reader_is_bounded_and_refuses_symlinks_and_special_files() {
        let d = tempfile::tempdir().unwrap();
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
}
