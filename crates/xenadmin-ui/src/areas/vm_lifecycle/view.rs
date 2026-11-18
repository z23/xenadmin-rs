//! What the area shows, as descriptions.
//!
//! Rune keys and labels are kept: `vm-lifecycle-dialog`,
//! `cancel-vm-lifecycle`, `back-vm-lifecycle`, `review-vm-lifecycle`,
//! `apply-vm-lifecycle`, the inputs `vm-lifecycle-name`, `-description`,
//! `-vcpus`, `-memory`, `-disk-size`, the check boxes `-full-copy`,
//! `-as-template`, `creation-cloud-enable`, `vm-delete-disk-<id>`,
//! `vm-delete-snapshot-<id>`, `vm-delete-snapshot-disk-<id>`, the inputs
//! `creation-cloud-hostname`, `-user-data`, `-network`, the buttons
//! `creation-cloud-continue`, `-close`, and of the drive dialog
//! `cloud-init-dialog`, `-close`, `-hostname`, `-device`, `-storage`,
//! `-user-data`, `-network`, `-prepare`, `-edit`, `-apply`.
//!
//! The Rune lists of buttons are choices now. Their items keep the keys of
//! the buttons (`vm-lifecycle-sr-<id>`, `-network-<id>`, `-iso-<id>`,
//! `-no-iso`, `-bios-host-<id>`, `-bios-generic`, `-firmware-inherit`,
//! `-bios`, `-uefi`, `-secure-uefi`, `cloud-init-sr-<id>`), and the choices
//! the keys of the Rune lists (`vm-lifecycle-sr-choices`,
//! `-network-choices`, `-iso-choices`, `-bios-host-choices`,
//! `cloud-init-storage`).
//!
//! New keys, for elements Rune left unkeyed: `vm-lifecycle-review`,
//! `-stale`, `-preview`, `-error`, `-firmware` (the firmware choice),
//! `-sr-empty`, `-network-empty`, `-iso-empty`, `-bios-host-empty`,
//! `-inherited-disks`; `creation-cloud-identity`, `-outcome`, `-readiness`;
//! `cloud-init-heading`, `-stale`, `-unavailable`, `-read-only`, `-error`,
//! `-preparing`, `-review`, `-storage-empty`. The permission line of a
//! review is the footer note, `review-permission-explanation`.

use xenadmin_core::{GuestFirmware, HostId, NetworkId, OperationState, SnapshotId, SrId, VdiId};

use super::edit::{self, Editor, Kind};
use super::{Panel, cloud, creation, deletion, drive};
use crate::prelude::*;

/// The surface of the lifecycle editor and of the continuation of a cloud
/// creation: the Rune overlay key.
pub const DIALOG: &str = "vm-lifecycle-dialog";
/// The cloud-init drive dialog.
pub const DRIVE: &str = "cloud-init-dialog";
const REVIEW: &str = "vm-lifecycle-review";

/// Back to front: the lifecycle surface, then the cloud-init drive dialog
/// (Rune ranked it in front).
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let lifecycle = m.areas.vm_lifecycle.panel.as_ref().map(|panel| match panel {
        Panel::Editor(form) => editor(m, form),
        Panel::CloudCreation(p) => pending(m, p),
    });
    let drive = m.areas.vm_lifecycle.drive.as_ref().map(|p| drive_dialog(m, p));
    lifecycle.into_iter().chain(drive).collect()
}

fn with(m: &mut Model, change: impl FnOnce(&mut Editor)) {
    if let Some(form) = edit::editor_mut(m) {
        change(form);
    }
}

/// A list of things to choose one of: the Rune `choices`. Choosing clears
/// the editor's error. An empty list says so under its heading.
fn chooser(
    prefix: &str,
    label: &str,
    options: Vec<OptionItem>,
    chosen: Option<String>,
    on_pick: impl Fn(&mut Editor, &str) + 'static,
) -> Vec<Block> {
    if options.is_empty() {
        return blocks![
            heading(label),
            caption(edit::NO_CHOICES).key(format!("{prefix}-empty")),
        ];
    }
    blocks![
        choice(
            format!("{prefix}-choices"),
            options,
            chosen.as_deref(),
            move |m, id| with(m, |form| {
                on_pick(form, id);
                form.error = None;
            }),
        )
        .label(label)
    ]
}

// -- The lifecycle editor -----------------------------------------------------------

fn editor(m: &Model, form: &Editor) -> Surface {
    let reviewing = form.review.is_some();
    let title = if reviewing {
        "Review VM change"
    } else {
        match form.kind {
            Kind::Create => "Create VM from template",
            Kind::Clone => "Clone or full copy",
            Kind::Convert => "Convert to template",
            Kind::Delete if form.source.is_template => "Delete template",
            Kind::Delete => "Delete VM",
        }
    };
    let stale = (!edit::current(m, form)).then(|| edit::STALE.to_owned());
    let body = blocks![
        match &form.review {
            Some(reviewed) => blocks![
                review(REVIEW, reviewed.lines.clone())
                    .warnings(edit::unavailable(m, form))
                    .stale(stale.clone())
            ],
            None => draft(form),
        },
        // The review has the stale line of its own.
        (!reviewing)
            .then_some(stale)
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("vm-lifecycle-stale")),
        m.is_synthetic_pool(form.pool)
            .then(|| notice(NoticeKind::Preview, edit::PREVIEW).key("vm-lifecycle-preview")),
        form.error
            .clone()
            .or_else(|| m.error.clone())
            .map(|error| alert(error).key("vm-lifecycle-error")),
    ];
    let surface = if form.kind == Kind::Create {
        let current = if reviewing {
            creation::Step::LABELS.len() - 1
        } else {
            form.step.index()
        };
        // Seven step labels need the width.
        Surface::wizard(DIALOG, title, creation::Step::LABELS, current).large()
    } else {
        Surface::dialog(DIALOG, title)
    };
    let surface = surface
        .subtitle(format!(
            "Source: {}\nUUID: {}\nPool: {}",
            form.source.name, form.source.id, form.pool
        ))
        .experimental(true)
        .body(body)
        .footer(footer(m, form))
        .on_close(edit::close);
    // The permission line belongs to the review: before it there is no
    // operation to assess.
    match &form.review {
        Some(reviewed) => surface.footer_note(permissions::footer_note(&permissions::assess_for(
            m,
            form.pool,
            &reviewed.operation.permission_requirements(),
        ))),
        None => surface,
    }
}

fn footer(m: &Model, form: &Editor) -> Vec<Button> {
    let cancel = button("cancel-vm-lifecycle", "Cancel", edit::close);
    let (back, forward) = if form.review.is_some() {
        // A conversion has no draft to go back to.
        let back = (form.kind != Kind::Convert)
            .then(|| button("back-vm-lifecycle", "Back", edit::back));
        let apply = if form.kind == Kind::Delete {
            button("apply-vm-lifecycle", "Delete reviewed resources", edit::apply).danger()
        } else {
            button("apply-vm-lifecycle", "Apply change", edit::apply).primary()
        };
        (back, apply.disabled_opt(edit::blocked(m, form)))
    } else {
        let stepping = form.kind == Kind::Create;
        let back = (stepping && form.step != creation::Step::Name)
            .then(|| button("back-vm-lifecycle", "Back", creation::previous));
        let forward = if stepping && form.step != creation::Step::LAST {
            button("review-vm-lifecycle", "Next", creation::next)
        } else {
            button("review-vm-lifecycle", "Review change", edit::review)
        };
        (
            back,
            forward.primary().disabled_opt(edit::step_blocked(m, form)),
        )
    };
    std::iter::once(cancel)
        .chain(back)
        .chain(std::iter::once(forward))
        .collect()
}

fn draft(form: &Editor) -> Vec<Block> {
    match form.kind {
        Kind::Create => match form.step {
            creation::Step::Name => blocks![
                names(form, true),
                caption("The selected template is preserved. The new VM will stay halted until you choose to start it."),
            ],
            creation::Step::Media => media(form),
            creation::Step::Resources => resources(form),
            creation::Step::Storage => storage(form),
            creation::Step::Networking => networking(form),
            creation::Step::Cloud => cloud_step(form),
        },
        Kind::Clone => blocks![
            names(form, false),
            checkbox(
                "vm-lifecycle-full-copy",
                "Full copy to another storage repository",
                form.full_copy,
                |m| with(m, |form| form.full_copy = !form.full_copy),
            ),
            caption("Fast clone uses the source storage. Full copy creates independent disk copies on the selected repository."),
            checkbox(
                "vm-lifecycle-as-template",
                "Create as a template",
                form.as_template,
                |m| with(m, |form| form.as_template = !form.as_template),
            ),
            form.full_copy.then(|| storage_chooser(form)),
        ],
        // Nothing to enter: a conversion whose review was refused shows the
        // refusal and "Review change".
        Kind::Convert => Vec::new(),
        Kind::Delete => deletion_draft(form),
    }
}

fn names(form: &Editor, description: bool) -> Vec<Block> {
    blocks![
        input("vm-lifecycle-name", "Name", &form.name, |m, text| with(
            m,
            |form| form.name = text.to_owned()
        )),
        description.then(|| input(
            "vm-lifecycle-description",
            "Description",
            &form.description,
            |m, text| with(m, |form| form.description = text.to_owned()),
        )),
    ]
}

fn storage_chooser(form: &Editor) -> Vec<Block> {
    let options = edit::storage_choices(form)
        .into_iter()
        .map(|(id, label)| option(id.to_string(), label).key(format!("vm-lifecycle-sr-{id}")))
        .collect();
    chooser(
        "vm-lifecycle-sr",
        "Destination storage repository",
        options,
        form.sr.map(|id| id.to_string()),
        |form, id| {
            if let Ok(id) = SrId::parse(id) {
                form.sr = Some(id);
            }
        },
    )
}

// -- The steps of a creation --------------------------------------------------------

const KEEP_MEDIA: &str = "keep";
const GENERIC_BIOS: &str = "generic";

fn media(form: &Editor) -> Vec<Block> {
    let mut images: Vec<_> = form
        .initial
        .details
        .vdis
        .values()
        .filter(|disk| disk.kind == "iso" && disk.missing == Some(false) && disk.read_only == Some(true))
        .map(|disk| (disk.id, edit::disk_label(&form.initial, disk.id)))
        .collect();
    images.sort_by_cached_key(|(_, name)| name.to_lowercase());
    let none = images.is_empty();
    let options = std::iter::once(
        option(KEEP_MEDIA, "Keep inherited installation media").key("vm-lifecycle-no-iso"),
    )
    .chain(
        images
            .into_iter()
            .map(|(id, label)| option(id.to_string(), label).key(format!("vm-lifecycle-iso-{id}"))),
    )
    .collect();
    let chosen = form
        .iso
        .map_or_else(|| KEEP_MEDIA.to_owned(), |id| id.to_string());
    blocks![
        chooser(
            "vm-lifecycle-iso",
            "Installation media",
            options,
            Some(chosen),
            |form, id| {
                if id == KEEP_MEDIA {
                    form.iso = None;
                } else if let Ok(id) = VdiId::parse(id) {
                    form.iso = Some(id);
                }
            },
        ),
        none.then(|| blocks![
            heading("Available ISO images"),
            caption(edit::NO_CHOICES).key("vm-lifecycle-iso-empty"),
        ]),
    ]
}

fn resources(form: &Editor) -> Vec<Block> {
    let configurable = creation::configurable(form);
    let firmware = [
        (GuestFirmware::Inherited, "Inherit template firmware", "inherit"),
        (GuestFirmware::Bios, "BIOS", "bios"),
        (GuestFirmware::Uefi, "UEFI · Secure Boot off", "uefi"),
        (GuestFirmware::SecureUefi, "UEFI · Secure Boot on", "secure-uefi"),
    ];
    let chosen = firmware
        .iter()
        .find(|(value, _, _)| *value == form.firmware)
        .map(|(_, _, id)| *id);
    let options = firmware.map(|(value, label, id)| {
        let fixed = value != GuestFirmware::Inherited;
        option(id, label)
            .key(format!("vm-lifecycle-firmware-{id}"))
            .disabled_if(fixed && !configurable, NOT_CONFIGURABLE)
            .disabled_if(
                fixed && !creation::firmware_allowed(form, value),
                "Installed template disks keep their BIOS/UEFI mode.",
            )
    });
    blocks![
        input("vm-lifecycle-vcpus", "Virtual CPUs", &form.vcpus, |m, text| {
            with(m, |form| form.vcpus = text.to_owned())
        }),
        input(
            "vm-lifecycle-memory",
            "Memory (bytes, KiB, MiB or GiB)",
            &form.memory,
            |m, text| with(m, |form| form.memory = text.to_owned()),
        ),
        caption("CPU startup and maximum counts match. Memory uses a fixed allocation; the host validates template constraints."),
        choice("vm-lifecycle-firmware", options, chosen, |m, id| {
            let value = match id {
                "bios" => GuestFirmware::Bios,
                "uefi" => GuestFirmware::Uefi,
                "secure-uefi" => GuestFirmware::SecureUefi,
                _ => GuestFirmware::Inherited,
            };
            creation::choose_firmware(m, value);
        })
        .label("Firmware"),
        (!configurable).then(|| caption(NOT_CONFIGURABLE)),
        caption("Installed template disks keep their BIOS/UEFI mode. Secure Boot selects a policy; certificate readiness and guest enforcement must be checked after creation. Pool certificates are not changed."),
        creation::bios_eligible(form).then(|| bios_identity(form)),
    ]
}

const NOT_CONFIGURABLE: &str = "This template does not advertise a configurable HVM boot policy. Its firmware is inherited.";

/// The generic identity, or the identity of one host of the captured
/// inventory (which also becomes the home host).
fn bios_identity(form: &Editor) -> Group {
    let options = std::iter::once(
        option(GENERIC_BIOS, "Use generic BIOS identity").key("vm-lifecycle-bios-generic"),
    )
    .chain(form.initial.hosts.values().map(|host| {
        option(host.id.to_string(), format!("Copy from {}", host.name))
            .key(format!("vm-lifecycle-bios-host-{}", host.id))
    }))
    .collect();
    let chosen = form
        .bios_host
        .map_or_else(|| GENERIC_BIOS.to_owned(), |id| id.to_string());
    group(
        "BIOS identity and home host",
        blocks![
            chooser(
                "vm-lifecycle-bios-host",
                "Copy host BIOS identity",
                options,
                Some(chosen),
                |form, id| {
                    if id == GENERIC_BIOS {
                        form.bios_host = None;
                    } else if let Ok(id) = HostId::parse(id) {
                        form.bios_host = Some(id);
                    }
                },
            ),
            caption("Copying BIOS identity also sets the new VM's home-host preference to that exact host. The API only accepts an empty BIOS identity; this cannot be overwritten later."),
        ],
    )
}

fn storage(form: &Editor) -> Vec<Block> {
    let disks = creation::inherited_disks(form);
    let inherited = if disks.is_empty() {
        blocks![caption("No attached data disks are reported. Enter a capacity below to create an installation disk, or leave it blank for a diskless template.")]
    } else {
        disks
            .into_iter()
            .map(|disk| {
                Block::from(caption(format!(
                    "Copy {} to the selected repository.",
                    edit::disk_label(&form.initial, disk)
                )))
            })
            .collect()
    };
    blocks![
        storage_chooser(form),
        group("Template disks", inherited).key("vm-lifecycle-inherited-disks"),
        input(
            "vm-lifecycle-disk-size",
            "Additional new disk capacity (optional)",
            &form.disk_size,
            |m, text| with(m, |form| form.disk_size = text.to_owned()),
        )
        .placeholder("32 GiB"),
        caption("An entered capacity creates an additional blank disk. Existing template disks are preserved as copies."),
    ]
}

fn networking(form: &Editor) -> Vec<Block> {
    let mut networks: Vec<_> = form
        .initial
        .networks
        .iter()
        .map(|network| {
            let id = NetworkId::from(network.uuid);
            (id, edit::network_label(&form.initial, id))
        })
        .collect();
    networks.sort_by_cached_key(|(_, name)| name.to_lowercase());
    let options = networks
        .into_iter()
        .map(|(id, label)| option(id.to_string(), label).key(format!("vm-lifecycle-network-{id}")))
        .collect();
    blocks![
        chooser(
            "vm-lifecycle-network",
            "Network for all VM interfaces",
            options,
            form.network.map(|id| id.to_string()),
            |form, id| {
                if let Ok(id) = NetworkId::parse(id) {
                    form.network = Some(id);
                }
            },
        ),
        caption("All inherited NICs use this network. A NIC is added if the template has none."),
    ]
}

fn cloud_step(form: &Editor) -> Vec<Block> {
    blocks![
        checkbox(
            "creation-cloud-enable",
            "Configure cloud-init before first boot",
            form.cloud,
            creation::toggle_cloud,
        ),
        caption(cloud::ENABLE_HELP),
        form.cloud.then(|| blocks![
            caption(cloud::INTRODUCTION),
            input(
                "creation-cloud-hostname",
                "Guest hostname",
                &form.cloud_input.hostname,
                |m, text| with(m, |form| form.cloud_input.hostname = text.to_owned()),
            ),
            textarea(
                "creation-cloud-user-data",
                "User-data (up to 2 MiB)",
                &form.cloud_input.user_data,
                |m, text| with(m, |form| form.cloud_input.user_data = text.to_owned()),
            ),
            textarea(
                "creation-cloud-network",
                "Optional network-config (up to 1 MiB)",
                &form.cloud_input.network,
                |m, text| with(m, |form| form.cloud_input.network = text.to_owned()),
            ),
            caption(cloud::CONSEQUENCES),
        ]),
    ]
}

// -- Deletion -----------------------------------------------------------------------

fn deletion_draft(form: &Editor) -> Vec<Block> {
    let candidates = deletion::candidate_disks(form);
    let all = deletion::data_disks(&form.initial, &form.source);
    let mut disks = Vec::new();
    if all.is_empty() {
        disks.push(Block::from(caption(deletion::NO_DISKS)));
    }
    for id in all {
        let label = edit::disk_label(&form.initial, id);
        if candidates.contains(&id) {
            disks.push(Block::from(checkbox(
                format!("vm-delete-disk-{id}"),
                format!("Delete data: {label}"),
                form.disks.contains(&id),
                move |m| deletion::toggle_disk(m, id),
            )));
        } else {
            disks.push(Block::from(caption(format!(
                "Retain {label} — ownership or exclusive use is not established."
            ))));
        }
        for consumer in deletion::consumers(&form.initial, id) {
            disks.push(Block::from(caption(consumer)));
        }
    }
    let mut snapshots = Vec::new();
    let reported: Vec<_> = form.initial.snapshots.for_vm(form.source.id).collect();
    if reported.is_empty() {
        snapshots.push(Block::from(caption(deletion::NO_SNAPSHOTS)));
    }
    for snapshot in reported {
        let id: SnapshotId = snapshot.id;
        let eligible = deletion::snapshot_eligible(snapshot);
        snapshots.push(Block::from(
            checkbox(
                format!("vm-delete-snapshot-{id}"),
                format!("Delete snapshot: {} ({id})", snapshot.name),
                form.snapshots.contains(&id),
                move |m| deletion::toggle_snapshot(m, id),
            )
            .disabled_if(!eligible, deletion::SNAPSHOT_RETAINED),
        ));
        if !eligible {
            snapshots.push(Block::from(caption(deletion::SNAPSHOT_RETAINED)));
        }
        if form.snapshots.contains(&id) {
            for disk in form.initial.snapshot_deletion_disks(id) {
                snapshots.push(Block::from(checkbox(
                    format!("vm-delete-snapshot-disk-{disk}"),
                    format!(
                        "Delete this snapshot's owned data: {}",
                        edit::disk_label(&form.initial, disk)
                    ),
                    form.snapshot_disks.contains(&disk),
                    move |m| deletion::toggle_snapshot_disk(m, disk),
                )));
            }
        }
    }
    blocks![
        caption(deletion::INTRODUCTION),
        group("VM data disks", disks),
        group("Snapshots", snapshots),
        caption(deletion::CONSEQUENCES),
    ]
}

// -- After a creation with cloud-init inputs ----------------------------------------

fn pending(m: &Model, p: &cloud::Pending) -> Surface {
    let record = cloud::record(m, p);
    let readiness = cloud::ready(m, p);
    let progress = record.as_ref().map(|r| {
        let state = match r.state {
            OperationState::Succeeded => OutcomeState::Succeeded,
            OperationState::PartiallyCompleted => OutcomeState::Partial,
            OperationState::Failed | OperationState::Rejected => OutcomeState::Failed,
            OperationState::Cancelled => OutcomeState::Cancelled,
            OperationState::OutcomeUnknown | OperationState::AcknowledgedUnknown => {
                OutcomeState::Unknown
            }
            _ => OutcomeState::Running(None),
        };
        outcome(
            "creation-cloud-outcome",
            state,
            format!("{:?} · {}", r.state, r.detail),
        )
        .lines(r.created_vm.map(|vm| format!("Created VM: {vm}")))
        .lines(
            r.steps
                .iter()
                .map(|step| format!("{} · {:?}", step.label, step.state)),
        )
    });
    let body = blocks![
        caption(format!(
            "Creation operation: {}\nSource template: {}\nPool: {}",
            p.id, p.source, p.pool
        ))
        .key("creation-cloud-identity"),
        progress,
        text(match &readiness {
            Ok(_) => cloud::VERIFIED.to_owned(),
            Err(reason) => reason.clone(),
        })
        .key("creation-cloud-readiness"),
        caption(cloud::PRIVATE),
    ];
    Surface::dialog(DIALOG, "Create VM · configuration drive")
        .experimental(true)
        .body(body)
        .footer([
            button("creation-cloud-close", "Close and discard draft", edit::close),
            button(
                "creation-cloud-continue",
                "Review configuration drive",
                cloud::continue_to_drive,
            )
            .primary()
            .disabled_opt(cloud::blocked(m, p)),
        ])
        .on_close(edit::close)
}

// -- The cloud-init drive -----------------------------------------------------------

fn drive_dialog(m: &Model, p: &drive::Panel) -> Surface {
    let reviewing = p.media.is_some();
    let stale = (!drive::current(m, p)).then(|| drive::STALE.to_owned());
    let synthetic = m.is_synthetic_pool(p.pool);
    let status = blocks![
        heading(if reviewing {
            "Review cloud-init drive"
        } else {
            "Prepare cloud-init drive"
        }),
        // The review has the stale line of its own.
        (!reviewing)
            .then(|| stale.clone())
            .flatten()
            .map(|stale| notice(NoticeKind::Warning, stale).key("cloud-init-stale")),
        drive::unavailable(m, p)
            .map(|reason| text(reason).tone(Tone::Warning).key("cloud-init-unavailable")),
        (!synthetic && !m.can_mutate_pool(p.pool))
            .then(|| text(drive::READ_ONLY).tone(Tone::Warning).key("cloud-init-read-only")),
        p.error.clone().map(|error| alert(error).key("cloud-init-error")),
    ];
    let close = button("cloud-init-close", "Close", drive::close);
    let surface = Surface::dialog(DRIVE, "Cloud-init drive")
        .subtitle(format!("{}\nVM {} · pool {}", p.name, p.vm, p.pool))
        .experimental(true)
        .on_close(drive::close);
    if let Some(media) = &p.media {
        let inventory = m.inventory_for(p.pool);
        let sr = p
            .sr
            .and_then(|id| {
                inventory
                    .as_ref()?
                    .storage
                    .iter()
                    .find(|sr| sr.uuid == id.uuid())
                    .map(|r| format!("{} ({})", r.name, r.uuid))
            })
            .unwrap_or_else(|| "Unavailable storage".into());
        let lines = [
            format!("Hostname: {}", p.hostname),
            format!("Storage: {sr}"),
            format!("Disk slot: {}", p.device),
            "Media: 8 MiB FAT16 · CIDATA".to_owned(),
            format!("Configuration ID: {}", media.configuration_id()),
        ];
        let surface = surface
            .body(blocks![
                status,
                review("cloud-init-review", lines).stale(stale),
                caption(drive::WHAT_HAPPENS),
                caption(drive::GUEST_LIMITS),
                caption(drive::POOL_BLOCK),
            ])
            .footer([
                close,
                button("cloud-init-edit", "Discard and edit", drive::discard),
                button("cloud-init-apply", "Upload and attach drive", drive::apply)
                    .primary()
                    .disabled_opt(drive::apply_blocked(m, p)),
            ]);
        return match drive::config(p) {
            Some(config) => surface.footer_note(permissions::footer_note(
                &permissions::assess_for(m, p.pool, &config.permission_requirements()),
            )),
            None => surface,
        };
    }
    if p.pending {
        return surface
            .body(blocks![
                status,
                outcome(
                    "cloud-init-preparing",
                    OutcomeState::Running(None),
                    drive::PREPARING
                ),
            ])
            .footer([close]);
    }
    surface
        .body(blocks![status, drive_inputs(m, p)])
        .footer([
            close,
            button("cloud-init-prepare", "Prepare and review", drive::prepare)
                .primary()
                .disabled_opt(drive::prepare_blocked(m, p)),
        ])
}

fn drive_edit(m: &mut Model, change: impl FnOnce(&mut drive::Panel)) {
    if let Some(p) = m.areas.vm_lifecycle.drive.as_mut() {
        change(p);
    }
}

fn drive_inputs(m: &Model, p: &drive::Panel) -> Vec<Block> {
    // The repositories of the pool as it is now, as in Rune.
    let options: Vec<OptionItem> = m.inventory_for(p.pool).map_or_else(Vec::new, |i| {
        i.details
            .storage
            .values()
            .filter(|sr| sr.content_type == "user")
            .map(|sr| {
                let name = i
                    .storage
                    .iter()
                    .find(|s| s.uuid == sr.id.uuid())
                    .map_or("Unnamed storage", |s| s.name.as_str());
                option(sr.id.to_string(), name).key(format!("cloud-init-sr-{}", sr.id))
            })
            .collect()
    });
    let chosen = p.sr.map(|id| id.to_string());
    let storage = if options.is_empty() {
        blocks![
            heading(drive::NO_STORAGE),
            caption(edit::NO_CHOICES).key("cloud-init-storage-empty"),
        ]
    } else {
        blocks![
            choice("cloud-init-storage", options, chosen.as_deref(), |m, id| {
                if let Ok(id) = SrId::parse(id) {
                    drive_edit(m, |p| p.sr = Some(id));
                }
            })
            .label(drive::NO_STORAGE)
        ]
    };
    blocks![
        caption(drive::INTRODUCTION),
        input("cloud-init-hostname", "Guest hostname", &p.hostname, |m, text| {
            drive_edit(m, |p| p.hostname = text.to_owned())
        }),
        input(
            "cloud-init-device",
            "Unused disk slot (0–255)",
            &p.device,
            |m, text| drive_edit(m, |p| p.device = text.to_owned()),
        ),
        storage,
        textarea(
            "cloud-init-user-data",
            "User-data (up to 2 MiB)",
            &p.user_data,
            |m, text| drive_edit(m, |p| p.user_data = text.to_owned()),
        ),
        textarea(
            "cloud-init-network",
            "Optional network-config (up to 1 MiB)",
            &p.network,
            |m, text| drive_edit(m, |p| p.network = text.to_owned()),
        ),
        caption(drive::PRIVACY),
    ]
}
