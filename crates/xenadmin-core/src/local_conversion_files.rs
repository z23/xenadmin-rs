use super::*;
use rustix::fs::{FlockOperation, Mode, OFlags};
use std::io::{Seek, SeekFrom};

pub(super) fn private_dir(path: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| fail("Cannot create private conversion state directory"))?;
    let m = std::fs::symlink_metadata(path)
        .map_err(|_| fail("Cannot inspect private conversion state"))?;
    if !m.is_dir() || m.mode() & 0o077 != 0 || m.uid() != rustix::process::getuid().as_raw() {
        return Err(fail(
            "Conversion state must be a private directory owned by this user",
        ));
    }
    Ok(())
}
pub(super) struct Permit(File);
impl Drop for Permit {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, FlockOperation::Unlock);
    }
}
pub(super) fn lock(dir: &Path) -> Result<Permit> {
    let fd = rustix::fs::open(
        dir.join("worker.lock"),
        OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|_| fail("Cannot open conversion worker lock"))?;
    let file = File::from(fd);
    if !file
        .metadata()
        .map_err(|_| fail("Cannot inspect conversion lock"))?
        .is_file()
    {
        return Err(fail("Invalid conversion worker lock"));
    }
    rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| fail("Another conversion or review already owns the local worker"))?;
    Ok(Permit(file))
}
pub(super) fn open_regular(path: &Path) -> Result<File> {
    let fd = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| {
        fail("Choose a regular local file; symbolic links and devices are not accepted")
    })?;
    let f = File::from(fd);
    if !f
        .metadata()
        .map_err(|_| fail("Cannot inspect local file"))?
        .is_file()
    {
        return Err(fail("Choose a regular local file"));
    }
    Ok(f)
}
pub(super) fn hash_file(file: &mut File, cancel: &AtomicBool) -> Result<(Identity, [u8; 32])> {
    let identity = Identity::from(
        &file
            .metadata()
            .map_err(|_| fail("Cannot inspect local image"))?,
    );
    if identity.size == 0 || identity.size > crate::transfer::MAX_TRANSFER_BYTES {
        return Err(fail("Local image exceeds supported size limits"));
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| fail("Cannot seek local image"))?;
    let started = std::time::Instant::now();
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut bytes = [0u8; 64 * 1024];
    loop {
        stopped(cancel)?;
        if started.elapsed() > Duration::from_secs(4 * 3600) {
            return Err(fail("Image hashing exceeded its time limit"));
        }
        let n = file
            .read(&mut bytes)
            .map_err(|_| fail("Cannot read local image"))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or_else(|| fail("Image size overflow"))?;
        if total > identity.size {
            return Err(fail("Image changed during hashing"));
        }
        hash.update(&bytes[..n]);
    }
    if total != identity.size
        || Identity::from(
            &file
                .metadata()
                .map_err(|_| fail("Cannot inspect local image"))?,
        ) != identity
    {
        return Err(fail("Image changed during hashing"));
    }
    Ok((identity, hash.finalize().into()))
}
pub(super) fn verify_file(
    path: &Path,
    identity: Option<&Identity>,
    hash: Option<&[u8; 32]>,
    cancel: &AtomicBool,
) -> Result<()> {
    let (Some(identity), Some(hash)) = (identity, hash) else {
        return Err(fail("Missing captured image identity"));
    };
    let mut f = open_regular(path)?;
    if Identity::from(&f.metadata().map_err(|_| fail("Cannot inspect image"))?) != *identity {
        return Err(fail("Captured image changed"));
    }
    let (now, digest) = hash_file(&mut f, cancel)?;
    if now != *identity || digest != *hash {
        return Err(fail("Captured image content changed"));
    }
    Ok(())
}
pub(super) fn prepare_paths(
    source: &Path,
    destination: &Path,
    id: uuid::Uuid,
) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if !source.is_absolute() || !destination.is_absolute() {
        return Err(fail("Choose absolute source and destination file paths"));
    }
    let source_parent = source
        .parent()
        .ok_or_else(|| fail("Invalid source path"))?
        .canonicalize()
        .map_err(|_| fail("Cannot inspect source directory"))?;
    let source = source_parent.join(
        source
            .file_name()
            .ok_or_else(|| fail("Invalid source filename"))?,
    );
    open_regular(&source)?;
    let parent = destination
        .parent()
        .ok_or_else(|| fail("Invalid output path"))?
        .canonicalize()
        .map_err(|_| fail("Choose an existing output directory"))?;
    if !parent.is_dir() {
        return Err(fail("Choose an existing output directory"));
    }
    let destination = parent.join(
        destination
            .file_name()
            .ok_or_else(|| fail("Invalid output filename"))?,
    );
    if destination.exists() || std::fs::symlink_metadata(&destination).is_ok() {
        return Err(fail("Output already exists; choose a new filename"));
    }
    let working = parent.join(format!(".xenadmin-conversion-{id}"));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&working)
        .map_err(|_| fail("Cannot create private conversion staging directory"))?;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(working.join("output"))
        .map_err(|_| fail("Cannot create private output directory"))?;
    Ok((source, destination, working))
}
pub(super) fn stage(
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<(Identity, [u8; 32])> {
    let mut original = open_regular(source)?;
    let before = Identity::from(
        &original
            .metadata()
            .map_err(|_| fail("Cannot inspect original image"))?,
    );
    if before.size == 0 || before.size > crate::transfer::MAX_TRANSFER_BYTES {
        return Err(fail("Source image exceeds supported size limits"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .map_err(|_| fail("Cannot create an owned staging file"))?;
    let owned = Identity::from(
        &file
            .metadata()
            .map_err(|_| fail("Cannot inspect staging file"))?,
    );
    let result = (|| {
        let started = std::time::Instant::now();
        let digest = if rustix::fs::ioctl_ficlone(&file, &original).is_ok() {
            hash_file(&mut file, cancel)?.1
        } else {
            file.set_len(0)
                .map_err(|_| fail("Cannot reset staging file"))?;
            file.seek(SeekFrom::Start(0))
                .map_err(|_| fail("Cannot seek staging file"))?;
            let mut bytes = [0; 64 * 1024];
            let mut hash = Sha256::new();
            let mut total = 0u64;
            loop {
                stopped(cancel)?;
                if started.elapsed() > Duration::from_secs(4 * 3600) {
                    return Err(fail("Source staging exceeded its time limit"));
                }
                let n = original
                    .read(&mut bytes)
                    .map_err(|_| fail("Cannot read original image"))?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > before.size {
                    return Err(fail("Original image changed while staging"));
                }
                if bytes[..n].iter().all(|v| *v == 0) {
                    file.seek(SeekFrom::Current(n as i64))
                        .map_err(|_| fail("Cannot stage sparse image"))?;
                } else {
                    file.write_all(&bytes[..n])
                        .map_err(|_| fail("Cannot stage image; check available disk space"))?;
                }
                hash.update(&bytes[..n]);
            }
            if total != before.size {
                return Err(fail("Original image changed while staging"));
            }
            file.set_len(total)
                .map_err(|_| fail("Cannot set staging file size"))?;
            <[u8; 32]>::from(hash.finalize())
        };
        if Identity::from(
            &original
                .metadata()
                .map_err(|_| fail("Cannot inspect original image"))?,
        ) != before
            || Identity::from(
                &open_regular(source)?
                    .metadata()
                    .map_err(|_| fail("Cannot inspect original path"))?,
            ) != before
        {
            return Err(fail("Original image changed while staging"));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o400))
            .map_err(|_| fail("Cannot protect staged source"))?;
        file.sync_all()
            .map_err(|_| fail("Cannot persist staged source"))?;
        let id = Identity::from(
            &file
                .metadata()
                .map_err(|_| fail("Cannot inspect staged source"))?,
        );
        if id.same_inode(&before) {
            return Err(fail("Staged source must be a distinct owned inode"));
        }
        Ok((id, digest))
    })();
    if result.is_err()
        && std::fs::symlink_metadata(destination)
            .is_ok_and(|m| Identity::from(&m).same_inode(&owned))
    {
        let _ = std::fs::remove_file(destination);
    }
    result
}
pub(super) fn verify_working(record: &Record) -> Result<()> {
    let m = std::fs::symlink_metadata(&record.working)
        .map_err(|_| fail("Staging directory disappeared"))?;
    if !m.is_dir()
        || m.mode() & 0o077 != 0
        || !Identity::from(&m).same_inode(&record.working_identity)
    {
        return Err(fail("Owned staging directory changed"));
    }
    let output = std::fs::symlink_metadata(record.working.join("output"))
        .map_err(|_| fail("Output staging directory disappeared"))?;
    let parent = std::fs::symlink_metadata(
        record
            .destination
            .parent()
            .ok_or_else(|| fail("Missing destination parent"))?,
    )
    .map_err(|_| fail("Destination directory disappeared"))?;
    if !output.is_dir()
        || output.mode() & 0o077 != 0
        || !Identity::from(&output).same_inode(&record.output_directory_identity)
        || !parent.is_dir()
        || !Identity::from(&parent).same_inode(&record.parent_identity)
    {
        return Err(fail("Captured output directory changed"));
    }
    Ok(())
}
pub(super) fn save(dir: &Path, record: &Record) -> Result<()> {
    let temp = dir.join(format!(
        "{}.{}.tmp",
        record.summary.id,
        uuid::Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|_| fail("Cannot write private conversion journal"))?;
    let result = (|| {
        file.write_all(
            &serde_json::to_vec(record).map_err(|_| fail("Cannot encode conversion journal"))?,
        )
        .map_err(|_| fail("Cannot save conversion journal"))?;
        file.sync_all()
            .map_err(|_| fail("Cannot persist conversion journal"))?;
        std::fs::rename(&temp, dir.join(format!("{}.json", record.summary.id)))
            .map_err(|_| fail("Cannot publish conversion journal"))?;
        File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|_| fail("Cannot sync conversion journal directory"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub(super) fn publish(record: &mut Record) -> Result<()> {
    verify_working(record)?;
    let source = record.working.join("output/image");
    let current = Identity::from(
        &open_regular(&source)?
            .metadata()
            .map_err(|_| fail("Cannot inspect completed output"))?,
    );
    if record.output_identity.as_ref() != Some(&current) {
        return Err(fail("Converted output changed before publication"));
    }
    let directory = |path: &Path, expected: &Identity| -> Result<File> {
        let f = File::from(
            rustix::fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| fail("Cannot open captured output directory"))?,
        );
        if !Identity::from(
            &f.metadata()
                .map_err(|_| fail("Cannot inspect output directory"))?,
        )
        .same_inode(expected)
        {
            return Err(fail("Captured output directory changed"));
        }
        Ok(f)
    };
    let from = directory(
        &record.working.join("output"),
        &record.output_directory_identity,
    )?;
    let to = directory(
        record
            .destination
            .parent()
            .ok_or_else(|| fail("Missing output parent"))?,
        &record.parent_identity,
    )?;
    rustix::fs::renameat_with(
        &from,
        "image",
        &to,
        record
            .destination
            .file_name()
            .ok_or_else(|| fail("Missing output filename"))?,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|_| fail("Cannot publish output without overwriting an existing path"))?;
    let published_identity = Identity::from(
        &open_regular(&record.destination)?
            .metadata()
            .map_err(|_| fail("Cannot inspect published output"))?,
    );
    if !published_identity.same_inode(&current) {
        return Err(fail(
            "Published output identity changed; inspect the destination manually",
        ));
    }
    record.published_identity = Some(published_identity);
    to.sync_all()
        .map_err(|_| fail("Cannot sync published output directory"))?;
    Ok(())
}
pub(super) fn cleanup(record: &Record) -> Result<()> {
    verify_working(record)?;
    for (path, identity) in [
        (
            record.working.join("source"),
            record.source_identity.as_ref(),
        ),
        (
            record.working.join("output/image"),
            record.output_identity.as_ref(),
        ),
    ] {
        if let Some(id) = identity
            && let Ok(m) = std::fs::symlink_metadata(&path)
            && m.is_file()
            && Identity::from(&m).same_inode(id)
        {
            std::fs::remove_file(path).map_err(|_| fail("Cannot remove owned temporary image"))?;
        }
    }
    // Unknown files are retained; never recursively remove an image-tool directory.
    let _ = std::fs::remove_dir(record.working.join("output"));
    let _ = std::fs::remove_dir(&record.working);
    Ok(())
}
pub(super) fn history(dir: &Path) -> Result<Vec<ConversionSummary>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    private_dir(dir)?;
    let permit = lock(dir).ok();
    let mut rows = vec![];
    let entries = std::fs::read_dir(dir)
        .map_err(|_| fail("Cannot read conversion history"))?
        .take(1025)
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|_| fail("Cannot read conversion history entries"))?;
    if entries.len() > 1024 {
        return Err(fail(
            "Conversion history exceeds 1,024 directory entries; no incomplete list is displayed",
        ));
    }
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let mut record = load(&path)?;
        if permit.is_some()
            && matches!(
                record.summary.state,
                ConversionState::Preparing
                    | ConversionState::Ready
                    | ConversionState::Converting
                    | ConversionState::Verifying
            )
        {
            record.summary.state = ConversionState::Interrupted;
            record.summary.detail="The previous local worker stopped. Temporary files are retained; an output may already exist at the reviewed destination. Conversion is never resumed automatically.".into();
            save(dir, &record)?;
        }
        rows.push(record.summary);
    }
    rows.sort_by_key(|r| r.id);
    Ok(rows)
}

pub(super) fn load(path: &Path) -> Result<Record> {
    let f = open_regular(path)?;
    let m = f
        .metadata()
        .map_err(|_| fail("Cannot inspect conversion history"))?;
    if m.len() > 64 * 1024 || m.mode() & 0o077 != 0 || m.uid() != rustix::process::getuid().as_raw()
    {
        return Err(fail(
            "Invalid private conversion history permissions or size",
        ));
    }
    let mut bytes = vec![];
    f.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("Cannot read conversion history"))?;
    if bytes.len() > 64 * 1024 {
        return Err(fail("Conversion history exceeds size limit"));
    }
    let r: Record =
        serde_json::from_slice(&bytes).map_err(|_| fail("Invalid private conversion history"))?;
    if r.summary.id.is_nil()
        || path.file_name().and_then(|s| s.to_str()) != Some(&format!("{}.json", r.summary.id))
        || r.working.file_name().and_then(|s| s.to_str())
            != Some(&format!(".xenadmin-conversion-{}", r.summary.id))
        || !r.original.is_absolute()
        || !r.destination.is_absolute()
        || r.working.parent() != r.destination.parent()
        || r.summary.detail.len() > 2048
        || r.source_identity.is_some() != r.source_hash.is_some()
        || r.output_hash.is_some() && r.output_identity.is_none()
        || r.published_identity.is_some() && r.output_hash.is_none()
        || r.summary.state == ConversionState::Completed
            && (r.published_identity.is_none() || r.summary.virtual_size.is_none())
        || matches!(
            r.summary.state,
            ConversionState::Ready | ConversionState::Converting | ConversionState::Verifying
        ) && (r.source_hash.is_none() || r.summary.virtual_size.is_none())
    {
        return Err(fail(
            "Conversion history identity or evidence is inconsistent",
        ));
    }
    Ok(r)
}
