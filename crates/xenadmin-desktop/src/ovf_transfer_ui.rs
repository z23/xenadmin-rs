use super::*;
pub(super) fn scene(s: &mut State, export: bool) {
    super::scene(s);
    if export {
        s.transfers.package_export = Some(OvfPackageFormat::Ova);
    } else {
        super::open(s);
        s.transfers.ovf_import = true;
        let Some(i) = &mut s.inventory else { return };
        let i = Arc::make_mut(i);
        s.transfers.sr = i.storage.first().map(|v| SrId::from(v.uuid));
        if let Some(sr) = s.transfers.sr {
            let id = xenadmin_core::PbdId::from(uuid::Uuid::from_u128(0x9991));
            i.details.pbds.insert(
                id,
                xenadmin_core::StorageAttachment {
                    id,
                    location: None,
                    requires_credentials: false,
                    multipathed: None,
                    paths: Vec::new(),
                    host: i.hosts.keys().next().copied(),
                    sr: Some(sr),
                    currently_attached: Some(true),
                },
            );
        }
        if let Some(network) = i.details.networks.keys().next() {
            s.transfers
                .networks
                .insert("Application network".into(), *network);
        }
        s.transfers.ovf_preview = Some(xenadmin_core::OvfSummary {
            guests: vec![xenadmin_core::OvfGuest {
                key: "application".into(),
                name: "Portable application".into(),
                description: "Synthetic guest for layout verification".into(),
                cpus: 4,
                memory_bytes: 4 << 30,
                firmware: xenadmin_core::GuestFirmware::Uefi,
                disks: vec![xenadmin_core::OvfAttachment {
                    disk: "system".into(),
                    device: 0,
                }],
                nics: vec![xenadmin_core::OvfNic {
                    network: "Application network".into(),
                    device: 0,
                }],
            }],
            disks: vec![xenadmin_core::OvfDisk {
                key: "system".into(),
                file: Some("system.vhd".into()),
                virtual_size: 32 << 30,
                format: DiskImageFormat::Vhd,
            }],
            networks: vec!["Application network".into()],
            manifest_verified: true,
        });
    }
    s.transfers.review = true;
}
pub(super) fn summary(s: &State, summary: &xenadmin_core::OvfSummary, k: &Kit) -> El {
    let p = &s.transfers;
    let mut content=col().gap(10.0).child(caption(if summary.manifest_verified {"Package manifest verified. This checks file integrity, not publisher identity."} else {"No package manifest was supplied. File identities are captured and checked again before transfer."},k).wrap());
    for guest in &summary.guests {
        content = content.child(heading(&guest.name, k)).child(
            caption(
                format!(
                    "{} vCPUs · {} · {} firmware · Disks: {} · NICs: {}",
                    guest.cpus,
                    storage_bytes(Some(guest.memory_bytes)),
                    match guest.firmware {
                        xenadmin_core::GuestFirmware::Bios => "BIOS",
                        xenadmin_core::GuestFirmware::Uefi => "UEFI",
                        xenadmin_core::GuestFirmware::SecureUefi => "UEFI with Secure Boot",
                        xenadmin_core::GuestFirmware::Inherited => "Inherited",
                    },
                    guest.disks.len(),
                    guest.nics.len()
                ),
                k,
            )
            .wrap(),
        );
        if !guest.description.is_empty() {
            content = content.child(caption(&guest.description, k).wrap());
        }
        for attachment in &guest.disks {
            if let Some(d) = summary.disks.iter().find(|d| d.key == attachment.disk) {
                content = content.child(
                    caption(
                        format!(
                            "Disk {} · {} · {} · {}",
                            attachment.device,
                            storage_bytes(Some(d.virtual_size)),
                            match d.format {
                                DiskImageFormat::Raw => "Raw",
                                DiskImageFormat::Vhd => "VHD",
                            },
                            if d.file.is_some() {
                                "Imported data"
                            } else {
                                "New empty disk"
                            }
                        ),
                        k,
                    )
                    .wrap(),
                );
            }
        }
        for nic in &guest.nics {
            content = content.child(
                caption(
                    format!(
                        "NIC {} → package network {} · generated MAC address",
                        nic.device, nic.network
                    ),
                    k,
                )
                .wrap(),
            );
        }
    }
    for (index, name) in summary.networks.iter().enumerate() {
        content = content.child(heading(format!("Map package network: {name}"), k));
        if let Some(i) = &s.inventory {
            let mut networks = i.networks.iter().collect::<Vec<_>>();
            networks.sort_by(|a, b| a.name.cmp(&b.name).then(a.uuid.cmp(&b.uuid)));
            for network in networks {
                let id = xenadmin_core::NetworkId::from(network.uuid);
                let name = name.clone();
                if p.review {
                    if p.networks.get(&name) == Some(&id) {
                        content = content
                            .child(caption(format!("{} · {id}", network.name), k).wrap_anywhere());
                    }
                    continue;
                }
                content = content.child(
                    Button::new(format!(
                        "{}{} · {id}",
                        if p.networks.get(&name) == Some(&id) {
                            "✓ "
                        } else {
                            ""
                        },
                        network.name
                    ))
                    .disabled(p.review)
                    .on_press(move |s: &mut State| {
                        s.transfers.networks.insert(name.clone(), id);
                    })
                    .build(k)
                    .key(format!("ovf-network-{index}-{id}")),
                );
            }
        }
        if !p.networks.contains_key(name) {
            content =
                content.child(caption("Choose the destination network before review.", k).wrap());
        }
    }
    content.child(caption("Each source disk is copied into a new disk for each imported guest. A failed import may retain the new guest, disks and devices; confirmed UUIDs remain in history. Compatible hosts are checked before completion.",k).wrap())
}
