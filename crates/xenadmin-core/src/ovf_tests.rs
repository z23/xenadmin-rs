use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;
pub(crate) fn descriptor(file: &str, format: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1" xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1" xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData"><References><File ovf:id="f1" ovf:href="{file}"/></References><DiskSection><Info>Disks</Info><Disk ovf:diskId="d1" ovf:fileRef="f1" ovf:capacity="4096" ovf:format="{format}"/></DiskSection><NetworkSection><Info>Networks</Info><Network ovf:name="Guest network"/></NetworkSection><VirtualSystem ovf:id="vm1"><Name>Imported guest</Name><VirtualHardwareSection><Info>Hardware</Info><Item><rasd:InstanceID>1</rasd:InstanceID><rasd:ResourceType>3</rasd:ResourceType><rasd:VirtualQuantity>2</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>2</rasd:InstanceID><rasd:ResourceType>4</rasd:ResourceType><rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits><rasd:VirtualQuantity>512</rasd:VirtualQuantity></Item><Item><rasd:InstanceID>3</rasd:InstanceID><rasd:ResourceType>17</rasd:ResourceType><rasd:HostResource>ovf:/disk/d1</rasd:HostResource></Item><Item><rasd:InstanceID>4</rasd:InstanceID><rasd:ResourceType>10</rasd:ResourceType><rasd:Connection>Guest network</rasd:Connection></Item></VirtualHardwareSection></VirtualSystem></Envelope>"#
    )
}
pub(crate) fn tar(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![];
    for (name, data) in entries {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[148..156].fill(b' ');
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        let sum = h.iter().map(|v| usize::from(*v)).sum::<usize>();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend(h);
        out.extend(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}
fn manifest(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    entries
        .iter()
        .map(|(name, data)| format!("SHA256({name})= {:x}\n", Sha256::digest(data)))
        .collect::<String>()
        .into_bytes()
}
#[test]
fn ordinary_ovf_and_ova_capture_hardware_and_bounded_disk_slice() {
    for ova in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let xml = descriptor("disk.raw", "http://raw");
        let mut entries = vec![
            ("guest.ovf", xml.as_bytes().to_vec()),
            ("disk.raw", vec![7; 4096]),
        ];
        entries.push(("guest.mf", manifest(&entries)));
        let path = d.path().join(if ova { "guest.ova" } else { "guest.ovf" });
        if ova {
            std::fs::write(&path, tar(&entries)).unwrap()
        } else {
            for (n, b) in &entries {
                std::fs::write(d.path().join(n), b).unwrap()
            }
        }
        let selected = TransferFile::ovf_source(path).unwrap();
        let summary = selected.ovf().unwrap();
        assert_eq!(summary.guests.len(), 1);
        assert_eq!(summary.guests[0].memory_bytes, 512 * 1024 * 1024);
        assert_eq!(summary.guests[0].cpus, 2);
        assert_eq!(summary.guests[0].nics[0].network, "Guest network");
        assert!(summary.manifest_verified);
        let disk = &selected.ovf.as_ref().unwrap().files["disk.raw"];
        assert_eq!(disk.size(), Some(4096));
        let mut bytes = Vec::new();
        package::Window::new(disk)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, vec![7; 4096]);
        if ova {
            assert!(disk.range.is_some());
        }
    }
}
#[test]
fn vhd_footer_is_read_relative_to_its_ova_member() {
    let d = tempfile::tempdir().unwrap();
    let xml = descriptor(
        "disk.vhd",
        "http://www.microsoft.com/technet/virtualserver/downloads/vhdspec.mspx",
    );
    let path = d.path().join("guest.ova");
    std::fs::write(
        &path,
        tar(&[
            ("guest.ovf", xml.into_bytes()),
            ("disk.vhd", crate::disk_image::tests::fixed()),
        ]),
    )
    .unwrap();
    let selected = TransferFile::ovf_source(path).unwrap();
    assert_eq!(
        selected.ovf().unwrap().disks[0].format,
        DiskImageFormat::Vhd
    );
    assert!(!selected.ovf().unwrap().manifest_verified);
}
#[test]
fn traversal_external_uris_and_parent_images_are_refused() {
    for name in [
        "../outside.raw",
        "/outside.raw",
        "https://host/disk.raw",
        "dir/../disk.raw",
        "encoded%2fdisk.raw",
        "a\\b.raw",
    ] {
        assert!(descriptor::parse(&descriptor(name, "http://raw")).is_err());
    }
    let xml = descriptor("disk.raw", "http://raw").replace(
        "ovf:diskId=\"d1\"",
        "ovf:diskId=\"d1\" ovf:parentRef=\"external\"",
    );
    assert!(descriptor::parse(&xml).is_err());
}
#[test]
fn malformed_metadata_missing_files_or_changed_manifest_refuse_selection() {
    let d = tempfile::tempdir().unwrap();
    let xml = descriptor("disk.raw", "http://raw");
    let path = d.path().join("guest.ovf");
    std::fs::write(&path, &xml).unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
    std::fs::write(d.path().join("disk.raw"), vec![7; 4096]).unwrap();
    std::fs::write(d.path().join("guest.mf"), "SHA256(guest.ovf)= 0000").unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
    std::fs::remove_file(d.path().join("guest.mf")).unwrap();
    std::fs::write(
        &path,
        xml.replace(
            "<rasd:ResourceType>17</rasd:ResourceType>",
            "<rasd:ResourceType>99</rasd:ResourceType>",
        ),
    )
    .unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
    std::fs::write(
        &path,
        "<!DOCTYPE x [<!ENTITY e SYSTEM 'file:///etc/passwd'>]><x>&e;</x>",
    )
    .unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
}
#[test]
fn ova_duplicate_members_symlinks_and_trailing_content_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    let xml = descriptor("disk.raw", "http://raw");
    let entries = vec![("guest.ovf", xml.into_bytes()), ("disk.raw", vec![7; 4096])];
    let path = d.path().join("guest.ova");
    let mut duplicate = entries.clone();
    duplicate.push(("disk.raw", vec![8; 4096]));
    std::fs::write(&path, tar(&duplicate)).unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
    let mut symlink = tar(&entries);
    symlink[156] = b'2';
    std::fs::write(&path, symlink).unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
    let mut trailing = tar(&entries);
    trailing.extend(b"untrusted");
    std::fs::write(&path, trailing).unwrap();
    assert!(TransferFile::ovf_source(path).is_err());
}
#[test]
fn package_directory_cannot_escape_through_a_symlink() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("disk.raw"), vec![0; 4096]).unwrap();
    symlink(outside.path(), d.path().join("linked")).unwrap();
    let path = d.path().join("guest.ovf");
    std::fs::write(&path, descriptor("linked/disk.raw", "http://raw")).unwrap();
    assert!(TransferFile::ovf_source(path).is_err());
}
#[test]
fn bounds_duplicate_ids_and_unmapped_networks_are_refused() {
    let xml = descriptor("disk.raw", "http://raw");
    for replacement in [
        xml.replace(
            "<rasd:VirtualQuantity>2</rasd:VirtualQuantity>",
            "<rasd:VirtualQuantity>9999999</rasd:VirtualQuantity>",
        ),
        xml.replace("byte * 2^20", "byte * 2^999"),
        xml.replace(
            "<rasd:InstanceID>4</rasd:InstanceID>",
            "<rasd:InstanceID>1</rasd:InstanceID>",
        ),
        xml.replace(
            "<rasd:Connection>Guest network</rasd:Connection>",
            "<rasd:Connection>Missing</rasd:Connection>",
        ),
    ] {
        assert!(descriptor::parse(&replacement).is_err());
    }
}
#[test]
fn cancelled_manifest_inspection_stops_before_import_plan() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("guest.ovf");
    std::fs::write(&path, descriptor("disk.raw", "http://raw")).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(true);
    assert!(matches!(
        TransferFile::ovf_source_with_cancel(path, &cancel),
        Err(Error::Closed)
    ));
}

#[test]
fn required_collection_extensions_resource_constraints_and_ambiguous_slots_are_refused() {
    let base = descriptor("disk.raw", "http://raw");
    for xml in [
        base.replace(
            "<VirtualSystem ovf:id",
            "<VirtualSystemCollection ovf:id=\"collection\"><StartupSection/><VirtualSystem ovf:id",
        )
        .replace(
            "</VirtualSystem></Envelope>",
            "</VirtualSystem></VirtualSystemCollection></Envelope>",
        ),
        base.replace(
            "<rasd:VirtualQuantity>2",
            "<rasd:Reservation>1</rasd:Reservation><rasd:VirtualQuantity>2",
        ),
        base.replace("<DiskSection>", "<DiskSection><Unknown/>"),
        base.replace("<NetworkSection>", "<NetworkSection><Unknown/>"),
        base.replace(
            "<rasd:ResourceType>17",
            "<rasd:AutomaticAllocation>false</rasd:AutomaticAllocation><rasd:ResourceType>17",
        ),
    ] {
        assert!(descriptor::parse(&xml).is_err());
    }
}

#[test]
fn positive_base256_tar_size_is_bounded_and_negative_sizes_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("guest.ova");
    let mut bytes = tar(&[
        (
            "guest.ovf",
            descriptor("disk.raw", "http://raw").into_bytes(),
        ),
        ("disk.raw", vec![7; 4096]),
    ]);
    let size = u64::from_str_radix(std::str::from_utf8(&bytes[124..135]).unwrap(), 8).unwrap();
    bytes[124..136].fill(0);
    bytes[124] = 0x80;
    bytes[128..136].copy_from_slice(&size.to_be_bytes());
    bytes[148..156].fill(b' ');
    let checksum = bytes[..512].iter().map(|b| u64::from(*b)).sum::<u64>();
    bytes[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    std::fs::write(&path, &bytes).unwrap();
    assert!(TransferFile::ovf_source(&path).is_ok());
    bytes[124] = 0xff;
    bytes[148..156].fill(b' ');
    let checksum = bytes[..512].iter().map(|b| u64::from(*b)).sum::<u64>();
    bytes[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    std::fs::write(&path, bytes).unwrap();
    assert!(TransferFile::ovf_source(&path).is_err());
}

#[test]
fn disk_parent_graph_and_controller_models_are_not_silently_flattened() {
    let base = descriptor("disk.raw", "http://raw");
    let disk_parent = base.replace(
        "<rasd:ResourceType>17</rasd:ResourceType>",
        "<rasd:ResourceType>17</rasd:ResourceType><rasd:Parent>controller</rasd:Parent>",
    );
    assert!(descriptor::parse(&disk_parent).is_err());
    for (kind, subtype, accepted) in [
        ("5", "", true),
        (
            "5",
            "<rasd:ResourceSubType>unknown-model</rasd:ResourceSubType>",
            false,
        ),
        (
            "6",
            "<rasd:ResourceSubType>lsilogic</rasd:ResourceSubType>",
            false,
        ),
        ("20", "", false),
    ] {
        let controller = format!(
            "<Item><rasd:InstanceID>controller</rasd:InstanceID><rasd:ResourceType>{kind}</rasd:ResourceType>{subtype}</Item>"
        );
        let xml = disk_parent.replace(
            "</VirtualHardwareSection>",
            &format!("{controller}</VirtualHardwareSection>"),
        );
        assert_eq!(descriptor::parse(&xml).is_ok(), accepted);
    }
    assert!(
        descriptor::parse(&disk_parent.replace(
            "<rasd:Parent>controller</rasd:Parent>",
            "<rasd:Parent>1</rasd:Parent>"
        ))
        .is_err()
    );
}
