use super::*;
use roxmltree::Node;
use std::collections::{BTreeMap, BTreeSet};
pub(super) const NS: &str = "http://schemas.dmtf.org/ovf/envelope/1";
const RASD: &str =
    "http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData";
const VSSD: &str = "http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_VirtualSystemSettingData";
fn err() -> Error {
    Error::Ovf("descriptor structure or hardware is not supported")
}
fn attr<'a>(n: Node<'a, '_>, key: &str) -> Result<&'a str, Error> {
    n.attribute((NS, key)).ok_or_else(err)
}
fn value<'a>(n: Node<'a, '_>, ns: &str, key: &str) -> Result<Option<&'a str>, Error> {
    let mut values = n.children().filter(|n| n.has_tag_name((ns, key)));
    let one = values.next();
    if values.next().is_some() {
        return Err(err());
    }
    Ok(one.map(|n| n.text().unwrap_or_default()))
}
fn text(n: Node<'_, '_>, key: &str) -> Result<String, Error> {
    Ok(value(n, NS, key)?.unwrap_or_default().to_string())
}
fn key(v: &str) -> Result<String, Error> {
    if v.is_empty() || v.len() > 255 || v.chars().any(char::is_control) {
        return Err(err());
    }
    Ok(v.into())
}
pub(super) fn safe_name(v: &str) -> Result<String, Error> {
    if v.is_empty()
        || v.len() > 240
        || v.starts_with('/')
        || v.contains(['\\', ':', '%', '\0'])
        || v.chars().any(char::is_control)
        || v.split('/').any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(Error::Ovf(
            "file references must stay inside the selected package",
        ));
    }
    Ok(v.into())
}
fn units(value: &str, unit: &str) -> Result<u64, Error> {
    let n = value.parse::<u64>().map_err(|_| err())?;
    let unit = unit
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    let multiplier = if unit == "byte" {
        1
    } else if let Some(power) = unit.strip_prefix("byte*2^") {
        1u64.checked_shl(power.parse().map_err(|_| err())?)
            .ok_or_else(err)?
    } else if let Some(power) = unit.strip_prefix("byte*10^") {
        10u64
            .checked_pow(power.parse().map_err(|_| err())?)
            .ok_or_else(err)?
    } else {
        return Err(Error::Ovf(
            "unsupported capacity or memory allocation units",
        ));
    };
    n.checked_mul(multiplier).ok_or_else(err)
}
fn section_safe(n: Node<'_, '_>) -> Result<(), Error> {
    let name = n.tag_name().name();
    if n.tag_name().namespace() != Some(NS) {
        if n.attribute((NS, "required")) == Some("false") {
            return Ok(());
        }
        return Err(Error::Ovf("unknown required extension"));
    }
    match name {
        "Info"
        | "Name"
        | "AnnotationSection"
        | "OperatingSystemSection"
        | "VirtualHardwareSection"
        | "References"
        | "DiskSection"
        | "NetworkSection"
        | "VirtualSystem"
        | "VirtualSystemCollection" => Ok(()),
        "ProductSection" if !n.descendants().any(|n| n.has_tag_name((NS, "Property"))) => Ok(()),
        _ => Err(Error::Ovf(
            "package properties, deployment options or required sections need an unsupported workflow",
        )),
    }
}
pub(super) type FileReferences = BTreeMap<String, (String, Option<u64>)>;
pub(super) fn parse(xml: &str) -> Result<(OvfSummary, FileReferences), Error> {
    if xml.len() > 8 * 1024 * 1024 {
        return Err(Error::TransferLimit);
    }
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
        },
    )
    .map_err(|_| Error::Ovf("invalid XML"))?;
    if doc.descendants().any(|n| n.ancestors().count() > 32) {
        return Err(Error::TransferLimit);
    }
    let root = doc.root_element();
    if !root.has_tag_name((NS, "Envelope")) {
        return Err(err());
    }
    for n in root.children().filter(|n| n.is_element()) {
        section_safe(n)?;
    }
    for name in ["References", "DiskSection", "NetworkSection"] {
        if root
            .children()
            .filter(|n| n.has_tag_name((NS, name)))
            .count()
            > 1
        {
            return Err(err());
        }
    }
    for collection in root
        .descendants()
        .filter(|n| n.has_tag_name((NS, "VirtualSystemCollection")))
    {
        for section in collection.children().filter(|n| n.is_element()) {
            section_safe(section)?;
            if section.tag_name().namespace() == Some(NS)
                && !matches!(
                    section.tag_name().name(),
                    "Info"
                        | "Name"
                        | "AnnotationSection"
                        | "VirtualSystem"
                        | "VirtualSystemCollection"
                )
            {
                return Err(err());
            }
        }
    }
    for (section, item) in [("DiskSection", "Disk"), ("NetworkSection", "Network")] {
        for n in root
            .children()
            .filter(|n| n.has_tag_name((NS, section)))
            .flat_map(|n| n.children().filter(|n| n.is_element()))
        {
            if !n.has_tag_name((NS, "Info")) && !n.has_tag_name((NS, item)) {
                return Err(err());
            }
        }
    }
    for n in root.descendants().filter(|n| n.is_element()) {
        if (n.has_tag_name((NS, "VirtualSystem"))
            || n.has_tag_name((NS, "VirtualSystemCollection")))
            && n.parent().is_none_or(|p| {
                !p.has_tag_name((NS, "Envelope"))
                    && !p.has_tag_name((NS, "VirtualSystemCollection"))
            })
        {
            return Err(err());
        }
        let allowed: Option<&[&str]> = match n.tag_name().namespace().zip(Some(n.tag_name().name()))
        {
            Some((NS, "OperatingSystemSection")) => Some(&["Info", "Description"]),
            Some((NS, "AnnotationSection")) => Some(&["Info", "Annotation"]),
            Some((NS, "Network")) => Some(&["Description"]),
            _ => None,
        };
        if let Some(allowed) = allowed {
            for child in n.children().filter(|n| n.is_element()) {
                if (child.tag_name().namespace() != Some(NS)
                    || !allowed.contains(&child.tag_name().name()))
                    && child.attribute((NS, "required")) != Some("false")
                {
                    return Err(err());
                }
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut paths = BTreeSet::new();
    for n in root
        .children()
        .filter(|n| n.has_tag_name((NS, "References")))
        .flat_map(|n| n.children().filter(|n| n.is_element()))
    {
        if !n.has_tag_name((NS, "File"))
            || n.attribute((NS, "compression"))
                .is_some_and(|v| v != "identity")
            || n.attribute((NS, "chunkSize")).is_some()
        {
            return Err(Error::Ovf(
                "compressed or chunked package files are unsupported",
            ));
        }
        let id = key(attr(n, "id")?)?;
        let path = safe_name(attr(n, "href")?)?;
        let size = n
            .attribute((NS, "size"))
            .map(|v| v.parse::<u64>().map_err(|_| err()))
            .transpose()?;
        if !paths.insert(path.clone())
            || files.insert(id, (path, size)).is_some()
            || files.len() > 256
        {
            return Err(err());
        }
    }
    let mut disks = Vec::new();
    let mut disk_ids = BTreeSet::new();
    let mut disk_files = BTreeSet::new();
    for n in root
        .children()
        .filter(|n| n.has_tag_name((NS, "DiskSection")))
        .flat_map(|n| n.children().filter(|n| n.has_tag_name((NS, "Disk"))))
    {
        let id = key(attr(n, "diskId")?)?;
        if !disk_ids.insert(id.clone())
            || disks.len() >= 256
            || n.attribute((NS, "parentRef")).is_some()
        {
            return Err(Error::Ovf(
                "duplicate disks or external/differencing parents are unsupported",
            ));
        }
        let virtual_size = units(
            attr(n, "capacity")?,
            n.attribute((NS, "capacityAllocationUnits"))
                .unwrap_or("byte"),
        )?;
        let file = n
            .attribute((NS, "fileRef"))
            .map(|id| files.get(id).map(|v| v.0.clone()).ok_or_else(err))
            .transpose()?;
        if file.as_ref().is_some_and(|f| !disk_files.insert(f.clone())) {
            return Err(err());
        }
        let format = match n.attribute((NS, "format")) {
            None if file.is_none() => DiskImageFormat::Raw,
            Some(
                "http://www.microsoft.com/technet/virtualserver/downloads/vhdspec.mspx"
                | "http://www.microsoft.com/technet/virtualserver/downloads/vhdspec.mspx#dynamic"
                | "http://www.microsoft.com/technet/virtualserver/downloads/vhdspec.mspx#fixed",
            ) => DiskImageFormat::Vhd,
            Some("http://raw" | "http://xenbits.xen.org/ovf/disk-format/raw") => {
                DiskImageFormat::Raw
            }
            _ => {
                return Err(Error::Ovf(
                    "only raw and complete fixed/dynamic VHD disks are supported; VMDK conversion is pending",
                ));
            }
        };
        if virtual_size == 0 || virtual_size > format.limit() || !virtual_size.is_multiple_of(512) {
            return Err(Error::Ovf(
                "disk capacity is outside the supported format limits",
            ));
        }
        disks.push(OvfDisk {
            key: id,
            file,
            virtual_size,
            format,
        });
    }
    if files.values().any(|(f, _)| !disk_files.contains(f)) {
        return Err(Error::Ovf(
            "unreferenced or auxiliary files are unsupported",
        ));
    }
    let mut networks = BTreeSet::new();
    for n in root
        .children()
        .filter(|n| n.has_tag_name((NS, "NetworkSection")))
        .flat_map(|n| n.children().filter(|n| n.has_tag_name((NS, "Network"))))
    {
        if !networks.insert(key(attr(n, "name")?)?) || networks.len() > 64 {
            return Err(err());
        }
    }
    let mut guests = Vec::new();
    let mut guest_ids = BTreeSet::new();
    for n in root
        .descendants()
        .filter(|n| n.has_tag_name((NS, "VirtualSystem")))
    {
        if !guest_ids.insert(key(attr(n, "id")?)?) || guests.len() >= 16 {
            return Err(Error::TransferLimit);
        }
        for section in n.children().filter(|n| n.is_element()) {
            section_safe(section)?;
        }
        let name = key(&text(n, "Name")?)?;
        let description = n
            .children()
            .find(|n| n.has_tag_name((NS, "AnnotationSection")))
            .map(|n| text(n, "Annotation"))
            .transpose()?
            .unwrap_or_default();
        if description.chars().count() > 4096 {
            return Err(err());
        }
        let mut hardware = n
            .children()
            .filter(|n| n.has_tag_name((NS, "VirtualHardwareSection")));
        let h = hardware.next().ok_or_else(err)?;
        if hardware.next().is_some() {
            return Err(Error::Ovf(
                "multiple alternative hardware profiles are unsupported",
            ));
        }
        let mut cpus = None;
        let mut memory = None;
        let mut firmware = GuestFirmware::Bios;
        let mut attachments = Vec::new();
        let mut nics = Vec::new();
        let mut instance_ids = BTreeSet::new();
        let mut controllers = BTreeSet::new();
        let mut disk_parents = Vec::new();
        for item in h.children().filter(|n| n.is_element()) {
            if item.has_tag_name((NS, "Info")) {
                continue;
            }
            if item.has_tag_name((NS, "System")) {
                for n in item.children().filter(|n| n.is_element()) {
                    if n.tag_name().namespace() != Some(VSSD)
                        || !matches!(
                            n.tag_name().name(),
                            "ElementName"
                                | "InstanceID"
                                | "VirtualSystemIdentifier"
                                | "VirtualSystemType"
                        )
                    {
                        return Err(err());
                    }
                }
                continue;
            }
            if !item.has_tag_name((NS, "Item")) {
                // Firmware has a narrow typed VMware-compatible extension; arbitrary configuration maps are refused.
                if item.tag_name().namespace() == Some("http://www.vmware.com/schema/ovf")
                    && item.tag_name().name() == "Config"
                    && item.attribute(("http://www.vmware.com/schema/ovf", "key"))
                        == Some("firmware")
                {
                    firmware = match item.attribute(("http://www.vmware.com/schema/ovf", "value")) {
                        Some("efi") => GuestFirmware::Uefi,
                        Some("bios") => GuestFirmware::Bios,
                        _ => return Err(err()),
                    };
                    continue;
                }
                if item.attribute((NS, "required")) == Some("false") {
                    continue;
                }
                return Err(Error::Ovf("unknown required virtual hardware extension"));
            }
            if item.attribute((NS, "configuration")).is_some() {
                return Err(Error::Ovf(
                    "conditional hardware profiles require deployment-option selection",
                ));
            }
            for field in item.children().filter(|n| n.is_element()) {
                if field.tag_name().namespace() != Some(RASD)
                    || !matches!(
                        field.tag_name().name(),
                        "ElementName"
                            | "Description"
                            | "Caption"
                            | "InstanceID"
                            | "ResourceType"
                            | "VirtualQuantity"
                            | "AllocationUnits"
                            | "Parent"
                            | "AddressOnParent"
                            | "ResourceSubType"
                            | "AutomaticAllocation"
                            | "Connection"
                            | "HostResource"
                            | "Address"
                    )
                {
                    if field.attribute((NS, "required")) == Some("false") {
                        continue;
                    }
                    return Err(Error::Ovf(
                        "unsupported resource constraints or required device fields",
                    ));
                }
            }
            if value(item, RASD, "AutomaticAllocation")?.is_some_and(|v| v != "true") {
                return Err(Error::Ovf("disconnected virtual devices are unsupported"));
            }
            let instance = value(item, RASD, "InstanceID")?.ok_or_else(err)?;
            if !instance_ids.insert(key(instance)?) {
                return Err(err());
            }
            let resource = value(item, RASD, "ResourceType")?.ok_or_else(err)?;
            match resource {
                "3" => {
                    if cpus.is_some() {
                        return Err(err());
                    }
                    let count = value(item, RASD, "VirtualQuantity")?
                        .ok_or_else(err)?
                        .parse::<u32>()
                        .map_err(|_| err())?;
                    if !(1..=256).contains(&count) {
                        return Err(err());
                    }
                    cpus = Some(count)
                }
                "4" => {
                    if memory.is_some() {
                        return Err(err());
                    }
                    let size = units(
                        value(item, RASD, "VirtualQuantity")?.ok_or_else(err)?,
                        value(item, RASD, "AllocationUnits")?.ok_or_else(err)?,
                    )?;
                    if !(16 * 1024 * 1024..=8 * 1024 * 1024 * 1024 * 1024).contains(&size)
                        || !size.is_multiple_of(1024)
                    {
                        return Err(err());
                    }
                    memory = Some(size)
                }
                "5" => {
                    if value(item, RASD, "ResourceSubType")?.is_some_and(|s| !s.is_empty())
                        || !controllers.insert(instance.to_string())
                        || controllers.len() > 1
                    {
                        return Err(Error::Ovf(
                            "only a single standard IDE controller without a custom subtype can be represented",
                        ));
                    }
                }
                "6" | "20" => {
                    return Err(Error::Ovf(
                        "SCSI and SATA controller models require an explicit supported controller mapping",
                    ));
                }
                "17" => {
                    if let Some(parent) = value(item, RASD, "Parent")? {
                        disk_parents.push(parent.to_string());
                    }
                    let reference = value(item, RASD, "HostResource")?.ok_or_else(err)?;
                    let id = reference
                        .strip_prefix("ovf:/disk/")
                        .or_else(|| reference.strip_prefix("/disk/"))
                        .ok_or_else(err)?;
                    if !disk_ids.contains(id)
                        || attachments.iter().any(|d: &OvfAttachment| d.disk == id)
                        || attachments.len() >= 64
                    {
                        return Err(err());
                    }
                    let device = value(item, RASD, "AddressOnParent")?
                        .map(|v| v.parse::<u32>().map_err(|_| err()))
                        .transpose()?
                        .unwrap_or(attachments.len() as u32);
                    if device > 63 || attachments.iter().any(|d| d.device == device) {
                        return Err(err());
                    }
                    attachments.push(OvfAttachment {
                        disk: id.into(),
                        device,
                    });
                }
                "10" => {
                    let network = value(item, RASD, "Connection")?.ok_or_else(err)?;
                    if !networks.contains(network) || nics.len() >= 16 {
                        return Err(err());
                    }
                    let device = value(item, RASD, "AddressOnParent")?
                        .map(|v| v.parse::<u32>().map_err(|_| err()))
                        .transpose()?
                        .unwrap_or(nics.len() as u32);
                    if device > 15 || nics.iter().any(|n: &OvfNic| n.device == device) {
                        return Err(err());
                    }
                    nics.push(OvfNic {
                        network: network.into(),
                        device,
                    })
                }
                _ => {
                    return Err(Error::Ovf(
                        "unsupported virtual device; import would lose guest configuration",
                    ));
                }
            }
        }
        if disk_parents
            .iter()
            .any(|parent| !controllers.contains(parent))
        {
            return Err(Error::Ovf(
                "disk parent does not identify a supported controller",
            ));
        }
        attachments.sort_by_key(|a| a.device);
        nics.sort_by_key(|a| a.device);
        guests.push(OvfGuest {
            key: attr(n, "id")?.into(),
            name,
            description,
            cpus: cpus.ok_or_else(err)?,
            memory_bytes: memory.ok_or_else(err)?,
            firmware,
            disks: attachments,
            nics,
        });
    }
    if guests.is_empty()
        || disks.iter().any(|d| {
            !guests
                .iter()
                .any(|g| g.disks.iter().any(|a| a.disk == d.key))
        })
    {
        return Err(err());
    }
    Ok((
        OvfSummary {
            guests,
            disks,
            networks: networks.into_iter().collect(),
            manifest_verified: false,
        },
        files,
    ))
}
