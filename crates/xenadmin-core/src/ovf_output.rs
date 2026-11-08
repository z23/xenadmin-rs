use super::*;
use crate::{
    OperationId,
    transfer::{self, Plan, files},
};
use rustix::fs::{AtFlags, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Directory {
    pub path: PathBuf,
    pub identity: files::Identity,
}
pub(crate) fn create(destination: &TransferFile, id: OperationId) -> Result<Directory, Error> {
    let parent = files::parent_handle(destination)?;
    if destination.path.symlink_metadata().is_ok() {
        return Err(Error::TransferDestinationExists);
    }
    let name = format!(".xenadmin-ovf-{id}.part");
    rustix::fs::mkdirat(&parent, &name, Mode::RUSR | Mode::WUSR | Mode::XUSR)
        .map_err(|_| Error::TransferFile)?;
    let dir = File::from(
        rustix::fs::openat(
            &parent,
            &name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| Error::TransferFile)?,
    );
    let identity = files::Identity::from(&dir.metadata().map_err(|_| Error::TransferFile)?);
    parent.sync_all().map_err(|_| Error::TransferFile)?;
    Ok(Directory {
        path: destination
            .path
            .parent()
            .ok_or(Error::TransferFile)?
            .join(name),
        identity,
    })
}
fn handle(directory: &Directory, destination: &TransferFile) -> Result<File, Error> {
    let parent = files::parent_handle(destination)?;
    for path in [&directory.path, &destination.path] {
        if let Ok(fd) = rustix::fs::openat(
            &parent,
            path.file_name().ok_or(Error::TransferFile)?,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            let f = File::from(fd);
            if directory.identity.same_inode(&files::Identity::from(
                &f.metadata().map_err(|_| Error::TransferFile)?,
            )) {
                return Ok(f);
            }
        }
    }
    Err(Error::TransferFileChanged)
}
fn open(dir: &File, name: &str, write: bool) -> Result<File, Error> {
    super::descriptor::safe_name(name)?;
    if name.contains('/') {
        return Err(Error::TransferFile);
    }
    let flags = OFlags::CLOEXEC
        | OFlags::NOFOLLOW
        | OFlags::NONBLOCK
        | if write {
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL
        } else {
            OFlags::RDONLY
        };
    let f = File::from(
        rustix::fs::openat(dir, name, flags, Mode::RUSR | Mode::WUSR)
            .map_err(|_| Error::TransferFile)?,
    );
    if !f.metadata().map_err(|_| Error::TransferFile)?.is_file() {
        return Err(Error::TransferFile);
    }
    Ok(f)
}
fn text_file(dir: &File, name: &str, bytes: &[u8]) -> Result<(), Error> {
    match open(dir, name, true) {
        Ok(mut f) => {
            f.write_all(bytes).map_err(|_| Error::TransferFile)?;
            f.sync_all().map_err(|_| Error::TransferFile)
        }
        Err(e) => {
            let f = open(dir, name, false)?;
            if f.metadata().map_err(|_| Error::TransferFile)?.len() != bytes.len() as u64 {
                return Err(e);
            }
            let mut actual = Vec::new();
            f.take(bytes.len() as u64 + 1)
                .read_to_end(&mut actual)
                .map_err(|_| Error::TransferFile)?;
            if actual == bytes {
                Ok(())
            } else {
                Err(Error::TransferFileChanged)
            }
        }
    }
}
fn cancelled(c: &AtomicBool) -> Result<(), Error> {
    if c.load(Ordering::Relaxed) {
        Err(Error::Closed)
    } else {
        Ok(())
    }
}
fn hash_file(
    dir: &File,
    name: &str,
    receipt: &files::Receipt,
    c: &AtomicBool,
) -> Result<(), Error> {
    let mut input = open(dir, name, false)?;
    let initial = files::Identity::from(&input.metadata().map_err(|_| Error::TransferFile)?);
    if !initial.same_inode(&receipt.identity) || initial.size != receipt.bytes {
        return Err(Error::TransferFileChanged);
    }
    let mut hash = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        cancelled(c)?;
        let n = input.read(&mut buf).map_err(|_| Error::TransferFile)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if <[u8; 32]>::from(hash.finalize()) != receipt.sha256
        || files::Identity::from(&input.metadata().map_err(|_| Error::TransferFile)?) != initial
    {
        return Err(Error::TransferFileChanged);
    }
    Ok(())
}
fn header(name: &str, size: u64) -> Result<[u8; 512], Error> {
    if name.len() > 100 || size > transfer::MAX_TRANSFER_BYTES {
        return Err(Error::TransferLimit);
    }
    let mut h = [0; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000600\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    if size < 8 * 1024 * 1024 * 1024 {
        h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    } else {
        h[124] = 0x80;
        h[128..136].copy_from_slice(&size.to_be_bytes());
    }
    h[136..148].copy_from_slice(b"00000000000\0");
    h[148..156].fill(b' ');
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    let sum = h.iter().map(|b| u64::from(*b)).sum::<u64>();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    Ok(h)
}
struct Writer {
    file: File,
    hash: Sha256,
    bytes: u64,
}
impl Writer {
    fn write(&mut self, b: &[u8]) -> Result<(), Error> {
        self.bytes = self
            .bytes
            .checked_add(b.len() as u64)
            .filter(|n| *n <= transfer::MAX_TRANSFER_BYTES)
            .ok_or(Error::TransferLimit)?;
        self.file.write_all(b).map_err(|_| Error::TransferFile)?;
        self.hash.update(b);
        Ok(())
    }
    fn member(&mut self, dir: &File, name: &str, size: u64, c: &AtomicBool) -> Result<(), Error> {
        self.write(&header(name, size)?)?;
        let mut input = open(dir, name, false)?;
        let before = files::Identity::from(&input.metadata().map_err(|_| Error::TransferFile)?);
        if before.size != size {
            return Err(Error::TransferFileChanged);
        }
        let mut count = 0u64;
        let mut buf = [0; 65536];
        loop {
            cancelled(c)?;
            let n = input.read(&mut buf).map_err(|_| Error::TransferFile)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > size {
                return Err(Error::TransferFileChanged);
            }
            self.write(&buf[..n])?;
        }
        if count != size
            || files::Identity::from(&input.metadata().map_err(|_| Error::TransferFile)?) != before
        {
            return Err(Error::TransferFileChanged);
        }
        let pad = (512 - size % 512) % 512;
        self.write(&vec![0; pad as usize])
    }
}
pub(crate) fn publish(
    plan: &Plan,
    format: OvfPackageFormat,
    cancel: &AtomicBool,
) -> Result<Option<files::Receipt>, Error> {
    let export = plan.ovf_export.as_ref().ok_or(Error::TransferConflict)?;
    let dir = handle(&export.directory, &plan.file)?;
    let mut sizes = Vec::new();
    let mut manifest = String::new();
    let mut names = Vec::new();
    for (i, step) in export.steps.iter().enumerate() {
        cancelled(cancel)?;
        if !step.complete {
            return Err(Error::TransferConflict);
        }
        let receipt = step
            .stream
            .stream_receipt
            .as_ref()
            .ok_or(Error::TransferFile)?;
        let name = format!("disk-{i}.vhd");
        hash_file(&dir, &name, receipt, cancel)?;
        sizes.push(receipt.bytes);
        manifest += &format!(
            "SHA256({name})= {:x}\n",
            sha2::digest::Output::<Sha256>::from(receipt.sha256)
        );
        names.push((name, receipt.bytes));
    }
    let descriptor = super::export::descriptor(&export.captured.summary, &sizes)?;
    manifest += &format!(
        "SHA256(guest.ovf)= {:x}\n",
        Sha256::digest(descriptor.as_bytes())
    );
    text_file(&dir, "guest.ovf", descriptor.as_bytes())?;
    text_file(&dir, "guest.mf", manifest.as_bytes())?;
    dir.sync_all().map_err(|_| Error::TransferFile)?;
    cancelled(cancel)?;
    if format == OvfPackageFormat::Ovf {
        let parent = files::parent_handle(&plan.file)?;
        if plan.file.path.symlink_metadata().is_ok() {
            let final_dir = File::from(
                rustix::fs::openat(
                    &parent,
                    plan.file.path.file_name().ok_or(Error::TransferFile)?,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                    Mode::empty(),
                )
                .map_err(|_| Error::TransferDestinationExists)?,
            );
            if !export.directory.identity.same_inode(&files::Identity::from(
                &final_dir.metadata().map_err(|_| Error::TransferFile)?,
            )) {
                return Err(Error::TransferDestinationExists);
            }
        } else {
            rustix::fs::renameat_with(
                &parent,
                export
                    .directory
                    .path
                    .file_name()
                    .ok_or(Error::TransferFile)?,
                &parent,
                plan.file.path.file_name().ok_or(Error::TransferFile)?,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|_| Error::TransferDestinationExists)?;
        }
        parent.sync_all().map_err(|_| Error::TransferFile)?;
        Ok(None)
    } else {
        let output = files::output(plan)?;
        let mut writer = Writer {
            file: output,
            hash: Sha256::new(),
            bytes: 0,
        };
        writer.member(&dir, "guest.ovf", descriptor.len() as u64, cancel)?;
        writer.member(&dir, "guest.mf", manifest.len() as u64, cancel)?;
        for (name, size) in names {
            writer.member(&dir, &name, size, cancel)?;
        }
        writer.write(&[0; 1024])?;
        writer.file.sync_all().map_err(|_| Error::TransferFile)?;
        let selected = super::package::select_ova(
            plan.temporary.as_ref().ok_or(Error::TransferFile)?,
            cancel,
        )?;
        if selected.ovf().is_none_or(|v| *v != export.captured.summary) {
            return Err(Error::TransferConflict);
        }
        Ok(Some(files::receipt(
            &writer.file,
            writer.bytes,
            writer.hash.finalize().into(),
        )?))
    }
}
pub(crate) fn cleanup(plan: &Plan) {
    let Some(export) = &plan.ovf_export else {
        return;
    };
    let Ok(dir) = handle(&export.directory, &plan.file) else {
        return;
    };
    if !export.directory.path.exists() {
        return;
    }
    for (i, step) in export.steps.iter().enumerate() {
        let name = format!("disk-{i}.vhd");
        if let Some(receipt) = &step.stream.stream_receipt
            && let Ok(file) = open(&dir, &name, false)
            && file
                .metadata()
                .is_ok_and(|m| receipt.identity.same_inode(&files::Identity::from(&m)))
        {
            let _ = rustix::fs::unlinkat(&dir, &name, AtFlags::empty());
        }
        files::cleanup(&step.stream);
    }
    // These bounded metadata files were created only inside this owned private directory.
    for name in ["guest.ovf", "guest.mf"] {
        if let Ok(file) = open(&dir, name, false)
            && file.metadata().is_ok_and(|m| m.len() <= 8 * 1024 * 1024)
        {
            let _ = rustix::fs::unlinkat(&dir, name, AtFlags::empty());
        }
    }
    if let Ok(parent) = files::parent_handle(&plan.file) {
        let _ = rustix::fs::unlinkat(
            &parent,
            export.directory.path.file_name().unwrap_or_default(),
            AtFlags::REMOVEDIR,
        );
    }
}
