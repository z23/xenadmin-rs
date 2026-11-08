use super::*;
use std::{
    cell::Cell,
    io::{Read, Seek},
    rc::Rc,
};

const MAX_DECODED: u64 = 256 * 1024 * 1024 * 1024;
const MAX_METADATA: u64 = 1024 * 1024;
struct Limited<R> {
    inner: R,
    total: u64,
    allowance: Rc<Cell<u64>>,
    cancel: CancellationToken,
    deadline: std::time::Instant,
}
impl<R: Read> Read for Limited<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() || std::time::Instant::now() >= self.deadline {
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
    let compressed = matches!(operation, HostArchiveOperation::Backup);
    let reader: Box<dyn Read + '_> = if compressed {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    scan_cancel(reader, host, expected_version, compressed, cancel)
}
#[cfg(test)]
fn scan(
    reader: impl Read,
    host: HostId,
    expected_version: &str,
    backup: bool,
) -> Result<(), Error> {
    scan_cancel(
        reader,
        host,
        expected_version,
        backup,
        &CancellationToken::new(),
    )
}
fn scan_cancel(
    reader: impl Read,
    host: HostId,
    expected_version: &str,
    backup: bool,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    let result = scan_inner(reader, host, expected_version, backup, cancel, deadline);
    if cancel.is_cancelled() {
        Err(Error::Closed)
    } else if std::time::Instant::now() >= deadline {
        Err(Error::Timeout)
    } else {
        result
    }
}
fn scan_inner(
    reader: impl Read,
    host: HostId,
    expected_version: &str,
    backup: bool,
    cancel: &CancellationToken,
    deadline: std::time::Instant,
) -> Result<(), Error> {
    // tar's long-name/PAX handling reads extension bodies into memory. Bound each next()
    // independently; drain ordinary file bodies separately with a fixed-size buffer.
    let allowance = Rc::new(Cell::new(MAX_METADATA));
    let mut archive = tar::Archive::new(Limited {
        inner: reader,
        total: 0,
        allowance: allowance.clone(),
        cancel: cancel.clone(),
        deadline,
    });
    let mut entries = archive.entries().map_err(|_| Error::TransferArchive)?;
    let mut count = 0;
    let mut logical = 0u64;
    let mut identity_found = false;
    let mut names = BTreeSet::new();
    let mut name_bytes = 0usize;
    loop {
        allowance.set(MAX_METADATA);
        let Some(entry) = entries.next() else { break };
        let mut entry = entry.map_err(|_| Error::TransferArchive)?;
        count += 1;
        if count > 500_000 {
            return Err(Error::TransferLimit);
        }
        let name = path(&entry.path_bytes())?;
        if !names.insert(name.clone()) && !entry.header().entry_type().is_dir() {
            return Err(Error::TransferArchive);
        }
        name_bytes = name_bytes
            .checked_add(name.len())
            .filter(|n| *n <= 16 * 1024 * 1024)
            .ok_or(Error::TransferLimit)?;
        let size = entry.size();
        logical = logical
            .checked_add(size)
            .filter(|n| *n <= MAX_DECODED)
            .ok_or(Error::TransferLimit)?;
        allowance.set(size.saturating_add(MAX_METADATA));
        let metadata = if backup {
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
            if backup {
                inventory(&bytes, host, expected_version)?;
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
                if cancel.is_cancelled() {
                    return Err(Error::Closed);
                }
                if std::time::Instant::now() >= deadline {
                    return Err(Error::Timeout);
                }
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
    Ok(())
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
        let mut r = Limited {
            inner: &b"abc"[..],
            total: 0,
            allowance: Rc::new(Cell::new(100)),
            cancel: CancellationToken::new(),
            deadline: std::time::Instant::now(),
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
}
