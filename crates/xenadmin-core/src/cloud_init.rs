use crate::{DiskImageFormat, Error, TransferFile, VmId};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::{DirBuilder, File, OpenOptions},
    io::{Cursor, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[path = "cloud_init_journal.rs"]
mod journal;
pub(crate) use journal::validate_record;

pub(crate) const MEDIA_BYTES: usize = 8 * 1024 * 1024;
const USER_DATA_LIMIT: usize = 2 * 1024 * 1024;
const NETWORK_LIMIT: usize = 1024 * 1024;

/// Runtime-only input. User-data is never part of an operation's public record.
pub struct CloudInitInputs {
    hostname: String,
    user_data: Zeroizing<String>,
    network_config: Option<Zeroizing<String>>,
}
impl fmt::Debug for CloudInitInputs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CloudInitInputs([private])")
    }
}
impl CloudInitInputs {
    pub fn new(
        hostname: String,
        user_data: String,
        network_config: Option<String>,
    ) -> Result<Self, Error> {
        let user_data = Zeroizing::new(user_data);
        let network_config = network_config.map(Zeroizing::new);
        Self::validate(
            &hostname,
            &user_data,
            network_config.as_deref().map(String::as_str),
        )?;
        Ok(Self {
            hostname,
            user_data,
            network_config,
        })
    }
    pub fn validate(
        hostname: &str,
        user_data: &str,
        network_config: Option<&str>,
    ) -> Result<(), Error> {
        let valid_hostname = !hostname.is_empty()
            && hostname.len() <= 253
            && hostname.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        let valid_text = |text: &str, limit: usize| {
            !text.trim().is_empty() && text.len() <= limit && !text.contains('\0')
        };
        if !valid_hostname
            || !valid_text(user_data, USER_DATA_LIMIT)
            || network_config.is_some_and(|s| !valid_text(s, NETWORK_LIMIT))
        {
            return Err(Error::InvalidCloudInit);
        }
        Ok(())
    }
}

/// Prepared local media, consumed by one reviewed operation. Dropping an unused draft
/// removes its private artifact. No payload or local path is exposed by Debug/serde.
pub struct CloudInitMedia {
    pub(crate) vm: VmId,
    pub(crate) id: Uuid,
    pub(crate) file: TransferFile,
    pub(crate) sha256: [u8; 32],
    retained: bool,
}
impl fmt::Debug for CloudInitMedia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CloudInitMedia([private])")
    }
}
impl CloudInitMedia {
    /// Performs bounded local filesystem work; call from a background/blocking worker.
    pub fn prepare(vm: VmId, inputs: CloudInitInputs) -> Result<Self, Error> {
        Self::prepare_in(vm, inputs, &std::env::temp_dir())
    }
    fn prepare_in(vm: VmId, inputs: CloudInitInputs, parent: &Path) -> Result<Self, Error> {
        let id = Uuid::new_v4();
        let mut bytes = Zeroizing::new(vec![0_u8; MEDIA_BYTES]);
        write_volume(&mut bytes, id, &inputs).map_err(|_| Error::CloudInitMedia)?;
        let sha256 = Sha256::digest(&bytes).into();
        let directory = parent.join(format!("xenadmin-nocloud-{id}"));
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| Error::CloudInitMedia)?;
        let path = directory.join("seed.img");
        let mut created = false;
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
                .open(&path)
                .map_err(|_| Error::CloudInitMedia)?;
            created = true;
            output
                .write_all(&bytes)
                .map_err(|_| Error::CloudInitMedia)?;
            output.sync_all().map_err(|_| Error::CloudInitMedia)?;
            File::open(&directory)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| Error::CloudInitMedia)?;
            TransferFile::disk_source(&path, DiskImageFormat::Raw)
        })();
        match result {
            Ok(file) => Ok(Self {
                vm,
                id,
                file,
                sha256,
                retained: false,
            }),
            Err(error) => {
                if created {
                    let _ = std::fs::remove_file(&path);
                }
                let _ = std::fs::remove_dir(&directory);
                Err(error)
            }
        }
    }
    pub fn configuration_id(&self) -> Uuid {
        self.id
    }
    pub fn size(&self) -> u64 {
        MEDIA_BYTES as u64
    }
    pub(crate) fn retain(&mut self) {
        self.retained = true;
    }
}
impl Drop for CloudInitMedia {
    fn drop(&mut self) {
        if !self.retained {
            remove_artifact(&self.file);
        }
    }
}
pub(crate) fn remove_artifact(file: &TransferFile) {
    // A replaced file/directory is not ours to remove.
    if crate::transfer::files::source(file).is_err() {
        return;
    }
    let Some(parent) = file.path.parent() else {
        return;
    };
    let Ok(metadata) = parent.symlink_metadata() else {
        return;
    };
    if !metadata.is_dir() || metadata.mode() & 0o777 != 0o700 {
        return;
    }
    let _ = std::fs::remove_file(&file.path);
    let _ = std::fs::remove_dir(parent);
}
fn write_volume(bytes: &mut [u8], id: Uuid, inputs: &CloudInitInputs) -> std::io::Result<()> {
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat16)
        .volume_label(*b"CIDATA     ")
        .volume_id(u32::from_le_bytes(id.as_bytes()[..4].try_into().unwrap()));
    fatfs::format_volume(Cursor::new(&mut *bytes), options)?;
    let filesystem = fatfs::FileSystem::new(Cursor::new(bytes), fatfs::FsOptions::new())?;
    {
        let root = filesystem.root_dir();
        let metadata = serde_json::to_vec(&serde_json::json!({
            "instance-id": format!("xenadmin-{id}"),
            "local-hostname": inputs.hostname,
        }))?;
        root.create_file("meta-data")?.write_all(&metadata)?;
        root.create_file("user-data")?
            .write_all(inputs.user_data.as_bytes())?;
        if let Some(network) = &inputs.network_config {
            root.create_file("network-config")?
                .write_all(network.as_bytes())?;
        }
    }
    filesystem.unmount()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn vm() -> VmId {
        VmId::parse(&Uuid::new_v4().to_string()).unwrap()
    }
    fn inputs() -> CloudInitInputs {
        CloudInitInputs::new(
            "fixture-vm".into(),
            "#cloud-config\npassword: private-fixture-only\n".into(),
            Some("version: 2\nethernets: {}\n".into()),
        )
        .unwrap()
    }
    #[test]
    fn media_is_private_labelled_and_contains_exact_root_files() {
        let parent = tempfile::tempdir().unwrap();
        let media = CloudInitMedia::prepare_in(vm(), inputs(), parent.path()).unwrap();
        assert_eq!(media.file.path.metadata().unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            media.file.path.parent().unwrap().metadata().unwrap().mode() & 0o777,
            0o700
        );
        let bytes = std::fs::read(&media.file.path).unwrap();
        assert_eq!(bytes.len(), MEDIA_BYTES);
        assert_eq!(<[u8; 32]>::from(Sha256::digest(&bytes)), media.sha256);
        let fs = fatfs::FileSystem::new(Cursor::new(bytes), fatfs::FsOptions::new()).unwrap();
        assert_eq!(fs.fat_type(), fatfs::FatType::Fat16);
        assert_eq!(fs.volume_label(), "CIDATA");
        let root = fs.root_dir();
        let mut data = String::new();
        root.open_file("user-data")
            .unwrap()
            .read_to_string(&mut data)
            .unwrap();
        assert_eq!(data, *inputs().user_data);
        data.clear();
        root.open_file("network-config")
            .unwrap()
            .read_to_string(&mut data)
            .unwrap();
        assert_eq!(data, **inputs().network_config.as_ref().unwrap());
        data.clear();
        root.open_file("meta-data")
            .unwrap()
            .read_to_string(&mut data)
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(metadata["instance-id"], format!("xenadmin-{}", media.id));
        assert_eq!(metadata["local-hostname"], "fixture-vm");
        assert!(!format!("{media:?} {:?}", inputs()).contains("private-fixture-only"));
        let path = media.file.path.clone();
        drop(media);
        assert!(!path.exists());
        assert_eq!(parent.path().read_dir().unwrap().count(), 0);
    }
    #[test]
    fn invalid_or_oversize_inputs_are_rejected_before_artifact_creation() {
        for hostname in ["", "-bad", "bad-", "a..b", "a/b", "a\nkey: value", "_bad"] {
            assert!(CloudInitInputs::new(hostname.into(), "#cloud-config".into(), None).is_err());
        }
        for data in [
            String::new(),
            "\0secret".into(),
            "x".repeat(USER_DATA_LIMIT + 1),
        ] {
            assert!(CloudInitInputs::new("valid".into(), data, None).is_err());
        }
        assert!(
            CloudInitInputs::new(
                "valid".into(),
                "#cloud-config".into(),
                Some("x".repeat(NETWORK_LIMIT + 1))
            )
            .is_err()
        );
    }
    #[test]
    fn optional_network_is_absent_and_maximum_inputs_fit() {
        let parent = tempfile::tempdir().unwrap();
        let inputs = CloudInitInputs::new(
            "fixture".into(),
            "x".repeat(USER_DATA_LIMIT),
            Some("n".repeat(NETWORK_LIMIT)),
        )
        .unwrap();
        let media = CloudInitMedia::prepare_in(vm(), inputs, parent.path()).unwrap();
        let bytes = std::fs::read(&media.file.path).unwrap();
        let fs = fatfs::FileSystem::new(Cursor::new(bytes), fatfs::FsOptions::new()).unwrap();
        let mut data = Vec::new();
        fs.root_dir()
            .open_file("user-data")
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();
        assert_eq!(data.len(), USER_DATA_LIMIT);
        let inputs =
            CloudInitInputs::new("fixture".into(), "#!/bin/sh\ntrue\n".into(), None).unwrap();
        let media = CloudInitMedia::prepare_in(vm(), inputs, parent.path()).unwrap();
        let bytes = std::fs::read(&media.file.path).unwrap();
        let fs = fatfs::FileSystem::new(Cursor::new(bytes), fatfs::FsOptions::new()).unwrap();
        assert!(fs.root_dir().open_file("network-config").is_err());
    }
}

pub(crate) const MARKER: &str = "xenadmin-rs-cloud-init";
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CloudInitConfiguration {
    pub sr: crate::SrId,
    pub device: String,
    pub configuration_id: Uuid,
}
impl CloudInitConfiguration {
    pub(crate) fn valid(&self) -> bool {
        !self.configuration_id.is_nil()
            && self
                .device
                .parse::<u8>()
                .is_ok_and(|d| d.to_string() == self.device)
    }
    pub fn availability(
        &self,
        inventory: &crate::Inventory,
        vm: VmId,
    ) -> crate::ActionAvailability {
        use crate::{ActionAvailability, ClassAvailability, PowerState};
        let no = |s: &str| ActionAvailability::Prerequisite { reason: s.into() };
        if !self.valid() {
            return no("Choose a disk slot from 0 to 255");
        }
        if ["vbd", "vdi", "sr"]
            .iter()
            .any(|c| inventory.details.classes.get(*c) != Some(&ClassAvailability::Available))
        {
            return ActionAvailability::Unknown;
        }
        if inventory.vms.get(&vm).is_none_or(|v| {
            v.is_template || v.is_control_domain || v.power_state != PowerState::Halted
        }) {
            return no("Shut down an ordinary VM before preparing its cloud-init drive");
        }
        if inventory
            .details
            .vms
            .get(&vm)
            .is_none_or(|v| !v.current_operations.is_empty())
        {
            return no("Wait for the VM's current operations to finish");
        }
        if inventory
            .details
            .vbds
            .values()
            .any(|v| v.vm == Some(vm) && v.userdevice == self.device)
        {
            return no("Choose an unused disk slot");
        }
        self.transfer_action().availability(inventory)
    }
    pub(crate) fn transfer_action(&self) -> crate::TransferAction {
        crate::TransferAction::ImportDisk {
            sr: self.sr,
            name: format!("Cloud-init seed {}", self.configuration_id),
            format: DiskImageFormat::Raw,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Attachment {
    pub baseline: Vec<Uuid>,
    pub reference: Option<String>,
    pub uuid: Option<crate::VbdId>,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Plan {
    pub operation: crate::OperationId,
    pub pool: crate::PoolId,
    pub vm: VmId,
    pub configuration: CloudInitConfiguration,
    pub sha256: [u8; 32],
    pub interrupted: bool,
    pub upload_verified: bool,
    pub attachment: Option<Attachment>,
}
impl Plan {
    pub fn accepts(&self, next: &Self) -> bool {
        self.operation == next.operation
            && self.pool == next.pool
            && self.vm == next.vm
            && self.configuration == next.configuration
            && self.sha256 == next.sha256
            && (!self.upload_verified || next.upload_verified)
            && (!self.interrupted || next.interrupted)
            && match (&self.attachment, &next.attachment) {
                (None, _) => true,
                (Some(a), Some(b)) => {
                    a.baseline == b.baseline
                        && (a.reference.is_none() || a.reference == b.reference)
                        && (a.uuid.is_none() || a.uuid == b.uuid)
                        && (!a.verified || b.verified)
                }
                _ => false,
            }
    }
}
pub(crate) fn verify_artifact(file: &TransferFile, expected: [u8; 32]) -> Result<(), Error> {
    use std::io::Read;
    let mut input = crate::transfer::files::source(file)?;
    if input.metadata().map_err(|_| Error::CloudInitMedia)?.mode() & 0o777 != 0o600 {
        return Err(Error::CloudInitMedia);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(MEDIA_BYTES));
    (&mut input)
        .take(MEDIA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::CloudInitMedia)?;
    crate::transfer::files::source_unchanged(file, &input)?;
    if bytes.len() != MEDIA_BYTES || <[u8; 32]>::from(Sha256::digest(&bytes)) != expected {
        return Err(Error::TransferFileChanged);
    }
    Ok(())
}
