use crate::{DiskImageFormat, Error, GuestFirmware, TransferFile};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[path = "ovf_descriptor.rs"]
mod descriptor;
#[path = "ovf_package.rs"]
pub(crate) mod package;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OvfPackageFormat {
    Ova,
    Ovf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvfSummary {
    pub guests: Vec<OvfGuest>,
    pub disks: Vec<OvfDisk>,
    pub networks: Vec<String>,
    pub manifest_verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvfGuest {
    pub key: String,
    pub name: String,
    pub description: String,
    pub cpus: u32,
    pub memory_bytes: u64,
    pub firmware: GuestFirmware,
    pub disks: Vec<OvfAttachment>,
    pub nics: Vec<OvfNic>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvfDisk {
    pub key: String,
    pub file: Option<String>,
    pub virtual_size: u64,
    pub format: DiskImageFormat,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvfAttachment {
    pub disk: String,
    pub device: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvfNic {
    pub network: String,
    pub device: u32,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Selection {
    pub summary: OvfSummary,
    pub descriptor: String,
    pub files: BTreeMap<String, TransferFile>,
    pub digest: [u8; 32],
}
pub(crate) fn select(path: &Path) -> Result<TransferFile, Error> {
    package::select(path)
}
#[cfg(test)]
#[path = "ovf_tests.rs"]
pub(crate) mod tests;

#[path = "ovf_import.rs"]
pub(crate) mod import;

#[path = "ovf_export.rs"]
pub(crate) mod export;
#[path = "ovf_output.rs"]
pub(crate) mod output;
