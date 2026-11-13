use super::*;
use std::{
    cell::{Cell, RefCell},
    io::{Read, Seek},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

const MAX_DECODED: u64 = 256 * 1024 * 1024 * 1024;
const MAX_METADATA: u64 = 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Backup,
    Support,
    Restore,
}
struct Stop<'a> {
    connection: Option<&'a CancellationToken>,
    selection: Option<&'a AtomicBool>,
    deadline: std::time::Instant,
}
impl Stop<'_> {
    fn check(&self) -> Result<(), Error> {
        if self.connection.is_some_and(CancellationToken::is_cancelled)
            || self.selection.is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err(Error::Closed);
        }
        if std::time::Instant::now() >= self.deadline {
            return Err(Error::Timeout);
        }
        Ok(())
    }
}
struct Limited<'a, R> {
    inner: R,
    total: u64,
    allowance: Rc<Cell<u64>>,
    stop: &'a Stop<'a>,
}
impl<R: Read> Read for Limited<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.stop.check().is_err() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let available = self
            .allowance
            .get()
            .min(MAX_DECODED.saturating_sub(self.total));
        if available == 0 {
            return Err(std::io::Error::other(
                "host archive exceeds inspection bounds",
            ));
        }
        let limit = buffer.len().min(available as usize);
        let n = self.inner.read(&mut buffer[..limit])?;
        self.total += n as u64;
        self.allowance.set(self.allowance.get() - n as u64);
        Ok(n)
    }
}
fn path(bytes: &[u8]) -> Result<String, Error> {
    let name = std::str::from_utf8(bytes).map_err(|_| Error::TransferArchive)?;
    if name.len() > 4096
        || name.starts_with('/')
        || name.chars().any(char::is_control)
        || name.split('/').any(|p| p == "..")
    {
        return Err(Error::TransferArchive);
    }
    Ok(name
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/"))
}
fn inventory(bytes: &[u8], host: HostId, expected_version: &str) -> Result<(), Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::TransferArchive)?;
    let mut uuid = None;
    let mut version = None;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let slot = match key.trim() {
            "INSTALLATION_UUID" => &mut uuid,
            "PRODUCT_VERSION" => &mut version,
            _ => continue,
        };
        if slot.is_some() {
            return Err(Error::TransferArchive);
        }
        let value = value.trim();
        let value = value
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .or_else(|| value.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
            .unwrap_or(value);
        if value.is_empty()
            || value.len() > 64
            || value
                .chars()
                .any(|c| c.is_control() || matches!(c, '\'' | '"' | '`' | '$' | '\\'))
        {
            return Err(Error::TransferArchive);
        }
        *slot = Some(value);
    }
    if uuid.and_then(|s| HostId::parse(s).ok()) != Some(host) || version != Some(expected_version) {
        return Err(Error::TransferConflict);
    }
    Ok(())
}
pub(crate) fn inspect(
    file: &mut std::fs::File,
    host: HostId,
    expected_version: &str,
    operation: &HostArchiveOperation,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    file.rewind().map_err(|_| Error::TransferFile)?;
    let backup = matches!(operation, HostArchiveOperation::Backup);
    let reader: Box<dyn Read + '_> = if backup {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    scan_cancel(reader, host, expected_version, backup, cancel)
}
struct Hashed<R> {
    inner: R,
    hash: Rc<RefCell<sha2::Sha256>>,
}
impl<R: Read> Read for Hashed<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        let n = self.inner.read(buffer)?;
        self.hash.borrow_mut().update(&buffer[..n]);
        Ok(n)
    }
}
pub(crate) fn inspect_restore(
    file: &mut std::fs::File,
    host: HostId,
    version: &str,
    connection: Option<&CancellationToken>,
    selection: Option<&AtomicBool>,
) -> Result<super::HostBackupSummary, Error> {
    use sha2::Digest;
    file.rewind().map_err(|_| Error::TransferFile)?;
    let hash = Rc::new(RefCell::new(sha2::Sha256::new()));
    let reader = flate2::read::MultiGzDecoder::new(Hashed {
        inner: file,
        hash: hash.clone(),
    });
    let stop = Stop {
        connection,
        selection,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(600),
    };
    let result = scan_inner(reader, host, version, Mode::Restore, &stop);
    stop.check()?;
    let (entries, logical_bytes) = result?;
    let sha256 = hash.borrow().clone().finalize().into();
    Ok(super::HostBackupSummary {
        host,
        version: version.into(),
        entries,
        logical_bytes,
        sha256,
    })
}
#[cfg(test)]
fn scan(reader: impl Read, host: HostId, version: &str, backup: bool) -> Result<(), Error> {
    scan_cancel(reader, host, version, backup, &CancellationToken::new())
}
fn scan_cancel(
    reader: impl Read,
    host: HostId,
    version: &str,
    backup: bool,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let stop = Stop {
        connection: Some(cancel),
        selection: None,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(600),
    };
    let result = scan_inner(
        reader,
        host,
        version,
        if backup { Mode::Backup } else { Mode::Support },
        &stop,
    );
    stop.check()?;
    result.map(|_| ())
}
fn scan_inner(
    reader: impl Read,
    host: HostId,
    expected_version: &str,
    mode: Mode,
    stop: &Stop<'_>,
) -> Result<(u32, u64), Error> {
    // tar's long-name/PAX handling reads extension bodies into memory. Bound each next()
    // independently; drain ordinary file bodies separately with a fixed-size buffer.
    let allowance = Rc::new(Cell::new(MAX_METADATA));
    let mut archive = tar::Archive::new(Limited {
        inner: reader,
        total: 0,
        allowance: allowance.clone(),
        stop,
    });
    let mut entries = archive.entries().map_err(|_| Error::TransferArchive)?;
    let mut count = 0;
    let mut logical = 0u64;
    let mut identity_found = false;
    let mut names = BTreeSet::new();
    let mut name_bytes = 0usize;
    let mut graph = super::restore::Graph::default();
    loop {
        allowance.set(MAX_METADATA);
        let Some(entry) = entries.next() else { break };
        let mut entry = entry.map_err(|_| Error::TransferArchive)?;
        count += 1;
        if count > 500_000 {
            return Err(Error::TransferLimit);
        }
        if mode == Mode::Restore
            && let Some(attributes) = entry.pax_extensions().map_err(|_| Error::TransferArchive)?
        {
            let mut seen = BTreeSet::new();
            for attribute in attributes {
                let attribute = attribute.map_err(|_| Error::TransferArchive)?;
                let key = attribute.key().map_err(|_| Error::TransferArchive)?;
                if !matches!(
                    key,
                    "path"
                        | "linkpath"
                        | "size"
                        | "uid"
                        | "gid"
                        | "uname"
                        | "gname"
                        | "mtime"
                        | "atime"
                        | "ctime"
                ) || !seen.insert(key.to_owned())
                {
                    return Err(Error::Protocol(
                        "Recovery archive has unsupported or ambiguous PAX metadata",
                    ));
                }
            }
        }
        let name = path(&entry.path_bytes())?;
        let kind = entry.header().entry_type();
        // XCP-ng's host backup repeats a few names as hard links to themselves (files on
        // the vfat EFI partition that tar sees twice). A hard link whose target is a name
        // already in the archive adds nothing on restore and is accepted; any other
        // repeated non-directory name is still malformed.
        let repeated_hard_link = kind.is_hard_link()
            && names.contains(&name)
            && entry
                .link_name_bytes()
                .as_deref()
                .map(path)
                .transpose()?
                .is_some_and(|target| names.contains(&target));
        if !names.insert(name.clone()) && !kind.is_dir() && !repeated_hard_link {
            return Err(Error::TransferArchive);
        }
        name_bytes = name_bytes
            .checked_add(name.len())
            .filter(|n| *n <= 16 * 1024 * 1024)
            .ok_or(Error::TransferLimit)?;
        if mode == Mode::Restore && !repeated_hard_link {
            graph.insert(&name, kind, entry.link_name_bytes().as_deref())?;
        }
        let size = entry.size();
        logical = logical
            .checked_add(size)
            .filter(|n| *n <= MAX_DECODED)
            .ok_or(Error::TransferLimit)?;
        allowance.set(size.saturating_add(MAX_METADATA));
        let metadata = if mode != Mode::Support {
            name == "etc/xensource-inventory"
        } else {
            name.ends_with("/inventory.xml") || name == "inventory.xml"
        };
        if metadata {
            if identity_found || !entry.header().entry_type().is_file() || size > MAX_METADATA {
                return Err(Error::TransferArchive);
            }
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|_| Error::TransferArchive)?;
            if mode != Mode::Support {
                inventory(&bytes, host, expected_version)?;
                if mode == Mode::Restore {
                    super::restore::literal_inventory(&bytes)?;
                }
            } else {
                let xml = std::str::from_utf8(&bytes).map_err(|_| Error::TransferArchive)?;
                let doc = roxmltree::Document::parse(xml).map_err(|_| Error::TransferArchive)?;
                if doc.root_element().tag_name().name() != "system-status-inventory" {
                    return Err(Error::TransferArchive);
                }
            }
            identity_found = true;
        } else {
            let mut buffer = [0; 65536];
            loop {
                stop.check()?;
                if entry
                    .read(&mut buffer)
                    .map_err(|_| Error::TransferArchive)?
                    == 0
                {
                    break;
                }
            }
        }
    }
    allowance.set(MAX_DECODED);
    let mut rest = archive.into_inner();
    let mut buffer = [0; 65536];
    loop {
        let n = rest.read(&mut buffer).map_err(|_| Error::TransferArchive)?;
        if n == 0 {
            break;
        }
        if buffer[..n].iter().any(|b| *b != 0) {
            return Err(Error::TransferArchive);
        }
    }
    if !identity_found {
        return Err(Error::TransferArchive);
    }
    if mode == Mode::Restore {
        graph.validate()?;
    }
    Ok((count, logical))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tar_file(name: &str, content: &[u8]) -> Vec<u8> {
        let mut out = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        out.append_data(&mut header, name, content).unwrap();
        out.into_inner().unwrap()
    }
    #[test]
    fn backup_identity_and_support_inventory_are_not_interchangeable() {
        let host = HostId::parse("00000000-0000-0000-0000-000000000002").unwrap();
        let data = tar_file(
            "./etc/xensource-inventory",
            format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='8.3.0'\n").as_bytes(),
        );
        assert!(scan(&data[..], host, "8.3.0", true).is_ok());
        assert!(scan(&data[..], host, "8.2.0", true).is_err());
        assert!(scan(&data[..], host, "8.3.0", false).is_err());
        let support = tar_file("bundle/inventory.xml", b"<system-status-inventory/>");
        assert!(scan(&support[..], host, "8.3.0", false).is_ok());
        assert!(scan(&support[..], host, "8.3.0", true).is_err());
        let mut trailing = support.clone();
        trailing.push(1);
        assert!(scan(&trailing[..], host, "8.3.0", false).is_err());
    }
    #[test]
    fn hard_links_repeating_a_known_name_are_accepted_but_repeated_files_are_not() {
        let host = HostId::from(uuid::Uuid::from_u128(2));
        let inventory = format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='8.3.0'\n");
        let build = |second_is_link: bool| {
            let mut builder = tar::Builder::new(Vec::new());
            let mut h = tar::Header::new_gnu();
            h.set_size(inventory.len() as u64);
            h.set_mode(0o600);
            h.set_cksum();
            builder
                .append_data(&mut h, "./etc/xensource-inventory", inventory.as_bytes())
                .unwrap();
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_mode(0o700);
            h.set_cksum();
            builder
                .append_data(&mut h, "./boot/efi/EFI/BOOT/BOOTX64.EFI", &b"x"[..])
                .unwrap();
            let mut h = tar::Header::new_gnu();
            if second_is_link {
                h.set_entry_type(tar::EntryType::Link);
                h.set_link_name("./boot/efi/EFI/BOOT/BOOTX64.EFI").unwrap();
                h.set_size(0);
            } else {
                h.set_size(1);
            }
            h.set_mode(0o700);
            h.set_cksum();
            if second_is_link {
                builder
                    .append_data(&mut h, "./boot/efi/EFI/BOOT/BOOTX64.EFI", std::io::empty())
                    .unwrap();
            } else {
                builder
                    .append_data(&mut h, "./boot/efi/EFI/BOOT/BOOTX64.EFI", &b"y"[..])
                    .unwrap();
            }
            builder.into_inner().unwrap()
        };
        assert!(scan(&build(true)[..], host, "8.3.0", true).is_ok());
        assert!(scan(&build(false)[..], host, "8.3.0", true).is_err());
    }
    #[test]
    fn extension_bodies_and_paths_are_bounded_without_extraction() {
        let host = HostId::parse("00000000-0000-0000-0000-000000000002").unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::GNULongName);
        header.set_size(MAX_METADATA + 1);
        header.set_cksum();
        let mut data = header.as_bytes().to_vec();
        data.resize((MAX_METADATA + 1 + 512) as usize, b'a');
        assert!(scan(&data[..], host, "8.3.0", true).is_err());
        for name in ["/etc/file", "./../file", "a/../../file", "a\n"] {
            assert!(path(name.as_bytes()).is_err());
        }
    }
    #[test]
    fn archive_inspection_stops_on_cancellation_and_deadline() {
        let host = HostId::from(uuid::Uuid::from_u128(2));
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            scan_cancel(&b""[..], host, "8.3.0", false, &cancel),
            Err(Error::Closed)
        ));
        let stop = Stop {
            connection: None,
            selection: None,
            deadline: std::time::Instant::now(),
        };
        let mut r = Limited {
            inner: &b"abc"[..],
            total: 0,
            allowance: Rc::new(Cell::new(100)),
            stop: &stop,
        };
        assert!(r.read(&mut [0; 1]).is_err());
    }
    #[test]
    fn gnu_sparse_long_paths_and_repeated_directories_are_supported() {
        let host = HostId::from(uuid::Uuid::from_u128(2));
        let inventory = format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='8.3.0'\n");
        let mut builder = tar::Builder::new(Vec::new());
        for _ in 0..2 {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Directory);
            h.set_size(0);
            h.set_mode(0o700);
            h.set_cksum();
            builder
                .append_data(&mut h, "./boot/efi", std::io::empty())
                .unwrap();
        }
        let mut h = tar::Header::new_gnu();
        h.set_size(inventory.len() as u64);
        h.set_mode(0o600);
        h.set_cksum();
        builder
            .append_data(&mut h, "./etc/xensource-inventory", inventory.as_bytes())
            .unwrap();
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_mode(0o600);
        h.set_cksum();
        builder
            .append_data(&mut h, format!("var/log/{}", "a".repeat(220)), &b"x"[..])
            .unwrap();
        let mut h = tar::Header::new_gnu();
        h.set_path("var/log/sparse").unwrap();
        h.set_entry_type(tar::EntryType::GNUSparse);
        h.set_size(1);
        h.set_mode(0o600);
        let g = h.as_gnu_mut().unwrap();
        g.set_real_size(4096);
        g.sparse[0].set_offset(4095);
        g.sparse[0].set_length(1);
        h.set_cksum();
        builder.append(&h, &b"x"[..]).unwrap();
        let bytes = builder.into_inner().unwrap();
        assert!(scan(&bytes[..], host, "8.3.0", true).is_ok());
    }
    #[test]
    fn host_archive_gnu_long_name_cannot_hide_a_pax_path() {
        use std::io::Write;
        let host = HostId::from(uuid::Uuid::from_u128(2));
        let inventory = format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='8.3.0'\n");
        let inspect = |bytes: &[u8]| {
            let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gzip.write_all(bytes).unwrap();
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&gzip.finish().unwrap()).unwrap();
            inspect_restore(&mut file, host, "8.3.0", None, None)
        };
        fn gnu(builder: &mut tar::Builder<Vec<u8>>, kind: tar::EntryType, value: &str) {
            let mut body = value.as_bytes().to_vec();
            body.push(0);
            let mut header = tar::Header::new_gnu();
            let long = b"././@LongLink";
            header.as_gnu_mut().unwrap().name[..long.len()].copy_from_slice(long);
            header.set_entry_type(kind);
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &body[..]).unwrap();
        }
        let archive = |pax: &[(&str, &[u8])],
                       long_name: Option<&str>,
                       long_link: Option<&str>,
                       name: &str,
                       kind: tar::EntryType,
                       link: Option<&str>,
                       body: &[u8]| {
            let mut builder = tar::Builder::new(Vec::new());
            let mut header = tar::Header::new_gnu();
            header.set_size(inventory.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            builder
                .append_data(&mut header, "etc/xensource-inventory", inventory.as_bytes())
                .unwrap();
            if !pax.is_empty() {
                builder.append_pax_extensions(pax.iter().copied()).unwrap();
            }
            if let Some(value) = long_name {
                gnu(&mut builder, tar::EntryType::GNULongName, value);
            }
            if let Some(value) = long_link {
                gnu(&mut builder, tar::EntryType::GNULongLink, value);
            }
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_size(body.len() as u64);
            header.set_mode(0o600);
            if let Some(link) = link {
                header.set_link_name(link).unwrap();
            }
            header.set_cksum();
            builder.append_data(&mut header, name, body).unwrap();
            builder.into_inner().unwrap()
        };
        let cases = [
            (
                "gnu-long-name-and-pax-path",
                archive(
                    &[("path", b"../outside" as &[u8])],
                    Some("var/payload"),
                    None,
                    "var/payload",
                    tar::EntryType::Regular,
                    None,
                    b"x",
                ),
                false,
            ),
            (
                "pax-path-without-gnu-long-name",
                archive(
                    &[("path", b"../outside" as &[u8])],
                    None,
                    None,
                    "var/payload",
                    tar::EntryType::Regular,
                    None,
                    b"x",
                ),
                false,
            ),
            (
                "gnu-long-link-and-pax-linkpath",
                archive(
                    &[("linkpath", b"../../outside" as &[u8])],
                    None,
                    Some("etc/xensource-inventory"),
                    "var/link",
                    tar::EntryType::Symlink,
                    Some("etc/xensource-inventory"),
                    b"",
                ),
                false,
            ),
            (
                "pax-linkpath-without-gnu-long-link",
                archive(
                    &[("linkpath", b"../../outside" as &[u8])],
                    None,
                    None,
                    "var/link",
                    tar::EntryType::Symlink,
                    Some("var/link"),
                    b"",
                ),
                false,
            ),
            (
                "gnu-long-name-and-different-pax-path",
                archive(
                    &[("path", b"var/other" as &[u8])],
                    Some("var/payload"),
                    None,
                    "var/payload",
                    tar::EntryType::Regular,
                    None,
                    b"x",
                ),
                false,
            ),
            (
                "gnu-long-link-and-different-pax-linkpath",
                archive(
                    &[("linkpath", b"var/other" as &[u8])],
                    None,
                    Some("etc/xensource-inventory"),
                    "var/link",
                    tar::EntryType::Symlink,
                    Some("etc/xensource-inventory"),
                    b"",
                ),
                false,
            ),
            (
                "pax-path-agrees",
                archive(
                    &[("path", b"./var/payload" as &[u8])],
                    Some("var/payload"),
                    None,
                    "var/payload",
                    tar::EntryType::Regular,
                    None,
                    b"x",
                ),
                true,
            ),
            (
                "pax-linkpath-agrees",
                archive(
                    &[("linkpath", b"./etc/xensource-inventory" as &[u8])],
                    None,
                    Some("etc/xensource-inventory"),
                    "var/link",
                    tar::EntryType::Symlink,
                    Some("etc/xensource-inventory"),
                    b"",
                ),
                true,
            ),
        ];
        let mut missed = Vec::new();
        for (label, bytes, accept) in cases {
            let result = inspect(&bytes);
            let matches_contract = if accept {
                result.is_ok()
            } else {
                matches!(result, Err(Error::TransferArchive))
            };
            if !matches_contract {
                missed.push(format!("{label}: {result:?}"));
            }
        }
        assert!(missed.is_empty(), "{missed:?}");
    }
}
