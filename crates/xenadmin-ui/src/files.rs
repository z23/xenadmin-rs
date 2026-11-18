//! Files the client reads and writes for the user: exports, reports and
//! imported definitions.
//!
//! Both functions block on the disk: call them on a worker thread and
//! collect the result in your area's `poll` (`areas/search/files.rs` shows
//! the pattern), never from a handler.
//!
//! - [`write_private`] writes a file nobody else can read: a temporary file
//!   with mode 0600 beside the destination, renamed over it.
//! - [`read_regular`] reads a regular file up to a limit, and never through
//!   a symbolic link, a directory or a device.

use std::fmt;
use std::io::{Read, Write};
use std::path::Path;

/// The largest file [`write_private`] writes.
pub const MAX_WRITE_BYTES: usize = 32 * 1024 * 1024;

/// Writes `bytes` to `path` as a private file: a temporary file (mode
/// 0600) in the same directory, synced and renamed over the destination,
/// so a reader sees the old file or the whole new one. Refuses more than
/// [`MAX_WRITE_BYTES`] and a destination that exists and is not a regular
/// file. The temporary file is removed when anything fails. The error is
/// the text to show (the Rune `activity_export::write_private`).
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if bytes.len() > MAX_WRITE_BYTES {
        return Err("Export exceeds 32 MiB; narrow the selection.".into());
    }
    if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
        return Err("Choose a regular file destination.".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = parent.join(format!(".xenadmin-{}.tmp", uuid::Uuid::new_v4()));
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

/// Why [`read_regular`] did not return the file. `Display` gives a text
/// that names no kind of file; an area that knows what it reads says it in
/// its own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The path names a symbolic link, a directory, a device or anything
    /// else that is not a regular file, or the file was replaced between
    /// the check and the opening.
    NotRegular,
    /// The file does not exist or cannot be opened.
    Open,
    /// The opened file cannot be inspected.
    Inspect,
    /// Reading failed part way.
    Read,
    /// The file holds more than the limit.
    TooLarge,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRegular => "Choose a regular file",
            Self::Open => "Cannot open the file",
            Self::Inspect => "Cannot inspect the file",
            Self::Read => "Cannot read the file",
            Self::TooLarge => "The file is too large",
        })
    }
}

impl std::error::Error for ReadError {}

/// The identity of a file: device and inode.
fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

/// Reads a file of at most `max_bytes`. The path itself must name a
/// regular file: it is opened without following a link (`O_NOFOLLOW`) and
/// without waiting on a pipe or a device (`O_NONBLOCK`), and the opened
/// file must be the one the path named. No more than `max_bytes + 1` bytes
/// are ever read.
pub fn read_regular(path: &Path, max_bytes: usize) -> Result<Vec<u8>, ReadError> {
    use std::os::unix::fs::OpenOptionsExt;
    let named = path.symlink_metadata().map_err(|_| ReadError::Open)?;
    if !named.is_file() {
        return Err(ReadError::NotRegular);
    }
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags.bits() as i32)
        .open(path)
        .map_err(|_| ReadError::Open)?;
    let opened = file.metadata().map_err(|_| ReadError::Inspect)?;
    if !opened.is_file() || identity(&opened) != identity(&named) {
        return Err(ReadError::NotRegular);
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadError::Read)?;
    if bytes.len() > max_bytes {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    // Rune: search_files.rs `local_search_reader_is_bounded_and_refuses_symlinks_and_special_files`,
    // the part that is about the file and not about its content.
    #[test]
    fn the_reader_is_bounded_and_refuses_links_directories_and_devices() {
        let d = TempDir::new("files");
        let file = d.path().join("scope.json");
        std::fs::write(&file, b"{}").unwrap();
        assert_eq!(read_regular(&file, 2).as_deref(), Ok(&b"{}"[..]));
        assert_eq!(read_regular(&file, 1), Err(ReadError::TooLarge));
        let link = d.path().join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_regular(&link, 64), Err(ReadError::NotRegular));
        assert_eq!(read_regular(d.path(), 64), Err(ReadError::NotRegular));
        assert_eq!(
            read_regular(Path::new("/dev/null"), 64),
            Err(ReadError::NotRegular)
        );
        assert_eq!(
            read_regular(&d.path().join("missing.json"), 64),
            Err(ReadError::Open)
        );
        // A named pipe is refused before anything waits on it.
        let pipe = d.path().join("pipe");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &pipe,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        assert_eq!(read_regular(&pipe, 64), Err(ReadError::NotRegular));
        // An empty file is a file.
        std::fs::write(&file, b"").unwrap();
        assert_eq!(read_regular(&file, 0), Ok(Vec::new()));
        assert_eq!(ReadError::TooLarge.to_string(), "The file is too large");
    }

    // Rune: activity_export.rs / search_files.rs, moved.
    #[test]
    fn a_private_file_has_mode_0600_and_replaces_only_a_regular_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new("files");
        let file = d.path().join("out.json");
        write_private(&file, b"{}").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"{}");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Written again: replaced as a whole, still private.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&file, b"[1]").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"[1]");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // A directory and a link are no destination, and the link's target
        // is not written through.
        assert!(write_private(d.path(), b"{}").is_err());
        let link = d.path().join("link.json");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(
            write_private(&link, b"through").unwrap_err(),
            "Choose a regular file destination."
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"[1]");
        // Nothing temporary is left behind.
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 2);
    }
}
