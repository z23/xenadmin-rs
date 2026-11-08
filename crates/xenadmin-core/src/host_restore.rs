use super::*;
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostBackupSummary {
    pub host: HostId,
    pub version: String,
    pub entries: u32,
    pub logical_bytes: u64,
    pub sha256: [u8; 32],
}
pub(crate) fn validate_record(record: &crate::ResourceOperationRecord) -> Result<(), Error> {
    let crate::ResourceAction::Transfer(crate::TransferAction::Host {
        host,
        expected_version,
        operation: HostArchiveOperation::StageRestore { archive },
    }) = &record.action
    else {
        return Ok(());
    };
    let plan = record.transfer_plan.as_ref().ok_or(Error::Journal)?;
    if record.target != ResourceTarget::Pool(record.pool)
        || archive.host != *host
        || archive.version != *expected_version
        || expected_version.is_empty()
        || expected_version.len() > 64
        || expected_version.chars().any(char::is_control)
        || archive.entries == 0
        || archive.entries > 500_000
        || archive.logical_bytes > 256 * 1024 * 1024 * 1024
        || plan.file.host_backup.as_ref() != Some(archive)
        || plan.file.identity.is_none()
        || plan.file.manifest.is_some()
        || plan.file.disk.is_some()
        || plan.file.range.is_some()
        || plan.file.ovf.is_some()
        || plan
            .file
            .size()
            .is_none_or(|n| n == 0 || n > MAX_HOST_ARCHIVE_BYTES)
        || plan.ovf_import.is_some()
        || plan.ovf_export.is_some()
        || plan.disk_creation.is_some()
        || plan.temporary.is_some()
        || plan.baseline.is_some()
        || plan.result.is_some()
        || plan.dispatched && (plan.task_uuid.is_none() || record.task.is_none())
        || plan.task_success && !plan.dispatched
        || (plan.dispatched || !record.steps.is_empty())
            && (record.steps.len() != 2
                || record.steps[0].label != "Transfer archive"
                || record.steps[1].label != "Verify server result")
        || record
            .steps
            .first()
            .is_some_and(|s| s.state == crate::StepState::Verified)
            && plan.stream_receipt.is_none()
        || record
            .steps
            .get(1)
            .is_some_and(|s| s.state == crate::StepState::Verified)
            && !plan.task_success
        || record.state == crate::OperationState::Succeeded
            && (record
                .steps
                .iter()
                .any(|s| s.state != crate::StepState::Verified)
                || !plan.task_success
                || plan.stream_receipt.as_ref().is_none_or(|receipt| {
                    receipt.sha256 != archive.sha256 || Some(receipt.bytes) != plan.file.size()
                })
                || plan.stream_invalid
                || !record.transfer.as_ref().is_some_and(|v| v.stream_complete))
    {
        return Err(Error::Journal);
    }
    Ok(())
}
pub(crate) fn select(
    path: &Path,
    host: HostId,
    version: &str,
    cancel: &AtomicBool,
) -> Result<crate::TransferFile, Error> {
    let mut selected = crate::transfer::files::select_plain(path)?;
    if selected
        .size()
        .is_none_or(|size| size == 0 || size > MAX_HOST_ARCHIVE_BYTES)
    {
        return Err(Error::TransferLimit);
    }
    let mut file = crate::transfer::files::source(&selected)?;
    let summary = super::archive::inspect_restore(&mut file, host, version, None, Some(cancel))?;
    crate::transfer::files::source_unchanged(&selected, &file)?;
    selected.host_backup = Some(summary);
    Ok(selected)
}
pub(crate) fn revalidate(
    file: &crate::TransferFile,
    expected: &HostBackupSummary,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    if file.host_backup.as_ref() != Some(expected)
        || file
            .size()
            .is_none_or(|size| size == 0 || size > MAX_HOST_ARCHIVE_BYTES)
    {
        return Err(Error::TransferConflict);
    }
    let mut source = crate::transfer::files::source(file)?;
    let actual = super::archive::inspect_restore(
        &mut source,
        expected.host,
        &expected.version,
        Some(cancel),
        None,
    )?;
    crate::transfer::files::source_unchanged(file, &source)?;
    if actual != *expected {
        return Err(Error::TransferFileChanged);
    }
    Ok(())
}
pub(crate) fn literal_inventory(bytes: &[u8]) -> Result<(), Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::TransferArchive)?;
    let mut keys = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(Error::Protocol(
            "Recovery inventory must contain literal assignments only",
        ))?;
        if !matches!(
            key,
            "PRODUCT_BRAND"
                | "PRODUCT_NAME"
                | "PRODUCT_VERSION"
                | "PRODUCT_VERSION_TEXT"
                | "PRODUCT_VERSION_TEXT_SHORT"
                | "COMPANY_NAME"
                | "COMPANY_NAME_SHORT"
                | "COMPANY_PRODUCT_BRAND"
                | "BRAND_CONSOLE"
                | "BRAND_CONSOLE_URL"
                | "PLATFORM_NAME"
                | "PLATFORM_VERSION"
                | "PARTITION_LAYOUT"
                | "BUILD_NUMBER"
                | "INSTALLATION_DATE"
                | "PRIMARY_DISK"
                | "BACKUP_PARTITION"
                | "INSTALLATION_UUID"
                | "CONTROL_DOMAIN_UUID"
                | "DOM0_MEM"
                | "DOM0_VCPUS"
                | "MANAGEMENT_INTERFACE"
                | "MANAGEMENT_ADDRESS_TYPE"
                | "CC_PREPARATIONS"
                | "CURRENT_INTERFACES"
                | "OEM_MANUFACTURER"
                | "OEM_MODEL"
                | "OEM_BUILD_NUMBER"
                | "MACHINE_SERIAL_NUMBER"
                | "MACHINE_SERIAL_NAME"
                | "STUNNEL_IDLE_TIMEOUT"
                | "STUNNEL_LEGACY"
        ) || !keys.insert(key)
        {
            return Err(Error::TransferArchive);
        }
        let value = value.trim();
        let literal = if value.starts_with('\'') {
            value
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
                .filter(|s| !s.contains('\''))
        } else if value.starts_with('"') {
            value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .filter(|s| !s.contains(['"', '$', '`', '\\']))
        } else {
            Some(value).filter(|s| {
                s.bytes().all(|b| {
                    b.is_ascii_alphanumeric()
                        || matches!(b, b'_' | b'/' | b'.' | b'-' | b':' | b',' | b'+')
                })
            })
        };
        if literal.is_none_or(|s| s.len() > 4096 || s.chars().any(char::is_control)) {
            return Err(Error::Protocol(
                "Recovery inventory contains executable or ambiguous shell syntax",
            ));
        }
    }
    Ok(())
}
#[derive(Clone, PartialEq)]
enum Kind {
    Directory,
    File,
    Symlink,
    Hardlink(String),
}
#[derive(Default)]
pub(crate) struct Graph {
    names: BTreeMap<String, Kind>,
}
fn link_path(name: &str, target: &[u8], hard: bool) -> Result<String, Error> {
    let text = std::str::from_utf8(target).map_err(|_| Error::TransferArchive)?;
    if text.is_empty()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || hard && text.starts_with('/')
    {
        return Err(Error::TransferArchive);
    }
    let mut parts = if hard || text.starts_with('/') {
        vec![]
    } else {
        name.rsplit_once('/')
            .map(|(p, _)| p.split('/').collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or(Error::TransferArchive)?;
            }
            p => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}
impl Graph {
    pub fn insert(
        &mut self,
        name: &str,
        kind: tar::EntryType,
        link: Option<&[u8]>,
    ) -> Result<(), Error> {
        let value = if kind.is_dir() {
            Kind::Directory
        } else if kind.is_file() || kind.is_gnu_sparse() {
            Kind::File
        } else if kind.is_symlink() {
            link_path(name, link.ok_or(Error::TransferArchive)?, false)?;
            Kind::Symlink
        } else if kind.is_hard_link() {
            Kind::Hardlink(link_path(name, link.ok_or(Error::TransferArchive)?, true)?)
        } else {
            return Err(Error::Protocol(
                "Recovery archive contains an unsupported special file",
            ));
        };
        if name == ".xen-backup-partition" && value != Kind::File {
            return Err(Error::Protocol("Recovery marker must be an ordinary file"));
        }
        if name.is_empty() && value != Kind::Directory {
            return Err(Error::TransferArchive);
        }
        if let Some(old) = self.names.get(name)
            && (old != &Kind::Directory || value != Kind::Directory)
        {
            return Err(Error::TransferArchive);
        }
        self.names.insert(name.into(), value);
        Ok(())
    }
    pub fn validate(&self) -> Result<(), Error> {
        for (name, kind) in &self.names {
            let mut parent = name.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if self
                    .names
                    .get(prefix)
                    .is_some_and(|kind| kind != &Kind::Directory)
                {
                    return Err(Error::Protocol(
                        "Recovery archive traverses a link or non-directory entry",
                    ));
                }
                parent = prefix;
            }
            if let Kind::Hardlink(target) = kind {
                let mut current = target;
                let mut seen = BTreeSet::new();
                loop {
                    if seen.len() >= 64 || !seen.insert(current) {
                        return Err(Error::TransferArchive);
                    }
                    match self.names.get(current) {
                        Some(Kind::File) => break,
                        Some(Kind::Hardlink(next)) => current = next,
                        _ => {
                            return Err(Error::Protocol(
                                "Recovery archive hard link lacks a regular file target",
                            ));
                        }
                    }
                }
            }
        }
        if self.names.get("etc/xensource-inventory") != Some(&Kind::File) {
            return Err(Error::TransferArchive);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sourced_inventory_allows_only_known_literal_assignments() {
        assert!(literal_inventory(b"# inventory\nPRODUCT_VERSION='8.3.0'\nPRIMARY_DISK='/dev/disk/by-id/example'\nDOM0_MEM='4096'\nMANAGEMENT_INTERFACE=''\n").is_ok());
        for bad in [
            "PATH='/tmp/payload'",
            "TMP='/'",
            "IFS='x'",
            "ENV='/tmp/payload'",
            "PRODUCT_VERSION=$(echo hacked)",
            "PRODUCT_VERSION='8.3'; reboot",
            "PRODUCT_VERSION=\"${HOME}\"",
            "function x() { reboot; }",
            ". /tmp/other",
            "PRODUCT_VERSION='8.3'\nPRODUCT_VERSION='8.2'",
            "UNKNOWN_FUTURE_KEY='value'",
        ] {
            assert!(literal_inventory(bad.as_bytes()).is_err(), "{bad}");
        }
    }
    fn graph() -> Graph {
        let mut g = Graph::default();
        g.insert("etc", tar::EntryType::Directory, None).unwrap();
        g.insert("etc/xensource-inventory", tar::EntryType::Regular, None)
            .unwrap();
        g
    }
    #[test]
    fn complete_archive_graph_refuses_link_traversal_missing_targets_and_cycles() {
        let mut good = graph();
        good.insert("usr/lib/file", tar::EntryType::Regular, None)
            .unwrap();
        good.insert("lib", tar::EntryType::Symlink, Some(b"usr/lib"))
            .unwrap();
        good.insert("etc/mtab", tar::EntryType::Symlink, Some(b"/proc/mounts"))
            .unwrap();
        good.insert("hard", tar::EntryType::Link, Some(b"usr/lib/file"))
            .unwrap();
        assert!(good.validate().is_ok());
        good.insert("lib/escaped", tar::EntryType::Regular, None)
            .unwrap();
        assert!(good.validate().is_err());
        let mut reverse = graph();
        reverse
            .insert("before/entry", tar::EntryType::Regular, None)
            .unwrap();
        reverse
            .insert("before", tar::EntryType::Symlink, Some(b"/tmp"))
            .unwrap();
        assert!(reverse.validate().is_err());
        for target in ["missing", "/etc/passwd", "../../outside"] {
            let mut g = graph();
            if g.insert("hard", tar::EntryType::Link, Some(target.as_bytes()))
                .is_ok()
            {
                assert!(g.validate().is_err());
            }
        }
        let mut cycle = graph();
        cycle.insert("a", tar::EntryType::Link, Some(b"b")).unwrap();
        cycle.insert("b", tar::EntryType::Link, Some(b"a")).unwrap();
        assert!(cycle.validate().is_err());
        assert!(
            graph()
                .insert("dev/raw", tar::EntryType::Block, None)
                .is_err()
        );
    }
    #[test]
    fn compressed_restore_rejects_post_extract_marker_links_and_ambiguous_pax() {
        use std::io::Write;
        let host = HostId::from(uuid::Uuid::from_u128(3));
        for mode in [
            "absent",
            "regular",
            "symlink",
            "hardlink",
            "directory",
            "sparse-pax",
            "duplicate-pax",
        ] {
            let mut archive = tar::Builder::new(Vec::new());
            let inventory = format!("INSTALLATION_UUID='{host}'\nPRODUCT_VERSION='8.3.0'\n");
            let mut h = tar::Header::new_gnu();
            h.set_mode(0o600);
            h.set_size(inventory.len() as u64);
            h.set_cksum();
            archive
                .append_data(&mut h, "etc/xensource-inventory", inventory.as_bytes())
                .unwrap();
            if mode != "absent" {
                if mode == "sparse-pax" {
                    archive
                        .append_pax_extensions([("GNU.sparse.name", b"../../outside".as_slice())])
                        .unwrap();
                } else if mode == "duplicate-pax" {
                    archive
                        .append_pax_extensions([
                            ("path", b".xen-backup-partition".as_slice()),
                            ("path", b"other".as_slice()),
                        ])
                        .unwrap();
                }
                let mut h = tar::Header::new_gnu();
                h.set_size(0);
                h.set_mode(0o600);
                match mode {
                    "symlink" => {
                        h.set_entry_type(tar::EntryType::Symlink);
                        h.set_link_name("/outside/marker").unwrap();
                    }
                    "hardlink" => {
                        h.set_entry_type(tar::EntryType::Link);
                        h.set_link_name("etc/xensource-inventory").unwrap();
                    }
                    "directory" => h.set_entry_type(tar::EntryType::Directory),
                    _ => h.set_entry_type(tar::EntryType::Regular),
                }
                h.set_cksum();
                archive
                    .append_data(&mut h, ".xen-backup-partition", &[][..])
                    .unwrap();
            }
            let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gzip.write_all(&archive.into_inner().unwrap()).unwrap();
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&gzip.finish().unwrap()).unwrap();
            let result =
                super::super::archive::inspect_restore(&mut file, host, "8.3.0", None, None);
            assert_eq!(
                result.is_ok(),
                matches!(mode, "absent" | "regular"),
                "{mode}: {result:?}"
            );
        }
    }
}
