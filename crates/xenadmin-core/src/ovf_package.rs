use super::*;
use crate::transfer::{
    MAX_TRANSFER_BYTES,
    files::{self, FileRange},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::atomic::{AtomicBool, Ordering},
};
const META: u64 = 8 * 1024 * 1024;
fn cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        Err(Error::Closed)
    } else {
        Ok(())
    }
}
pub(crate) struct Window {
    file: File,
    start: u64,
    len: u64,
    pos: u64,
}
impl Window {
    pub(crate) fn new(file: &TransferFile) -> Result<Self, Error> {
        Ok(Self {
            file: files::source(file)?,
            start: file.range.as_ref().map_or(0, |r| r.offset),
            len: file.size().ok_or(Error::TransferFile)?,
            pos: 0,
        })
    }
}
impl Read for Window {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let limit = (self.len - self.pos).min(b.len() as u64) as usize;
        let n = self.file.read(&mut b[..limit])?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl Seek for Window {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        let n = match p {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => i128::from(self.len) + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.pos) + i128::from(n),
        };
        if !(0..=i128::from(self.len)).contains(&n) {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        self.pos = n as u64;
        self.file.seek(SeekFrom::Start(self.start + self.pos))?;
        Ok(self.pos)
    }
}
fn text(file: &TransferFile) -> Result<String, Error> {
    let size = file
        .size()
        .filter(|n| *n <= META)
        .ok_or(Error::TransferLimit)?;
    let mut data = vec![0; size as usize];
    Window::new(file)?
        .read_exact(&mut data)
        .map_err(|_| Error::TransferFile)?;
    String::from_utf8(data).map_err(|_| Error::Ovf("metadata is not UTF-8"))
}
fn octal(b: &[u8]) -> Result<u64, Error> {
    if b.first() == Some(&0x80) {
        return b[1..].iter().try_fold(0u64, |n, b| {
            n.checked_mul(256)
                .and_then(|n| n.checked_add(u64::from(*b)))
                .ok_or(Error::TransferLimit)
        });
    }

    u64::from_str_radix(
        std::str::from_utf8(b)
            .map_err(|_| Error::Ovf("invalid archive header"))?
            .trim_matches(['\0', ' ']),
        8,
    )
    .map_err(|_| Error::Ovf("invalid archive number"))
}
fn tar_name(b: &[u8]) -> Result<String, Error> {
    let length = b.iter().position(|v| *v == 0).unwrap_or(b.len());
    if b[length..].iter().any(|v| *v != 0) {
        return Err(Error::Ovf("invalid archive filename"));
    }
    String::from_utf8(b[..length].to_vec())
        .map_err(|_| Error::Ovf("archive filenames must be UTF-8"))
}
fn members(
    file: &TransferFile,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, TransferFile>, Error> {
    let mut input = Window::new(file)?;
    let mut result = BTreeMap::new();
    let length = file.size().ok_or(Error::TransferFile)?;
    let mut pos = 0u64;
    while pos.checked_add(512).is_some_and(|n| n <= length) {
        cancelled(cancel)?;
        let mut h = [0; 512];
        input
            .seek(SeekFrom::Start(pos))
            .map_err(|_| Error::TransferFile)?;
        input.read_exact(&mut h).map_err(|_| Error::TransferFile)?;
        if h.iter().all(|v| *v == 0) {
            let remain = length - pos - 512;
            if !(512..=128 * 1024).contains(&remain) {
                return Err(Error::Ovf(
                    "archive end markers or trailing data are invalid",
                ));
            }
            let mut end = vec![0; remain as usize];
            input
                .read_exact(&mut end)
                .map_err(|_| Error::TransferFile)?;
            if end.iter().any(|b| *b != 0) {
                return Err(Error::Ovf("archive has trailing data"));
            }
            return Ok(result);
        }
        let checksum = h
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u64::from(*b)
                }
            })
            .sum::<u64>();
        if octal(&h[148..156])? != checksum
            || !matches!(h[156], 0 | b'0')
            || h[157..257].iter().any(|b| *b != 0)
            || !matches!(&h[257..263], b"ustar\0" | b"\0\0\0\0\0\0")
        {
            return Err(Error::Ovf(
                "only regular USTAR package members are supported",
            ));
        }
        let leaf = tar_name(&h[..100])?;
        let prefix = tar_name(&h[345..500])?;
        let name = super::descriptor::safe_name(&if prefix.is_empty() {
            leaf
        } else {
            format!("{prefix}/{leaf}")
        })?;
        let size = octal(&h[124..136])?;
        let end = pos
            .checked_add(512)
            .and_then(|n| n.checked_add(size))
            .filter(|n| *n <= length)
            .ok_or(Error::TransferLimit)?;
        if size == 0 || size > MAX_TRANSFER_BYTES || result.len() >= 512 {
            return Err(Error::TransferLimit);
        }
        let mut part = file.clone();
        part.range = Some(FileRange {
            offset: pos + 512,
            length: size,
        });
        if result.insert(name, part).is_some() {
            return Err(Error::Ovf("duplicate package members"));
        }
        pos = end
            .checked_add(511)
            .map(|n| n / 512 * 512)
            .ok_or(Error::TransferLimit)?;
    }
    Err(Error::Ovf("archive has no complete end markers"))
}
fn hash(file: &TransferFile, sha1: bool, cancel: &AtomicBool) -> Result<String, Error> {
    let mut file = Window::new(file)?;
    let mut buffer = [0; 65536];
    let mut one = sha1::Sha1::new();
    let mut two = Sha256::new();
    loop {
        cancelled(cancel)?;
        let n = file.read(&mut buffer).map_err(|_| Error::TransferFile)?;
        if n == 0 {
            break;
        }
        if sha1 {
            one.update(&buffer[..n]);
        } else {
            two.update(&buffer[..n]);
        }
    }
    Ok(if sha1 {
        one.finalize().iter().map(|b| format!("{b:02x}")).collect()
    } else {
        two.finalize().iter().map(|b| format!("{b:02x}")).collect()
    })
}
fn verify_manifest(
    manifest: &str,
    files: &BTreeMap<String, TransferFile>,
    cancel: &AtomicBool,
) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for line in manifest.lines().filter(|s| !s.trim().is_empty()) {
        let (one, rest) = if let Some(s) = line.strip_prefix("SHA256(") {
            (false, s)
        } else if let Some(s) = line.strip_prefix("SHA1(") {
            (true, s)
        } else {
            return Err(Error::Ovf("manifest digest algorithm is unsupported"));
        };
        let (name, digest) = rest
            .split_once(")=")
            .or_else(|| rest.split_once(") ="))
            .ok_or(Error::Ovf("invalid manifest entry"))?;
        super::descriptor::safe_name(name)?;
        let digest = digest.trim();
        if digest.len() != if one { 40 } else { 64 }
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || !seen.insert(name.to_owned())
        {
            return Err(Error::Ovf("invalid or duplicate manifest digest"));
        }
        let file = files
            .get(name)
            .ok_or(Error::Ovf("manifest references an undeclared file"))?;
        if hash(file, one, cancel)? != digest.to_ascii_lowercase() {
            return Err(Error::Ovf(
                "manifest checksum does not match the selected file",
            ));
        }
    }
    if seen != files.keys().cloned().collect() {
        return Err(Error::Ovf(
            "manifest must cover the descriptor and every declared disk",
        ));
    }
    Ok(())
}
pub(crate) fn select(path: &Path) -> Result<TransferFile, Error> {
    select_cancel(path, &AtomicBool::new(false))
}
pub(crate) fn select_cancel(path: &Path, cancel: &AtomicBool) -> Result<TransferFile, Error> {
    let is_ova = path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("ova"));
    select_kind(path, cancel, is_ova)
}
pub(crate) fn select_ova(path: &Path, cancel: &AtomicBool) -> Result<TransferFile, Error> {
    select_kind(path, cancel, true)
}
fn select_kind(path: &Path, cancel: &AtomicBool, is_ova: bool) -> Result<TransferFile, Error> {
    cancelled(cancel)?;
    let mut outer = files::select_plain(path)?;
    let archive = if is_ova {
        Some(members(&outer, cancel)?)
    } else {
        None
    };
    let descriptor = if let Some(entries) = &archive {
        let candidates = entries
            .keys()
            .filter(|s| s.ends_with(".ovf"))
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return Err(Error::Ovf(
                "choose a package containing exactly one OVF descriptor",
            ));
        }
        candidates[0].clone()
    } else {
        outer
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(Error::TransferFile)?
            .to_owned()
    };
    if descriptor.contains('/') {
        return Err(Error::Ovf("the OVF descriptor must be at the package root"));
    }
    let descriptor_file = archive
        .as_ref()
        .and_then(|m| m.get(&descriptor))
        .cloned()
        .unwrap_or_else(|| outer.clone());
    let xml = text(&descriptor_file)?;
    let (mut summary, references) = super::descriptor::parse(&xml)?;
    let mut selected = BTreeMap::from([(descriptor.clone(), descriptor_file)]);
    let parent = outer.path.parent().ok_or(Error::TransferFile)?;
    for (name, size) in references.values() {
        cancelled(cancel)?;
        let file = if let Some(entries) = &archive {
            entries
                .get(name)
                .cloned()
                .ok_or(Error::Ovf("declared disk file is missing from the archive"))?
        } else {
            let file = files::select_plain(&parent.join(name))?;
            if !file.path.starts_with(parent) {
                return Err(Error::Ovf("disk file escaped the package directory"));
            }
            file
        };
        if size.is_some_and(|n| Some(n) != file.size()) {
            return Err(Error::Ovf(
                "declared file length does not match the selected file",
            ));
        }
        if selected.insert(name.clone(), file).is_some() {
            return Err(Error::Ovf("descriptor conflicts with a disk filename"));
        }
    }
    for disk in &summary.disks {
        if let Some(name) = &disk.file {
            let file = selected.get_mut(name).ok_or(Error::TransferFile)?;
            let mut input = Window::new(file)?;
            let inspected = crate::disk_image::inspect_reader(
                &mut input,
                file.size().ok_or(Error::TransferFile)?,
                disk.format,
            )?;
            if inspected.virtual_size != disk.virtual_size {
                return Err(Error::Ovf(
                    "disk image capacity does not match its descriptor",
                ));
            }
            file.disk = Some(inspected);
        }
    }
    let stem = descriptor
        .strip_suffix(".ovf")
        .ok_or(Error::Ovf("descriptor filename must end in .ovf"))?;
    let manifest_name = format!("{stem}.mf");
    let certificate = format!("{stem}.cert");
    let manifest = if let Some(entries) = &archive {
        if entries.contains_key(&certificate) {
            return Err(Error::Ovf(
                "signed packages require certificate verification support",
            ));
        }
        if entries
            .keys()
            .any(|k| !selected.contains_key(k) && k != &manifest_name)
        {
            return Err(Error::Ovf(
                "archive contains undeclared or unsupported members",
            ));
        }
        entries.get(&manifest_name).cloned()
    } else {
        if parent.join(&certificate).symlink_metadata().is_ok() {
            return Err(Error::Ovf(
                "signed packages require certificate verification support",
            ));
        }
        if parent.join(&manifest_name).symlink_metadata().is_ok() {
            Some(files::select_plain(&parent.join(&manifest_name))?)
        } else {
            None
        }
    };
    if let Some(file) = manifest {
        verify_manifest(&text(&file)?, &selected, cancel)?;
        summary.manifest_verified = true;
        selected.insert(manifest_name, file);
    }
    let total = summary
        .guests
        .iter()
        .flat_map(|g| g.disks.iter())
        .try_fold(0u64, |sum, a| {
            sum.checked_add(
                summary
                    .disks
                    .iter()
                    .find(|d| d.key == a.disk)
                    .ok_or(Error::TransferFile)?
                    .virtual_size,
            )
            .filter(|n| *n <= MAX_TRANSFER_BYTES)
            .ok_or(Error::TransferLimit)
        })?;
    let _ = total;
    for file in selected.values() {
        files::source(file)?;
    }
    files::source(&outer)?;
    let digest = Sha256::digest(xml.as_bytes()).into();
    outer.ovf = Some(Box::new(Selection {
        summary,
        descriptor,
        files: selected,
        digest,
    }));
    Ok(outer)
}
