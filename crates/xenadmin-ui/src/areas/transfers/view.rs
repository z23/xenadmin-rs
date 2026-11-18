//! The description of the transfer wizard (Rune `transfers::dialog` and
//! `ovf_transfer_ui::summary`).
//!
//! Rune keys kept: `transfer-dialog` (the surface), `transfer-disk-picker`,
//! `export-disk-{vdi}`, `transfer-format-{xva|ovf|vhd|raw}`,
//! `transfer-package-{xva|ova|ovf}`, `transfer-compression-{label}`,
//! `checkpoint-disks-only`, `checkpoint-preserve-memory`,
//! `transfer-disk-name`, `transfer-sr-{sr}`, `ovf-network-{index}-{network}`,
//! `transfer-choose`, `transfer-review`, `transfer-apply`,
//! `transfer-acknowledge-{id}`, `transfer-release-{id}`. The format,
//! package, compression, memory, storage and network buttons are options of
//! a choice now; each option has the key of its Rune button.
//!
//! New keys (Rune left these unkeyed, or they did not exist): the choices
//! `transfer-format`, `transfer-package`, `transfer-compression`,
//! `checkpoint-memory`, `transfer-sr`, `ovf-network-{index}`; the buttons
//! `transfer-close`, `transfer-history`, `transfer-back`, `transfer-next`,
//! `transfer-recheck-{id}`, `transfer-stop-{id}`; the texts `transfer-pool`,
//! `transfer-stale`, `transfer-unavailable`, `transfer-error`,
//! `transfer-file`; the review `transfer-summary`; the outcome
//! `transfer-outcome`.

use xenadmin_core::{
    DiskImageFormat, Error, ExportSource, GuestFirmware, NetworkId, OperationControl, OperationId,
    OperationState, OvfPackageFormat, OvfSummary, SrId, TransferAction, TransferFile,
    XvaCompression,
};

use super::panel::{self, Panel, Step};
use crate::prelude::*;

pub const SURFACE: &str = "transfer-dialog";

const XVA_IMPORT: &str = "Import halted guests or memory checkpoints from a plain, gzip or zstd XVA archive. New guest identities are created; existing guests are not replaced. Disk-only guests remain halted; memory checkpoints remain suspended and are never resumed automatically. XAPI matches networks by name or creates networks; review NICs before starting a guest.";
const OVF_IMPORT: &str = "Import ordinary BIOS or UEFI guests from OVF or OVA packages containing raw or complete VHD disks. Map each package network explicitly. New guest, disk and NIC identities are created; MAC addresses are generated. Guests are not started. Convert VMDK disks separately; automatic appliance remapping and unsupported device extensions are not available.";
const DISK_IMPORT: &str = "Create a new unattached data disk from a complete raw or fixed/dynamic VHD image. Existing disks are not overwritten. Differencing VHD, VHDX and QCOW2 are not supported here.";
const DISK_EXPORT: &str = "All consumers must remain halted. For a stable point in time, export a disk snapshot. The private file is published only after full transfer and format verification.";
const VM_EXPORT: &str = "Save to a new filename. Existing files are never overwritten. The final output appears only after all required transfers and verification complete. Exports contain guest disk data.";
const COMPRESSION: &str = "Compressed exports require host support. The downloaded format and complete archive are verified before the new file is published.";
const WITH_MEMORY: &str = "The archive includes guest RAM, which may contain credentials and other sensitive data. Import creates a suspended guest; CPU, hypervisor and guest compatibility must be checked before a separate resume action.";
const WITHOUT_MEMORY: &str = "Only configuration and disks are exported. Import creates a halted guest; the checkpoint's memory state is omitted.";
const FULL_XVA: &str = "Full XVA export creates one independent guest archive. Snapshot ancestry is not included by this endpoint.";
const OVA: &str = "Create one OVA file containing the descriptor and VHD disks.";
const OVF: &str = "Create a new private directory containing the OVF descriptor, manifest and VHD disks. Choose a new directory name in the save dialog; existing directories are not reused.";
const PACKAGE: &str = "OVF/OVA export supports ordinary halted BIOS/UEFI guests with supported disk and NIC settings. Mounted installation media and unsupported configuration must be resolved before export. Package disks contain guest data.";
const OVF_CLOSING: &str = "Each source disk is copied into a new disk for each imported guest. A failed import may retain the new guest, disks and devices; confirmed UUIDs remain on the Events tab of the Notifications view. Compatible hosts are checked before completion.";
const EXPORT_REVIEW: &str = "This review uses the captured source and new destination. Interrupted exports retain their evidence on the Events tab of the Notifications view. Transfers reserve pool changes until the outcome is resolved.";
const IMPORT_REVIEW: &str = "This review uses the selected file identity and destination. An interrupted import may retain partial resources; their confirmed UUIDs remain on the Events tab of the Notifications view. Transfers reserve pool changes until the outcome is resolved.";
const UNKNOWN: &str = "Stopping the stream does not prove server cleanup. Recheck reads existing evidence and never sends archive bytes again.";
const RELEASE: &str = "Acknowledge only after inspecting the pool and any retained resources. This releases the local reservation while leaving the transfer outcome unverified; it does not delete resources or cancel server work.";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let mut out = Vec::new();
    if m.areas.transfers.panel.show {
        out.push(transfer(m, &m.areas.transfers.panel));
    }
    if m.areas.transfers.conversion.show {
        out.push(super::conversion::surface(m));
    }
    out
}

fn title(p: &Panel) -> &'static str {
    if p.disk_export.is_some() {
        "Export disk image"
    } else if panel::exporting(p) {
        "Export VM or template"
    } else {
        "Import image"
    }
}

fn transfer(m: &Model, p: &Panel) -> Surface {
    let all = panel::steps(p);
    let here = panel::step(p);
    let index = all.iter().position(|s| *s == here).unwrap_or(0);
    let surface = Surface::wizard(SURFACE, title(p), all.iter().map(|s| s.label()), index)
        .experimental(true)
        .on_close(panel::close);
    if let Some(id) = p.started {
        return surface.body(started(m, p, id)).footer([
            button("transfer-history", "History", panel::history).ghost(),
            button("transfer-close", "Close", panel::close),
        ]);
    }
    // The review names the pool in its own lines.
    let mut body = blocks![(here != Step::Review).then(|| pool_line(m, p)).flatten()];
    if here != Step::Review && !panel::current(m, p) {
        body.extend(blocks![
            caption(panel::STALE)
                .key("transfer-stale")
                .tone(Tone::Warning)
        ]);
    }
    body.extend(match here {
        Step::Type => kind(m, p),
        Step::Source | Step::Destination => file_step(m, p),
        Step::Storage => storage(m, p),
        Step::Networking => networking(m, p),
        Step::Format => format_step(p),
        Step::Review => summary(m, p),
    });
    if let Some((false, explanation)) = panel::availability(m, p) {
        body.extend(blocks![caption(explanation).key("transfer-unavailable")]);
    }
    if let Some(e) = &p.error {
        body.extend(blocks![alert(e.clone()).key("transfer-error")]);
    }
    let permission = (here == Step::Review)
        .then(|| panel::action(p))
        .flatten()
        .zip(p.pool_id)
        .map(|(a, pool)| permissions::assess_for(m, pool, &a.permission_requirements()));
    let mut footer = vec![
        button("transfer-history", "History", panel::history).ghost(),
        button("transfer-close", "Close", panel::close),
    ];
    if here == Step::Review || p.step > 0 {
        footer.push(button("transfer-back", "Back", panel::back));
    }
    if here == Step::Review {
        let importing = !panel::exporting(p);
        let pool = p.pool_id;
        footer.push(
            button("transfer-apply", "Begin transfer", panel::apply)
                .primary()
                .disabled_if(!panel::current(m, p), panel::STALE)
                .disabled_if(panel::synthetic(m, p), panel::DEMO)
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_if(
                    m.resource_operations.len() >= 64,
                    Error::Busy.to_string(),
                )
                .disabled_opt(panel::review_blocked(m, p))
                .disabled_if(
                    importing && !pool.is_some_and(|pool| m.can_mutate_pool(pool)),
                    panel::NOT_WRITABLE,
                ),
        );
    } else if p.step + 2 >= all.len() {
        footer.push(
            button("transfer-review", "Review transfer", panel::next)
                .primary()
                .disabled_opt(panel::blocked(m, p)),
        );
    } else {
        footer.push(
            button("transfer-next", "Next", panel::next)
                .primary()
                .disabled_opt(panel::blocked(m, p)),
        );
    }
    let surface = surface.body(body).footer(footer);
    match permission {
        Some(permission) => surface.footer_note(permissions::footer_note(&permission)),
        None => surface,
    }
}

fn pool_text(m: &Model, p: &Panel) -> Option<String> {
    let pool = p.pool_id?;
    let inventory = m.inventory_for(pool);
    let name = inventory
        .as_ref()
        .map_or("Captured pool", |i| i.pool_name.as_str());
    Some(format!("Pool: {name} · {pool}"))
}

fn pool_line(m: &Model, p: &Panel) -> Option<Text> {
    pool_text(m, p).map(|line| caption(line).key("transfer-pool"))
}

/// What is exported, as the Rune caption named it.
fn subject(p: &Panel) -> Option<String> {
    if let Some((vdi, sr, size)) = p.disk_export {
        return Some(format!(
            "Disk: {}\nUUID: {vdi}\nStorage: {sr}\nVirtual size: {size} bytes",
            p.source_name
        ));
    }
    Some(match p.source.as_ref()? {
        ExportSource::Vm {
            vm,
            expected_template,
        } => format!(
            "{}: {}\nUUID: {vm}",
            if *expected_template {
                "Template"
            } else {
                "Virtual machine"
            },
            p.source_name
        ),
        ExportSource::Snapshot { snapshot, parent } => format!(
            "Disk snapshot: {}\nSnapshot UUID: {snapshot}\nParent VM UUID: {parent}",
            p.source_name
        ),
        ExportSource::Checkpoint {
            snapshot, parent, ..
        } => format!(
            "Memory checkpoint: {}\nSnapshot UUID: {snapshot}\nParent VM UUID: {parent}",
            p.source_name
        ),
    })
}

fn kind_id(p: &Panel) -> &'static str {
    match (p.ovf_import, p.format) {
        (true, _) => "ovf",
        (false, Some(DiskImageFormat::Vhd)) => "vhd",
        (false, Some(DiskImageFormat::Raw)) => "raw",
        (false, None) => "xva",
    }
}

fn kind_label(p: &Panel) -> &'static str {
    match kind_id(p) {
        "ovf" => "OVF / OVA",
        "vhd" => "VHD",
        "raw" => "Raw",
        _ => "XVA",
    }
}

fn import_help(p: &Panel) -> &'static str {
    if p.ovf_import {
        OVF_IMPORT
    } else if p.format.is_some() {
        DISK_IMPORT
    } else {
        XVA_IMPORT
    }
}

fn kind_choice(p: &Panel) -> Choice {
    let mut options = Vec::new();
    if p.disk_export.is_none() {
        options.push(option("xva", "XVA").key("transfer-format-xva"));
        options.push(option("ovf", "OVF / OVA").key("transfer-format-ovf"));
    }
    options.push(option("vhd", "VHD").key("transfer-format-vhd"));
    options.push(option("raw", "Raw").key("transfer-format-raw"));
    choice("transfer-format", options, Some(kind_id(p)), panel::pick_kind)
        .label("Image format")
        .segmented()
}

/// The first step of an import: the Rune disk picker, then the kind of
/// image.
fn kind(m: &Model, p: &Panel) -> Vec<Block> {
    let mut body = blocks![actions([button(
        "transfer-disk-picker",
        if p.disk_picker {
            "Hide export disk choices"
        } else {
            "Choose existing disk to export"
        },
        panel::toggle_picker,
    )])];
    if p.disk_picker {
        body.push(heading("Export an existing disk"));
        if let Some(pool) = p.pool_id
            && let Some(i) = m.inventory_for(pool)
        {
            let mut disks = i.details.vdis.values().collect::<Vec<_>>();
            disks.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            for d in disks {
                if let (Some(sr), Some(size)) = (d.sr, d.virtual_size)
                    && (TransferAction::ExportDisk {
                        vdi: d.id,
                        expected_sr: sr,
                        expected_size: size,
                        format: DiskImageFormat::Raw,
                    })
                    .availability(&i)
                    .is_available()
                {
                    let id = d.id;
                    body.extend(blocks![
                        caption(format!("{} · {}", d.name, d.id)),
                        actions([button(
                            format!("export-disk-{id}"),
                            "Export disk image",
                            move |m| {
                                panel::begin_disk(m, pool, id);
                            },
                        )]),
                    ]);
                }
            }
        }
        body.push(heading("Import an image"));
    }
    body.extend(blocks![kind_choice(p), caption(import_help(p))]);
    body
}

fn file_lines(file: &TransferFile) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(d) = file.disk_image() {
        lines.push(format!(
            "{:?} · {} virtual bytes · {} file bytes · {}",
            d.format,
            d.virtual_size,
            file.size().unwrap_or(0),
            if d.sparse {
                "Dynamic image"
            } else {
                "Fixed size image"
            }
        ));
    }
    if let Some(summary) = file.ovf() {
        lines.extend(ovf_lines(summary));
    }
    if let Some(a) = file.archive() {
        lines.push(format!(
            "Archive compression: {}",
            a.compression.map_or("Unknown", |c| c.label())
        ));
        lines.push(format!(
            "{} guests · {} data disks · {} virtual bytes · {} networks · {} archive bytes",
            a.guests.len(),
            a.data_disks,
            a.virtual_bytes,
            a.networks,
            file.size().unwrap_or(0)
        ));
        if a.memory_images() > 0 {
            lines.push(format!("{} guest memory image(s) fully checked. Import will create suspended guests. Memory may contain secrets; review target CPU/hypervisor compatibility and NICs before a separate resume action.", a.memory_images()));
        }
        for g in &a.guests {
            lines.push(format!(
                "{} · {}",
                g.name,
                if g.memory.is_some() {
                    "Suspended checkpoint"
                } else if g.template {
                    "Template"
                } else {
                    "Halted VM"
                }
            ));
        }
    }
    lines
}

/// The Rune `ovf_ui::summary` without its network mapping, which is the
/// Networking step.
fn ovf_lines(summary: &OvfSummary) -> Vec<String> {
    let mut lines = vec![
        if summary.manifest_verified {
            "Package manifest verified. This checks file integrity, not publisher identity."
        } else {
            "No package manifest was supplied. File identities are captured and checked again before transfer."
        }
        .to_owned(),
    ];
    for guest in &summary.guests {
        lines.push(guest.name.clone());
        lines.push(format!(
            "{} vCPUs · {} · {} firmware · Disks: {} · NICs: {}",
            guest.cpus,
            storage_bytes(Some(guest.memory_bytes)),
            match guest.firmware {
                GuestFirmware::Bios => "BIOS",
                GuestFirmware::Uefi => "UEFI",
                GuestFirmware::SecureUefi => "UEFI with Secure Boot",
                GuestFirmware::Inherited => "Inherited",
            },
            guest.disks.len(),
            guest.nics.len()
        ));
        if !guest.description.is_empty() {
            lines.push(guest.description.clone());
        }
        for attachment in &guest.disks {
            if let Some(d) = summary.disks.iter().find(|d| d.key == attachment.disk) {
                lines.push(format!(
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
                ));
            }
        }
        for nic in &guest.nics {
            lines.push(format!(
                "NIC {} → package network {} · generated MAC address",
                nic.device, nic.network
            ));
        }
    }
    lines
}

/// The name and the facts of the chosen file, and in the demo scene the
/// synthetic package.
fn file_facts(m: &Model, p: &Panel) -> Vec<(String, Vec<String>)> {
    let mut facts = Vec::new();
    if let Some(file) = &p.file {
        facts.push((file.name().to_string(), file_lines(file)));
    }
    if panel::synthetic(m, p)
        && let Some(summary) = &p.ovf_preview
    {
        facts.push((
            "application.ova · synthetic preview".to_owned(),
            ovf_lines(summary),
        ));
    }
    facts
}

/// The Source step of an import and the Destination step of an export.
fn file_step(m: &Model, p: &Panel) -> Vec<Block> {
    let mut body = blocks![
        (!panel::exporting(p)).then(|| caption(import_help(p))),
        panel::exporting(p)
            .then(|| subject(p))
            .flatten()
            .map(caption),
        actions([button(
            "transfer-choose",
            if p.pending {
                panel::INSPECTING
            } else if panel::exporting(p) {
                "Choose destination"
            } else {
                "Choose image file"
            },
            panel::choose,
        )
        .disabled_if(p.pending, panel::INSPECTING)]),
    ];
    for (name, lines) in file_facts(m, p) {
        body.push(heading(name));
        body.extend(blocks![
            (!lines.is_empty()).then(|| caption(lines.join("\n")).key("transfer-file"))
        ]);
    }
    if p.ovf_import && (p.file.is_some() || p.ovf_preview.is_some()) {
        body.extend(blocks![caption(OVF_CLOSING)]);
    }
    body
}

/// The repositories an import can go to: those for which the core allows
/// an XVA import, as in Rune.
fn storage_options(m: &Model, p: &Panel) -> Vec<(SrId, String)> {
    let Some(i) = p.pool_id.and_then(|pool| m.inventory_for(pool)) else {
        return Vec::new();
    };
    i.storage
        .iter()
        .filter_map(|sr| {
            let id = SrId::from(sr.uuid);
            TransferAction::ImportXva { sr: id }
                .availability(&i)
                .is_available()
                .then(|| (id, format!("{} · {}", sr.name, id)))
        })
        .collect()
}

fn storage(m: &Model, p: &Panel) -> Vec<Block> {
    let chosen = p.sr.map(|id| id.to_string());
    blocks![
        p.format.is_some().then(|| input(
            "transfer-disk-name",
            "New disk name",
            &p.disk_name,
            panel::edit_name
        )),
        choice(
            "transfer-sr",
            storage_options(m, p)
                .into_iter()
                .map(|(id, label)| option(id.to_string(), label).key(format!("transfer-sr-{id}"))),
            chosen.as_deref(),
            panel::pick_sr,
        )
        .label("Destination storage"),
    ]
}

/// The pool networks, sorted by name then UUID.
fn network_options(m: &Model, p: &Panel) -> Vec<(NetworkId, String)> {
    let Some(i) = p.pool_id.and_then(|pool| m.inventory_for(pool)) else {
        return Vec::new();
    };
    let mut networks = i.networks.iter().collect::<Vec<_>>();
    networks.sort_by(|a, b| a.name.cmp(&b.name).then(a.uuid.cmp(&b.uuid)));
    networks
        .into_iter()
        .map(|network| {
            let id = NetworkId::from(network.uuid);
            (id, format!("{} · {id}", network.name))
        })
        .collect()
}

fn networking(m: &Model, p: &Panel) -> Vec<Block> {
    let options = network_options(m, p);
    let mut body = Vec::new();
    for (index, name) in panel::package_networks(p).into_iter().enumerate() {
        let chosen = p.networks.get(&name).map(|id| id.to_string());
        let package = name.clone();
        body.extend(blocks![
            choice(
                format!("ovf-network-{index}"),
                options.iter().map(|(id, label)| {
                    option(id.to_string(), label.clone()).key(format!("ovf-network-{index}-{id}"))
                }),
                chosen.as_deref(),
                move |m, id| panel::pick_network(m, &package, id),
            )
            .label(format!("Map package network: {name}")),
            (!p.networks.contains_key(&name)).then(|| caption(panel::UNMAPPED)),
        ]);
    }
    body.extend(blocks![caption(OVF_CLOSING)]);
    body
}

fn package_id(p: &Panel) -> &'static str {
    match p.package_export {
        None => "xva",
        Some(OvfPackageFormat::Ova) => "ova",
        Some(OvfPackageFormat::Ovf) => "ovf",
    }
}

fn package_label(p: &Panel) -> &'static str {
    match p.package_export {
        None => "XVA",
        Some(OvfPackageFormat::Ova) => "OVA file",
        Some(OvfPackageFormat::Ovf) => "OVF directory",
    }
}

fn memory_line(source: &ExportSource) -> Option<String> {
    match source {
        ExportSource::Checkpoint { memory, .. } => Some(format!(
            "Checkpoint memory image: {} · {} bytes · storage {}",
            memory.vdi, memory.virtual_size, memory.sr
        )),
        _ => None,
    }
}

fn output_format(compression: XvaCompression) -> String {
    format!(
        "Output format: {} · .{}",
        compression.label(),
        compression.extension()
    )
}

/// The first step of an export: the package format of a VM, template,
/// snapshot or checkpoint, or the image format of a disk.
fn format_step(p: &Panel) -> Vec<Block> {
    let Some(source) = &p.source else {
        return blocks![
            subject(p).map(caption),
            caption(DISK_EXPORT),
            kind_choice(p)
        ];
    };
    let checkpoint = matches!(source, ExportSource::Checkpoint { .. });
    let mut packages = vec![option("xva", "XVA").key("transfer-package-xva")];
    if !checkpoint {
        packages.push(option("ova", "OVA file").key("transfer-package-ova"));
        packages.push(option("ovf", "OVF directory").key("transfer-package-ovf"));
    }
    let mut body = blocks![
        subject(p).map(caption),
        caption(VM_EXPORT),
        choice(
            "transfer-package",
            packages,
            Some(package_id(p)),
            panel::pick_package
        )
        .label("Package format")
        .segmented(),
    ];
    if p.package_export.is_none() {
        body.extend(blocks![
            choice(
                "transfer-compression",
                XvaCompression::ALL.into_iter().map(|c| {
                    option(c.label(), c.label()).key(format!("transfer-compression-{}", c.label()))
                }),
                Some(p.compression.label()),
                panel::pick_compression,
            )
            .label("XVA compression")
            .segmented(),
            caption(output_format(p.compression)),
            caption(COMPRESSION),
        ]);
    }
    if let ExportSource::Checkpoint {
        preserve_memory, ..
    } = source
    {
        body.extend(blocks![
            memory_line(source).map(caption),
            choice(
                "checkpoint-memory",
                [
                    option("disks", "Disks only").key("checkpoint-disks-only"),
                    option("memory", "Include guest memory").key("checkpoint-preserve-memory"),
                ],
                Some(if *preserve_memory { "memory" } else { "disks" }),
                |m, id| panel::pick_memory(m, id == "memory"),
            )
            .label("Guest memory")
            .segmented(),
            caption(if *preserve_memory {
                WITH_MEMORY
            } else {
                WITHOUT_MEMORY
            }),
            caption(FULL_XVA),
        ]);
    }
    if let Some(format) = p.package_export {
        body.extend(blocks![
            caption(match format {
                OvfPackageFormat::Ova => OVA,
                OvfPackageFormat::Ovf => OVF,
            }),
            caption(PACKAGE),
        ]);
    }
    body
}

/// The review: what the Rune dialog showed with its choices disabled, as
/// lines, and its explanations as warnings.
fn summary(m: &Model, p: &Panel) -> Vec<Block> {
    let mut lines = Vec::new();
    let mut warnings = Vec::new();
    lines.extend(pool_text(m, p));
    if let Some(subject) = subject(p) {
        lines.extend(subject.lines().map(str::to_owned));
    }
    if let Some(source) = &p.source {
        lines.push(format!("Format: {}", package_label(p)));
        if p.package_export.is_none() {
            lines.push(output_format(p.compression));
            warnings.push(COMPRESSION);
        }
        if let ExportSource::Checkpoint {
            preserve_memory, ..
        } = source
        {
            lines.extend(memory_line(source));
            lines.push(format!(
                "Guest memory: {}",
                if *preserve_memory {
                    "Include guest memory"
                } else {
                    "Disks only"
                }
            ));
            warnings.push(if *preserve_memory {
                WITH_MEMORY
            } else {
                WITHOUT_MEMORY
            });
            warnings.push(FULL_XVA);
        }
        match p.package_export {
            Some(OvfPackageFormat::Ova) => warnings.extend([OVA, PACKAGE]),
            Some(OvfPackageFormat::Ovf) => warnings.extend([OVF, PACKAGE]),
            None => {}
        }
        warnings.push(VM_EXPORT);
    } else if p.disk_export.is_some() {
        lines.push(format!("Format: {}", kind_label(p)));
        warnings.push(DISK_EXPORT);
    } else {
        lines.push(format!("Format: {}", kind_label(p)));
        if p.format.is_some() {
            lines.push(format!("New disk name: {}", p.disk_name));
        }
        if let Some(sr) = p.sr {
            let label = storage_options(m, p)
                .into_iter()
                .find(|(id, _)| *id == sr)
                .map_or_else(|| sr.to_string(), |(_, label)| label);
            lines.push(format!("Storage: {label}"));
        }
        warnings.push(import_help(p));
    }
    for (name, facts) in file_facts(m, p) {
        lines.push(name);
        lines.extend(facts);
    }
    if p.ovf_import {
        let options = network_options(m, p);
        for name in panel::package_networks(p) {
            lines.push(format!("Map package network: {name}"));
            if let Some(chosen) = p.networks.get(&name)
                && let Some((_, label)) = options.iter().find(|(id, _)| id == chosen)
            {
                lines.push(label.clone());
            }
        }
        warnings.push(OVF_CLOSING);
    }
    warnings.push(if panel::exporting(p) {
        EXPORT_REVIEW
    } else {
        IMPORT_REVIEW
    });
    blocks![
        review("transfer-summary", lines)
            .warnings(warnings)
            .stale((!panel::current(m, p)).then_some(panel::STALE)),
    ]
}

fn outcome_state(state: &OperationState, progress: Option<f32>) -> OutcomeState {
    match state {
        OperationState::Succeeded => OutcomeState::Succeeded,
        OperationState::PartiallyCompleted => OutcomeState::Partial,
        OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
        OperationState::Cancelled => OutcomeState::Cancelled,
        OperationState::OutcomeUnknown => OutcomeState::Unknown,
        _ => OutcomeState::Running(progress),
    }
}

/// The outcome of the transfer this wizard submitted: the card the Rune
/// history showed for it, with its controls.
fn started(m: &Model, p: &Panel, id: OperationId) -> Vec<Block> {
    let Some(r) = m.resource_rows.iter().find(|r| r.id == id) else {
        return blocks![
            outcome(
                "transfer-outcome",
                OutcomeState::Running(None),
                "Transfer submitted"
            )
            .line(format!("Operation {id}")),
            p.error.clone().map(|e| alert(e).key("transfer-error")),
        ];
    };
    let allowed = panel::allowed(m, r);
    let mut lines = vec![
        format!("Operation {} · pool {}", r.id, r.pool),
        r.detail.clone(),
    ];
    let mut progress = None;
    if let Some(t) = &r.transfer {
        lines.push(format!(
            "{} bytes{} · {}",
            t.bytes,
            t.total.map(|n| format!(" of {n}")).unwrap_or_default(),
            t.phase
        ));
        progress = t
            .total
            .filter(|total| *total > 0)
            .map(|total| (t.bytes as f64 / total as f64).clamp(0.0, 1.0) as f32);
    }
    let created = &r.transfer_created;
    lines.extend(
        created
            .vms
            .iter()
            .map(|id| format!("Retained guest UUID: {id}")),
    );
    lines.extend(
        created
            .disks
            .iter()
            .map(|id| format!("Retained disk UUID: {id}")),
    );
    lines.extend(
        created
            .suspend_images
            .iter()
            .map(|id| format!("Retained guest memory image UUID: {id}")),
    );
    lines.extend(
        created
            .networks
            .iter()
            .map(|id| format!("Retained network UUID: {id}")),
    );
    if let Some(parent) = r.batch_parent {
        lines.push(format!("Controlled by batch {parent}. Recheck, stop or acknowledge the batch on the Events tab of the Notifications view."));
    }
    lines.extend(
        created
            .vbds
            .iter()
            .map(|id| format!("Retained disk attachment UUID: {id}")),
    );
    lines.extend(
        created
            .vifs
            .iter()
            .map(|id| format!("Retained NIC UUID: {id}")),
    );
    lines.extend(
        r.steps
            .iter()
            .map(|step| format!("{} · {:?}", step.label, step.state)),
    );
    let mut buttons = Vec::new();
    if r.batch_parent.is_none() && !r.state.terminal() {
        buttons.push(
            button(format!("transfer-recheck-{id}"), "Recheck", move |m| {
                panel::control(m, id, OperationControl::Recheck)
            })
            .disabled_if(!allowed, panel::NOT_WRITABLE),
        );
        buttons.push(
            button(
                format!("transfer-stop-{id}"),
                "Stop transfer",
                move |m| panel::control(m, id, OperationControl::RequestCancellation),
            )
            .disabled_if(!allowed, panel::NOT_WRITABLE)
            .disabled_if(r.cancel_requested, "Cancellation was already requested."),
        );
    }
    if r.batch_parent.is_none() && r.state == OperationState::OutcomeUnknown {
        lines.push(UNKNOWN.into());
        if p.acknowledge == Some(id) {
            lines.push(RELEASE.into());
            buttons.push(
                button(
                    format!("transfer-release-{id}"),
                    "Release reservation",
                    move |m| panel::control(m, id, OperationControl::AcknowledgeUnknown),
                )
                .danger()
                .disabled_if(!allowed, panel::NOT_WRITABLE),
            );
        } else {
            buttons.push(
                button(
                    format!("transfer-acknowledge-{id}"),
                    "Acknowledge after inspection",
                    move |m| panel::review_acknowledge(m, id),
                )
                .warning()
                .disabled_if(!allowed, panel::NOT_WRITABLE),
            );
        }
    }
    blocks![
        outcome(
            "transfer-outcome",
            outcome_state(&r.state, progress),
            format!("{} · {:?}", r.action.label(), r.state),
        )
        .lines(lines)
        .actions(buttons),
        p.error.clone().map(|e| alert(e).key("transfer-error")),
    ]
}
