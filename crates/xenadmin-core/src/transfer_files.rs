use super::*;
use rustix::fs::{Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
impl Identity {
    pub(crate) fn from(m: &Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            size: m.len(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
        }
    }
    pub fn same_inode(&self, o: &Self) -> bool {
        self.device == o.device && self.inode == o.inode
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub identity: Identity,
}
fn canonical(path: &Path) -> Result<(PathBuf, Identity), Error> {
    if !path.is_absolute() {
        return Err(Error::TransferFile);
    }
    let name = path.file_name().ok_or(Error::TransferFile)?;
    let parent = path
        .parent()
        .ok_or(Error::TransferFile)?
        .canonicalize()
        .map_err(|_| Error::TransferFile)?;
    let m = parent.metadata().map_err(|_| Error::TransferFile)?;
    if !m.is_dir() {
        return Err(Error::TransferFile);
    }
    Ok((parent.join(name), Identity::from(&m)))
}
fn open(path: &Path, write: bool, create: bool) -> Result<File, Error> {
    let flags = if write {
        OFlags::WRONLY
    } else {
        OFlags::RDONLY
    } | OFlags::CLOEXEC
        | OFlags::NOFOLLOW
        | OFlags::NONBLOCK
        | if create {
            OFlags::CREATE | OFlags::EXCL
        } else {
            OFlags::empty()
        };
    let fd =
        rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR).map_err(|_| Error::TransferFile)?;
    let file = File::from(fd);
    if !file.metadata().map_err(|_| Error::TransferFile)?.is_file() {
        return Err(Error::TransferFile);
    }
    Ok(file)
}
pub(crate) fn select_source(path: &Path) -> Result<TransferFile, Error> {
    let (path, parent) = canonical(path)?;
    let mut file = open(&path, false, false)?;
    let identity = Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?);
    if !(2..=MAX_TRANSFER_BYTES).contains(&identity.size) {
        return Err(Error::TransferLimit);
    }
    let summary = super::archive::inspect(&mut file)?;
    if Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?) != identity {
        return Err(Error::TransferFileChanged);
    }
    Ok(TransferFile {
        path,
        identity: Some(identity),
        parent,
        manifest: Some(summary),
        disk: None,
        range: None,
        ovf: None,
    })
}
pub(crate) fn select_destination(path: &Path) -> Result<TransferFile, Error> {
    let (path, parent) = canonical(path)?;
    if path.symlink_metadata().is_ok() {
        return Err(Error::TransferDestinationExists);
    }
    Ok(TransferFile {
        path,
        identity: None,
        parent,
        manifest: None,
        disk: None,
        range: None,
        ovf: None,
    })
}
pub(crate) fn parent_handle(file: &TransferFile) -> Result<File, Error> {
    let path = file.path.parent().ok_or(Error::TransferFile)?;
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| Error::TransferFile)?;
    let dir = File::from(fd);
    if !file.parent.same_inode(&Identity::from(
        &dir.metadata().map_err(|_| Error::TransferFile)?,
    )) {
        return Err(Error::TransferFileChanged);
    }
    Ok(dir)
}
fn open_in(dir: &File, path: &Path, write: bool, create: bool) -> Result<File, Error> {
    let flags = if write {
        OFlags::WRONLY
    } else {
        OFlags::RDONLY
    } | OFlags::NOFOLLOW
        | OFlags::CLOEXEC
        | OFlags::NONBLOCK
        | if create {
            OFlags::CREATE | OFlags::EXCL
        } else {
            OFlags::empty()
        };
    let file = File::from(
        rustix::fs::openat(
            dir,
            path.file_name().ok_or(Error::TransferFile)?,
            flags,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| Error::TransferFile)?,
    );
    if !file.metadata().map_err(|_| Error::TransferFile)?.is_file() {
        return Err(Error::TransferFile);
    }
    Ok(file)
}
pub(crate) fn source(file: &TransferFile) -> Result<File, Error> {
    let dir = parent_handle(file)?;
    let mut input = open_in(&dir, &file.path, false, false)?;
    if Some(Identity::from(
        &input.metadata().map_err(|_| Error::TransferFile)?,
    )) != file.identity
    {
        return Err(Error::TransferFileChanged);
    }
    if let Some(range) = &file.range {
        if range.length == 0
            || range
                .offset
                .checked_add(range.length)
                .is_none_or(|end| end > file.identity.as_ref().map_or(0, |i| i.size))
        {
            return Err(Error::TransferFileChanged);
        }
        input
            .seek(SeekFrom::Start(range.offset))
            .map_err(|_| Error::TransferFile)?;
    }
    Ok(input)
}
pub(crate) fn source_unchanged(file: &TransferFile, input: &File) -> Result<(), Error> {
    if Some(Identity::from(
        &input.metadata().map_err(|_| Error::TransferFile)?,
    )) != file.identity
    {
        Err(Error::TransferFileChanged)
    } else {
        Ok(())
    }
}
pub(crate) fn create_output(plan: &mut Plan, id: crate::OperationId) -> Result<(), Error> {
    let dir = parent_handle(&plan.file)?;
    if plan.file.path.symlink_metadata().is_ok() {
        return Err(Error::TransferDestinationExists);
    }
    let path = plan
        .file
        .path
        .parent()
        .ok_or(Error::TransferFile)?
        .join(format!(".xenadmin-transfer-{id}.part"));
    let file = open_in(&dir, &path, true, true)?;
    plan.temporary_identity = Some(Identity::from(
        &file.metadata().map_err(|_| Error::TransferFile)?,
    ));
    plan.temporary = Some(path);
    Ok(())
}
pub(crate) fn output(plan: &Plan) -> Result<File, Error> {
    let path = plan.temporary.as_ref().ok_or(Error::TransferFile)?;
    let dir = parent_handle(&plan.file)?;
    let file = open_in(&dir, path, true, false)?;
    let current = Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?);
    if !plan
        .temporary_identity
        .as_ref()
        .is_some_and(|i| i.same_inode(&current))
        || current.size != 0
    {
        return Err(Error::TransferFileChanged);
    }
    Ok(file)
}
pub(crate) fn source_receipt(
    selected: &TransferFile,
    file: &File,
    bytes: u64,
) -> Result<Receipt, Error> {
    source_unchanged(selected, file)?;
    if selected.size() != Some(bytes) {
        return Err(Error::TransferConflict);
    }
    Ok(Receipt {
        bytes,
        sha256: [0; 32],
        identity: Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?),
    })
}
pub(crate) fn receipt(file: &File, bytes: u64, sha256: [u8; 32]) -> Result<Receipt, Error> {
    let identity = Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?);
    if identity.size != bytes {
        return Err(Error::TransferFileChanged);
    }
    Ok(Receipt {
        bytes,
        sha256,
        identity,
    })
}
pub(crate) fn matches_receipt(path: &Path, r: &Receipt) -> Result<bool, Error> {
    let mut file = open(path, false, false)?;
    let current = Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?);
    if !r.identity.same_inode(&current) || current.size != r.bytes {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| Error::TransferFile)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(<[u8; 32]>::from(hash.finalize()) == r.sha256)
}
pub(crate) fn finalize(plan: &Plan) -> Result<(), Error> {
    let dir = parent_handle(&plan.file)?;
    let receipt = plan.stream_receipt.as_ref().ok_or(Error::TransferFile)?;
    if plan.file.path.symlink_metadata().is_ok() {
        if matches_receipt(&plan.file.path, receipt)? {
            return Ok(());
        }
        return Err(Error::TransferDestinationExists);
    }
    let temp = plan.temporary.as_ref().ok_or(Error::TransferFile)?;
    if !matches_receipt(temp, receipt)? {
        return Err(Error::TransferFileChanged);
    }
    rustix::fs::renameat_with(
        &dir,
        temp.file_name().ok_or(Error::TransferFile)?,
        &dir,
        plan.file.path.file_name().ok_or(Error::TransferFile)?,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|_| Error::TransferDestinationExists)?;
    File::open(plan.file.path.parent().ok_or(Error::TransferFile)?)
        .and_then(|d| d.sync_all())
        .map_err(|_| Error::TransferFile)?;
    Ok(())
}
pub(crate) fn cleanup(plan: &Plan) {
    if let (Some(path), Some(expected)) = (&plan.temporary, &plan.temporary_identity)
        && let Ok(m) = path.symlink_metadata()
        && m.is_file()
        && expected.same_inode(&Identity::from(&m))
    {
        let _ = std::fs::remove_file(path);
    }
}
pub(crate) fn rewind(file: &mut File) -> Result<(), Error> {
    file.rewind().map_err(|_| Error::TransferFile)
}

pub(crate) fn validate_export(plan: &Plan, source: &ExportSource) -> Result<(), Error> {
    let receipt = plan.stream_receipt.as_ref().ok_or(Error::TransferFile)?;
    let path = if plan.file.path.symlink_metadata().is_ok()
        && matches_receipt(&plan.file.path, receipt)?
    {
        &plan.file.path
    } else {
        plan.temporary.as_ref().ok_or(Error::TransferFile)?
    };
    let mut file = open(path, false, false)?;
    if !plan.temporary_identity.as_ref().is_some_and(|id| {
        file.metadata()
            .is_ok_and(|m| id.same_inode(&Identity::from(&m)))
    }) {
        return Err(Error::TransferFileChanged);
    }
    let summary = super::archive::inspect(&mut file)?;
    let expected = match source {
        ExportSource::Vm { vm, .. } => vm.uuid(),
        ExportSource::Snapshot { snapshot, .. } => snapshot.uuid(),
    };
    if summary.guests.len() != 1 || summary.guests[0].uuid != expected {
        return Err(Error::TransferConflict);
    }
    Ok(())
}

pub(crate) fn select_disk_source(
    path: &Path,
    format: crate::DiskImageFormat,
) -> Result<TransferFile, Error> {
    let (path, parent) = canonical(path)?;
    let mut file = open(&path, false, false)?;
    let identity = Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?);
    let disk = crate::disk_image::inspect(&mut file, format)?;
    if Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?) != identity {
        return Err(Error::TransferFileChanged);
    }
    Ok(TransferFile {
        path,
        parent,
        identity: Some(identity),
        manifest: None,
        disk: Some(disk),
        range: None,
        ovf: None,
    })
}

pub(crate) fn validate_disk_export(
    plan: &Plan,
    format: crate::DiskImageFormat,
    size: u64,
) -> Result<(), Error> {
    let receipt = plan.stream_receipt.as_ref().ok_or(Error::TransferFile)?;
    let path = if plan.file.path.symlink_metadata().is_ok()
        && matches_receipt(&plan.file.path, receipt)?
    {
        &plan.file.path
    } else {
        plan.temporary.as_ref().ok_or(Error::TransferFile)?
    };
    let mut file = open(path, false, false)?;
    if !receipt.identity.same_inode(&Identity::from(
        &file.metadata().map_err(|_| Error::TransferFile)?,
    )) {
        return Err(Error::TransferFileChanged);
    }
    let summary = crate::disk_image::inspect(&mut file, format)?;
    if summary.virtual_size != size {
        return Err(Error::TransferConflict);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileRange {
    pub offset: u64,
    pub length: u64,
}
pub(crate) fn select_plain(path: &Path) -> Result<TransferFile, Error> {
    let (path, parent) = canonical(path)?;
    let file = open(&path, false, false)?;
    let identity = Identity::from(&file.metadata().map_err(|_| Error::TransferFile)?);
    if identity.size == 0 || identity.size > MAX_TRANSFER_BYTES {
        return Err(Error::TransferLimit);
    }
    Ok(TransferFile {
        path,
        parent,
        identity: Some(identity),
        manifest: None,
        disk: None,
        range: None,
        ovf: None,
    })
}
