//! Offline, explicitly reviewed conversion. No connection or host command exists here.
use crate::{DiskImageFormat, transfer::files::Identity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[path = "local_conversion_files.rs"]
mod files;
#[path = "local_conversion_process.rs"]
mod process;
#[cfg(test)]
#[path = "local_conversion_tests.rs"]
mod tests;
use files::*;
type Result<T> = std::result::Result<T, String>;
fn fail(s: &str) -> String {
    s.into()
}
fn stopped(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(fail("Conversion cancelled"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionFormat {
    Qcow2,
    Vmdk,
    Vhdx,
    Vhd,
    Raw,
}
impl ConversionFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Qcow2 => "QCOW2",
            Self::Vmdk => "VMDK",
            Self::Vhdx => "VHDX",
            Self::Vhd => "VHD",
            Self::Raw => "Raw",
        }
    }
    fn wire(self) -> &'static str {
        match self {
            Self::Qcow2 => "qcow2",
            Self::Vmdk => "vmdk",
            Self::Vhdx => "vhdx",
            Self::Vhd => "vpc",
            Self::Raw => "raw",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ConversionTools {
    pub qemu: PathBuf,
}
impl Default for ConversionTools {
    fn default() -> Self {
        Self {
            qemu: "/usr/bin/qemu-img".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ConversionRequest {
    pub tools: ConversionTools,
    pub source: PathBuf,
    pub source_format: ConversionFormat,
    pub output: DiskImageFormat,
    pub destination: PathBuf,
}
#[derive(Clone)]
struct Tools {
    qemu: process::Executable,
    bwrap: process::Executable,
    prlimit: process::Executable,
}
impl Tools {
    fn inspect(options: &ConversionTools) -> Result<Self> {
        Ok(Self {
            qemu: process::Executable::inspect(&options.qemu)?,
            bwrap: process::Executable::inspect(Path::new("/usr/bin/bwrap"))?,
            prlimit: process::Executable::inspect(Path::new("/usr/bin/prlimit"))?,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        args: Vec<String>,
        input: Option<&Path>,
        output: Option<&Path>,
        write: bool,
        size: u64,
        deadline: Duration,
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>> {
        process::run(
            self,
            &args,
            process::Access {
                input,
                output,
                write_output: write,
                file_limit: size,
                deadline,
            },
            cancel,
        )
    }
    fn capabilities(
        &self,
        source: ConversionFormat,
        output: DiskImageFormat,
        cancel: &AtomicBool,
    ) -> Result<String> {
        let result = self.run(
            vec!["--version".into()],
            None,
            None,
            false,
            0,
            Duration::from_secs(30),
            cancel,
        )?;
        let version = std::str::from_utf8(&result)
            .map_err(|_| fail("Invalid QEMU version output"))?
            .lines()
            .next()
            .unwrap_or("");
        let major = version
            .strip_prefix("qemu-img version ")
            .and_then(|s| s.split('.').next())
            .and_then(|s| s.parse::<u32>().ok());
        if major.is_none_or(|m| !(8..=99).contains(&m)) || version.len() > 200 {
            return Err(fail(
                "QEMU 8 or later with supported JSON image metadata is required",
            ));
        }
        let help = self.run(
            vec!["--help".into()],
            None,
            None,
            false,
            0,
            Duration::from_secs(30),
            cancel,
        )?;
        let help =
            std::str::from_utf8(&help).map_err(|_| fail("Invalid QEMU capability output"))?;
        let formats = help
            .split_once("Supported image formats:")
            .or_else(|| help.split_once("Supported formats:"))
            .map(|(_, tail)| tail)
            .ok_or_else(|| fail("QEMU does not report supported formats"))?;
        let output_format = match output {
            DiskImageFormat::Raw => "raw",
            DiskImageFormat::Vhd => "vpc",
        };
        if [source.wire(), output_format]
            .iter()
            .any(|f| !formats.split_whitespace().any(|v| v == *f))
        {
            return Err(fail("This QEMU build lacks a required disk format"));
        }
        if output == DiskImageFormat::Vhd {
            let options = self.run(
                vec![
                    "create".into(),
                    "-f".into(),
                    "vpc".into(),
                    "-o".into(),
                    "help".into(),
                ],
                None,
                None,
                false,
                0,
                Duration::from_secs(30),
                cancel,
            )?;
            if !std::str::from_utf8(&options)
                .is_ok_and(|s| s.contains("force_size") && s.contains("subformat"))
            {
                return Err(fail("QEMU lacks exact-size dynamic VHD output options"));
            }
        }
        Ok(version.into())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionState {
    Preparing,
    Ready,
    Converting,
    Verifying,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversionSummary {
    pub id: uuid::Uuid,
    pub source: ConversionFormat,
    pub output: DiskImageFormat,
    pub virtual_size: Option<u64>,
    pub state: ConversionState,
    pub detail: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    summary: ConversionSummary,
    original: PathBuf,
    destination: PathBuf,
    working: PathBuf,
    working_identity: Identity,
    parent_identity: Identity,
    output_directory_identity: Identity,
    source_identity: Option<Identity>,
    source_hash: Option<[u8; 32]>,
    output_identity: Option<Identity>,
    output_hash: Option<[u8; 32]>,
    published_identity: Option<Identity>,
}
pub struct ConversionReview {
    record: Record,
    tools: Tools,
    version: String,
    directory: PathBuf,
    _permit: Permit,
}
impl ConversionReview {
    pub fn summary(&self) -> &ConversionSummary {
        &self.record.summary
    }
    pub fn qemu_version(&self) -> &str {
        &self.version
    }
    pub fn source_path(&self) -> &Path {
        &self.record.original
    }
    pub fn destination(&self) -> &Path {
        &self.record.destination
    }
    pub fn abandon(mut self) -> Result<()> {
        self.record.summary.state = ConversionState::Cancelled;
        self.record.summary.detail =
            "Review closed; no conversion or host import was submitted".into();
        save(&self.directory, &self.record)?;
        cleanup(&self.record)
    }
    pub fn convert(
        mut self,
        cancel: &AtomicBool,
        progress: impl Fn(ConversionSummary),
    ) -> Result<ConversionSummary> {
        let result = self.execute(cancel, &progress);
        if let Err(error) = &result {
            self.record.summary.state = if cancel.load(Ordering::Relaxed) {
                ConversionState::Cancelled
            } else {
                ConversionState::Failed
            };
            self.record.summary.detail = error.clone();
            let _ = save(&self.directory, &self.record);
        }
        let _ = cleanup(&self.record);
        progress(self.record.summary.clone());
        result.map(|_| self.record.summary.clone())
    }
    fn execute(
        &mut self,
        cancel: &AtomicBool,
        progress: &impl Fn(ConversionSummary),
    ) -> Result<()> {
        stopped(cancel)?;
        verify_working(&self.record)?;
        let source = self.record.working.join("source");
        verify_file(
            &source,
            self.record.source_identity.as_ref(),
            self.record.source_hash.as_ref(),
            cancel,
        )?;
        self.record.summary.state = ConversionState::Converting;
        self.record.summary.detail =
            "Converting the immutable local source in the restricted sandbox".into();
        save(&self.directory, &self.record)?;
        progress(self.record.summary.clone());
        let output_dir = self.record.working.join("output");
        let temporary = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(output_dir.join("image"))
            .map_err(|_| fail("Cannot create owned temporary output"))?;
        self.record.output_identity = Some(Identity::from(
            &temporary
                .metadata()
                .map_err(|_| fail("Cannot inspect temporary output"))?,
        ));
        save(&self.directory, &self.record)?;
        let size = self
            .record
            .summary
            .virtual_size
            .ok_or_else(|| fail("Missing reviewed virtual size"))?;
        let format = match self.record.summary.output {
            DiskImageFormat::Raw => "raw",
            DiskImageFormat::Vhd => "vpc",
        };
        let mut args = vec![
            "convert".into(),
            "-f".into(),
            self.record.summary.source.wire().into(),
            "-O".into(),
            format.into(),
            "-m".into(),
            "2".into(),
            "-r".into(),
            "128M".into(),
        ];
        if self.record.summary.output == DiskImageFormat::Vhd {
            args.extend(["-o".into(), "subformat=dynamic,force_size=on".into()]);
        }
        args.extend(["/input/source".into(), "/output/image".into()]);
        let limit = size
            .checked_add(size / 128)
            .and_then(|v| v.checked_add(16 * 1024 * 1024))
            .ok_or_else(|| fail("Output size exceeds conversion limit"))?;
        self.tools.run(
            args,
            Some(&source),
            Some(&output_dir),
            true,
            limit,
            Duration::from_secs(4 * 3600),
            cancel,
        )?;
        self.record.summary.state = ConversionState::Verifying;
        self.record.summary.detail =
            "Validating image structure and comparing complete guest-visible content".into();
        save(&self.directory, &self.record)?;
        progress(self.record.summary.clone());
        verify_working(&self.record)?;
        let output = output_dir.join("image");
        let mut file = open_regular(&output)?;
        let summary = crate::disk_image::inspect(&mut file, self.record.summary.output)
            .map_err(|_| fail("Converted image failed the native raw/VHD validator"))?;
        if summary.virtual_size != size {
            return Err(fail(
                "Converted image virtual size differs from the reviewed source",
            ));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o400))
            .map_err(|_| fail("Cannot protect the converted image"))?;
        let (identity, digest) = hash_file(&mut file, cancel)?;
        self.record.output_identity = Some(identity);
        self.record.output_hash = Some(digest);
        save(&self.directory, &self.record)?;
        self.tools.run(
            vec![
                "compare".into(),
                "-f".into(),
                self.record.summary.source.wire().into(),
                "-F".into(),
                format.into(),
                "/input/source".into(),
                "/output/image".into(),
            ],
            Some(&source),
            Some(&output_dir),
            false,
            0,
            Duration::from_secs(4 * 3600),
            cancel,
        )?;
        verify_file(
            &source,
            self.record.source_identity.as_ref(),
            self.record.source_hash.as_ref(),
            cancel,
        )?;
        verify_file(
            &output,
            self.record.output_identity.as_ref(),
            self.record.output_hash.as_ref(),
            cancel,
        )?;
        stopped(cancel)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| fail("Cannot set private published output permissions"))?;
        self.record.output_identity = Some(Identity::from(
            &file
                .metadata()
                .map_err(|_| fail("Cannot inspect output permissions"))?,
        ));
        save(&self.directory, &self.record)?;
        file.sync_all()
            .map_err(|_| fail("Cannot sync converted output"))?;
        publish(&mut self.record)?;
        save(&self.directory, &self.record)?;
        self.record.summary.state = ConversionState::Completed;
        self.record.summary.detail="Local conversion and content comparison verified. Import remains a separate reviewed action; guest boot compatibility is unverified.".into();
        save(&self.directory, &self.record)?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct LocalConversions {
    directory: PathBuf,
}
impl LocalConversions {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn prepare(
        &self,
        request: ConversionRequest,
        cancel: &AtomicBool,
        progress: impl Fn(ConversionSummary),
    ) -> Result<ConversionReview> {
        let ConversionRequest {
            tools,
            source,
            source_format,
            output,
            destination,
        } = request;
        private_dir(&self.directory)?;
        let permit = lock(&self.directory)?;
        let tools = Tools::inspect(&tools)?;
        let version = tools.capabilities(source_format, output, cancel)?;
        let id = uuid::Uuid::new_v4();
        let (original, destination, working) = prepare_paths(&source, &destination, id)?;
        let mut record = Record {
            summary: ConversionSummary {
                id,
                source: source_format,
                output,
                virtual_size: None,
                state: ConversionState::Preparing,
                detail: "Staging a private immutable source copy".into(),
            },
            parent_identity: Identity::from(
                &destination
                    .parent()
                    .ok_or_else(|| fail("Missing output parent"))?
                    .metadata()
                    .map_err(|_| fail("Cannot inspect output parent"))?,
            ),
            output_directory_identity: Identity::from(
                &working
                    .join("output")
                    .metadata()
                    .map_err(|_| fail("Cannot inspect output staging directory"))?,
            ),
            original,
            destination,
            working_identity: Identity::from(
                &working
                    .metadata()
                    .map_err(|_| fail("Cannot inspect staging directory"))?,
            ),
            working,
            source_identity: None,
            source_hash: None,
            output_identity: None,
            output_hash: None,
            published_identity: None,
        };
        save(&self.directory, &record)?;
        progress(record.summary.clone());
        let prepare = (|| {
            let input = record.working.join("source");
            let (identity, hash) = stage(&record.original, &input, cancel)?;
            record.source_identity = Some(identity);
            record.source_hash = Some(hash);
            save(&self.directory, &record)?;
            if source_format == ConversionFormat::Vhd {
                let mut staged = open_regular(&input)?;
                crate::disk_image::inspect(&mut staged, DiskImageFormat::Vhd).map_err(|_| {
                    fail("Only valid fixed/dynamic self-contained VHD images are supported")
                })?;
            }
            let json = tools.run(
                vec![
                    "info".into(),
                    "--output=json".into(),
                    "-f".into(),
                    source_format.wire().into(),
                    "/input/source".into(),
                ],
                Some(&input),
                None,
                false,
                0,
                Duration::from_secs(30),
                cancel,
            )?;
            let size = inspect_metadata(&json, source_format, output)?;
            verify_file(
                &input,
                record.source_identity.as_ref(),
                record.source_hash.as_ref(),
                cancel,
            )?;
            record.summary.virtual_size = Some(size);
            record.summary.state = ConversionState::Ready;
            record.summary.detail =
                "Source and tool capabilities inspected; review before conversion".into();
            save(&self.directory, &record)?;
            progress(record.summary.clone());
            Ok(())
        })();
        if let Err(error) = prepare {
            record.summary.state = if cancel.load(Ordering::Relaxed) {
                ConversionState::Cancelled
            } else {
                ConversionState::Failed
            };
            record.summary.detail = error;
            let _ = save(&self.directory, &record);
            let _ = cleanup(&record);
            return Err(record.summary.detail);
        }
        Ok(ConversionReview {
            record,
            tools,
            version,
            directory: self.directory.clone(),
            _permit: permit,
        })
    }
    pub fn history(&self) -> Result<Vec<ConversionSummary>> {
        history(&self.directory)
    }
    pub fn discard_temporary_files(&self, id: uuid::Uuid) -> Result<()> {
        private_dir(&self.directory)?;
        let _permit = lock(&self.directory)?;
        let mut record = load(&self.directory.join(format!("{id}.json")))?;
        if !matches!(
            record.summary.state,
            ConversionState::Failed | ConversionState::Cancelled | ConversionState::Interrupted
        ) {
            return Err(fail(
                "Only stopped conversion temporary files can be removed",
            ));
        }
        if record.working.exists() {
            cleanup(&record)?;
        }
        record.summary.detail="Stopped conversion; recorded temporary files removed where their owned identities still match. Original and published output were preserved.".into();
        save(&self.directory, &record)
    }
}

fn inspect_metadata(
    bytes: &[u8],
    format: ConversionFormat,
    output: DiskImageFormat,
) -> Result<u64> {
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| fail("QEMU returned invalid image metadata"))?;
    if !v.is_object()
        || v["format"] != format.wire()
        || v["encrypted"] == true
        || v["dirty-flag"] == true
        || [
            "backing-filename",
            "full-backing-filename",
            "backing-filename-format",
        ]
        .iter()
        .any(|key| v.get(*key).is_some())
    {
        return Err(fail(
            "Backing chains, encrypted images or unexpected formats require a separately supported workflow",
        ));
    }
    let size = v["virtual-size"]
        .as_u64()
        .filter(|n| *n > 0 && n.is_multiple_of(512) && *n <= output.limit())
        .ok_or_else(|| fail("Virtual disk size exceeds the selected output format limit"))?;
    if v["snapshots"].as_array().is_some_and(|v| !v.is_empty()) {
        return Err(fail(
            "Images with internal snapshots require explicit snapshot selection, which is not yet supported",
        ));
    }
    match format {
        ConversionFormat::Qcow2 => {
            let d = &v["format-specific"]["data"];
            if v["format-specific"]["type"] != "qcow2"
                || !d.is_object()
                || d.get("data-file").is_some()
                || d.get("encrypt").is_some()
                || d["corrupt"] == true
            {
                return Err(fail(
                    "QCOW2 external data, encryption or corruption is not supported",
                ));
            }
        }
        ConversionFormat::Vmdk => {
            let d = &v["format-specific"]["data"];
            let extents = d["extents"]
                .as_array()
                .ok_or_else(|| fail("VMDK extent metadata is unavailable"))?;
            if v["format-specific"]["type"] != "vmdk"
                || !matches!(
                    d["create-type"].as_str(),
                    Some("monolithicSparse" | "streamOptimized")
                )
                || d["parent-cid"].as_u64() != Some(u32::MAX as u64)
                || extents.len() != 1
                || extents[0]["filename"] != "/input/source"
                || extents[0]["virtual-size"].as_u64() != Some(size)
            {
                return Err(fail(
                    "Select a self-contained monolithic VMDK; parent images and multiple extents are not supported yet",
                ));
            }
        }
        _ => {}
    }
    Ok(size)
}
