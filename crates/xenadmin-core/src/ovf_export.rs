use super::*;
use crate::{
    ExportSource, OperationId, PoolId, SrId, TransferAction, VdiId,
    transfer::{self, Plan as Stream},
    xapi::Session,
};
use serde_json::{Value, json};
use uuid::Uuid;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DiskSource {
    pub id: VdiId,
    pub sr: SrId,
    pub size: u64,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Captured {
    pub summary: OvfSummary,
    pub disks: Vec<DiskSource>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Step {
    pub token: OperationId,
    pub task: Option<String>,
    pub task_dispatched: bool,
    pub cancel_dispatched: bool,
    pub stream: Stream,
    pub complete: bool,
}
impl Step {
    fn accepts(&self, n: &Self) -> bool {
        self.token == n.token
            && (self.task.is_none() || self.task == n.task)
            && (!self.task_dispatched || n.task_dispatched)
            && (!self.cancel_dispatched || n.cancel_dispatched)
            && (!self.complete || n.complete)
            && transfer::plan_accepts(&self.stream, &n.stream)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExportPlan {
    pub captured: Captured,
    pub directory: super::output::Directory,
    pub task_baseline: Vec<Uuid>,
    pub steps: Vec<Step>,
    pub published: bool,
}
impl ExportPlan {
    pub(crate) fn accepts(&self, n: &Self) -> bool {
        self.captured == n.captured
            && self.directory == n.directory
            && self.task_baseline == n.task_baseline
            && self.steps.len() == n.steps.len()
            && self.steps.iter().zip(&n.steps).all(|(a, b)| a.accepts(b))
            && (!self.published || n.published)
    }
}
fn num(v: &Value) -> Result<u64, Error> {
    v.as_u64()
        .or_else(|| v.as_str()?.parse().ok())
        .ok_or(Error::TransferConflict)
}
fn text(v: &Value, max: usize) -> Result<String, Error> {
    v.as_str()
        .filter(|s| s.chars().count() <= max && !s.chars().any(|c| c == '\0'))
        .map(str::to_owned)
        .ok_or(Error::TransferConflict)
}
async fn record(s: &Session, class: &str, r: &Value) -> Result<Value, Error> {
    let reference = crate::snapshots::reference(r)?;
    s.call(&format!("{class}.get_record"), vec![json!(reference)])
        .await
}
fn refs(v: &Value) -> Result<&Vec<Value>, Error> {
    v.as_array()
        .filter(|a| a.len() <= 256)
        .ok_or(Error::TransferConflict)
}
pub(crate) async fn capture(
    s: &Session,
    pool: PoolId,
    source: &ExportSource,
) -> Result<Captured, Error> {
    transfer::xapi::prepare(
        s,
        pool,
        &TransferAction::ExportXva {
            compression: crate::XvaCompression::Uncompressed,
            source: source.clone(),
        },
        None,
    )
    .await?;
    let id = match source {
        ExportSource::Vm { vm, .. } => vm.uuid(),
        ExportSource::Snapshot { snapshot, .. } => snapshot.uuid(),
        ExportSource::Checkpoint { .. } => return Err(Error::TransferConflict),
    };
    let (reference, v) = super::import::fresh(s, "VM", id).await?;
    let cpus = u32::try_from(num(&v["VCPUs_at_startup"])?)
        .ok()
        .filter(|n| (1..=256).contains(n))
        .ok_or(Error::TransferConflict)?;
    let memory_bytes = num(&v["memory_static_max"])?;
    if !(16 * 1024 * 1024..=8 * 1024 * 1024 * 1024 * 1024).contains(&memory_bytes)
        || !memory_bytes.is_multiple_of(1024)
        || v["HVM_boot_policy"] != "BIOS order"
        || v["HVM_boot_params"]["order"] != "dc"
        || v["platform"]["secureboot"]
            .as_str()
            .is_some_and(|v| v != "false")
    {
        return Err(Error::Ovf(
            "OVF export currently requires ordinary HVM guests with dc boot order and Secure Boot disabled",
        ));
    }
    for field in ["VGPUs", "attached_PCIs", "VUSBs"] {
        if v.get(field)
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err(Error::Ovf(
                "specialized guest devices cannot be represented in this package",
            ));
        }
    }
    let firmware = match v["HVM_boot_params"]["firmware"].as_str() {
        None | Some("bios") => GuestFirmware::Bios,
        Some("uefi") => GuestFirmware::Uefi,
        _ => return Err(Error::TransferConflict),
    };
    let mut disk_sources = vec![];
    let mut disks = vec![];
    let mut attachments = vec![];
    let mut device_ids = std::collections::BTreeSet::new();
    let mut source_ids = std::collections::BTreeSet::new();
    let mut vbds = vec![];
    for r in refs(&v["VBDs"])? {
        vbds.push(record(s, "VBD", r).await?)
    }
    vbds.sort_by_key(|d| num(&d["userdevice"]).unwrap_or(u64::MAX));
    for d in vbds {
        if d["VM"] != reference
            || d["currently_attached"] != false
            || d["current_operations"]
                .as_object()
                .is_none_or(|m| !m.is_empty())
        {
            return Err(Error::TransferConflict);
        }
        if d["type"] == "CD" && d["empty"] == true {
            continue;
        }
        if d["type"] != "Disk" || d["empty"] != false || d["mode"] != "RW" {
            return Err(Error::Ovf(
                "eject installation media; read-only and non-disk devices are not supported in OVF export",
            ));
        }
        let device = u32::try_from(num(&d["userdevice"])?)
            .ok()
            .filter(|n| *n <= 63)
            .ok_or(Error::TransferConflict)?;
        let vdi = record(s, "VDI", &d["VDI"]).await?;
        let sr = record(s, "SR", &vdi["SR"]).await?;
        let id: VdiId = vdi["uuid"]
            .as_str()
            .and_then(|v| v.parse::<Uuid>().ok())
            .ok_or(Error::TransferConflict)?
            .into();
        let sr: SrId = sr["uuid"]
            .as_str()
            .and_then(|v| v.parse::<Uuid>().ok())
            .ok_or(Error::TransferConflict)?
            .into();
        if !device_ids.insert(device) || !source_ids.insert(id) {
            return Err(Error::TransferConflict);
        }
        let size = num(&vdi["virtual_size"])?;
        transfer::disk::disk_record(s, id, sr, DiskImageFormat::Vhd, false).await?;
        let index = disks.len();
        let key = format!("disk-{index}");
        disk_sources.push(DiskSource { id, sr, size });
        disks.push(OvfDisk {
            key: key.clone(),
            file: Some(format!("disk-{index}.vhd")),
            virtual_size: size,
            format: DiskImageFormat::Vhd,
        });
        attachments.push(OvfAttachment { disk: key, device });
    }
    let mut networks = std::collections::BTreeSet::new();
    let mut nics = vec![];
    let mut devices = std::collections::BTreeSet::new();
    for r in refs(&v["VIFs"])? {
        let nic = record(s, "VIF", r).await?;
        let n = record(s, "network", &nic["network"]).await?;
        if nic["VM"] != reference
            || nic["currently_attached"] != false
            || nic["current_operations"]
                .as_object()
                .is_none_or(|m| !m.is_empty())
        {
            return Err(Error::TransferConflict);
        }
        let id = n["uuid"]
            .as_str()
            .and_then(|v| v.parse::<Uuid>().ok())
            .ok_or(Error::TransferConflict)?;
        let network = format!("network-{id}");
        let device = u32::try_from(num(&nic["device"])?)
            .ok()
            .filter(|n| *n <= 15)
            .ok_or(Error::TransferConflict)?;
        if !devices.insert(device) {
            return Err(Error::TransferConflict);
        }
        networks.insert(network.clone());
        nics.push(OvfNic { network, device });
    }
    nics.sort_by_key(|n| n.device);
    let summary = OvfSummary {
        guests: vec![OvfGuest {
            key: "guest-1".into(),
            name: text(&v["name_label"], 255)?,
            description: text(&v["name_description"], 4096)?,
            cpus,
            memory_bytes,
            firmware,
            disks: attachments,
            nics,
        }],
        disks,
        networks: networks.into_iter().collect(),
        manifest_verified: true,
    };
    if summary
        .disks
        .iter()
        .try_fold(0u64, |n, d| n.checked_add(d.virtual_size))
        .is_none_or(|n| n > transfer::MAX_TRANSFER_BYTES)
    {
        return Err(Error::TransferLimit);
    }
    Ok(Captured {
        summary,
        disks: disk_sources,
    })
}
pub(crate) fn descriptor(summary: &OvfSummary, sizes: &[u64]) -> Result<String, Error> {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
    if sizes.len() != summary.disks.len() {
        return Err(Error::TransferConflict);
    }
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1" xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1" xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData" xmlns:vmw="http://www.vmware.com/schema/ovf"><References>"#,
    );
    for (i, d) in summary.disks.iter().enumerate() {
        out += &format!(
            "<File ovf:id=\"file-{i}\" ovf:href=\"{}\" ovf:size=\"{}\"/>",
            esc(d.file.as_deref().ok_or(Error::TransferConflict)?),
            sizes[i]
        );
    }
    out += "</References><DiskSection><Info>Exported data disks</Info>";
    for (i, d) in summary.disks.iter().enumerate() {
        out += &format!(
            "<Disk ovf:diskId=\"{}\" ovf:fileRef=\"file-{i}\" ovf:capacity=\"{}\" ovf:format=\"http://www.microsoft.com/technet/virtualserver/downloads/vhdspec.mspx\"/>",
            esc(&d.key),
            d.virtual_size
        );
    }
    out += "</DiskSection><NetworkSection><Info>Map these networks when importing</Info>";
    for n in &summary.networks {
        out += &format!("<Network ovf:name=\"{}\"/>", esc(n));
    }
    out += "</NetworkSection>";
    for g in &summary.guests {
        out += &format!(
            "<VirtualSystem ovf:id=\"{}\"><Name>{}</Name><AnnotationSection><Info>Description</Info><Annotation>{}</Annotation></AnnotationSection><VirtualHardwareSection><Info>Ordinary HVM guest</Info><vmw:Config ovf:required=\"true\" vmw:key=\"firmware\" vmw:value=\"{}\"/><Item><rasd:InstanceID>1</rasd:InstanceID><rasd:ResourceType>3</rasd:ResourceType><rasd:VirtualQuantity>{}</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>2</rasd:InstanceID><rasd:ResourceType>4</rasd:ResourceType><rasd:AllocationUnits>byte</rasd:AllocationUnits><rasd:VirtualQuantity>{}</rasd:VirtualQuantity></Item>",
            esc(&g.key),
            esc(&g.name),
            esc(&g.description),
            if g.firmware == GuestFirmware::Uefi {
                "efi"
            } else {
                "bios"
            },
            g.cpus,
            g.memory_bytes
        );
        let mut instance = 3;
        for a in &g.disks {
            out += &format!(
                "<Item><rasd:InstanceID>{instance}</rasd:InstanceID><rasd:ResourceType>17</rasd:ResourceType><rasd:AddressOnParent>{}</rasd:AddressOnParent><rasd:HostResource>ovf:/disk/{}</rasd:HostResource></Item>",
                a.device,
                esc(&a.disk)
            );
            instance += 1;
        }
        for n in &g.nics {
            out += &format!(
                "<Item><rasd:InstanceID>{instance}</rasd:InstanceID><rasd:ResourceType>10</rasd:ResourceType><rasd:AddressOnParent>{}</rasd:AddressOnParent><rasd:Connection>{}</rasd:Connection></Item>",
                n.device,
                esc(&n.network)
            );
            instance += 1;
        }
        out += "</VirtualHardwareSection></VirtualSystem>";
    }
    out += "</Envelope>";
    super::descriptor::parse(&out)?;
    Ok(out)
}
