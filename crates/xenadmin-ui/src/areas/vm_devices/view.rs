//! The surfaces of the area and the pages it gives a virtual disk's
//! Properties dialog.
//!
//! Keys. Kept from Rune: `apply-device-change`, `review-device-change`;
//! `lifecycle-name`, `lifecycle-size`, `lifecycle-slot`, `lifecycle-mac`,
//! `lifecycle-read-only`, the options `lifecycle-sr-<id>`,
//! `lifecycle-disk-<id>`, `lifecycle-network-<id>`,
//! `cancel-device-lifecycle`, `back-device-lifecycle`,
//! `apply-device-lifecycle`, `review-device-lifecycle`; `property-name`,
//! `property-description`, `property-value`, `property-enabled` (a checkbox
//! now; it was a toggle button), the options `property-choice-<n>`,
//! `property-cancel`, `property-back`, `property-apply`, `property-review`;
//! `disk-size`, `disk-copy-name`, the options `disk-destination-<id>`,
//! `disk-delete-confirmation`, `disks-close`, `disk-history`,
//! `disk-review-back`, `disk-review`, `disk-confirm`.
//!
//! New, for what Rune left unkeyed: `cancel-device-change`,
//! `device-change-review`, `device-change-image` (the ISO choice),
//! `device-change-empty`, `device-change-stale`, `device-change-error`;
//! `lifecycle-sr`, `lifecycle-disk`, `lifecycle-network` (the three
//! choices), `device-lifecycle-review`, `device-lifecycle-stale`,
//! `device-lifecycle-preview`, `device-lifecycle-error`; `property-choice`
//! (the chooser), `property-review-lines`, `property-stale`,
//! `property-preview`, `property-error`, `property-close` (a page's Close);
//! `disk-destination` (the choice), `disk-review-lines`, `disk-stale`,
//! `disk-error`, `disk-operation-<id>`, `disk-ownership`;
//! `disk-resize-open`, `disk-copy-open`, `disk-properties-close` (the page
//! Size and Location).

use xenadmin_core::{
    DeviceProperty as Change, DiskOperation, Inventory, NetworkId,
    PermissionAssessment, PoolId, ResourceAction, ResourceTarget, SrId, VdiId,
};

use super::commands::{COPY, GROW};
use super::disks::{self, Mode, Panel};
use super::{actions, lifecycle, properties};
use crate::prelude::*;

/// The demo reason of a confirm button that Rune disabled without a word.
const DEMO: &str = "Demo mode cannot change infrastructure.";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.vm_devices;
    let mut out = Vec::new();
    if let Some(panel) = &state.disks {
        out.push(disk_dialog(m, panel));
    }
    if let Some(form) = &state.form {
        out.push(change_dialog(m, form));
    }
    if let Some(form) = &state.lifecycle {
        out.push(lifecycle_dialog(m, form));
    }
    if let Some(form) = state.properties.as_ref().filter(|form| !form.page) {
        out.push(properties_dialog(m, form));
    }
    out
}

fn identity(name: &str, vm: impl ToString) -> Block {
    properties([("VM", name.to_owned()), ("VM UUID", vm.to_string())])
}

fn noted(surface: Surface, permission: Option<&PermissionAssessment>) -> Surface {
    match permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

// -- device change: connect, disconnect, eject, insert ISO ------------------

fn change_dialog(m: &Model, form: &actions::Form) -> Surface {
    let inventory = actions::inventory(m, form);
    let current = actions::target_current(m, form);
    let stale = (!current).then_some(actions::STALE);
    let unavailable = form
        .review
        .then(|| actions::unavailable(m, form))
        .flatten();
    let permission = form
        .operation
        .as_ref()
        .filter(|_| form.review)
        .map(|op| permissions::assess_for(m, form.pool, &op.permission_requirements()));
    let content = match (&inventory, form.review, form.media_drive) {
        (Some(i), true, _) => blocks![form.operation.as_ref().map(|op| {
            review("device-change-review", [actions::summary(i, op)])
                .warnings(unavailable.clone())
                .stale(stale)
        })],
        (Some(i), false, Some(vbd)) => {
            let choices = actions::media_choices(i, form.vm, vbd);
            let selected = match &form.operation {
                Some(xenadmin_core::DeviceOperation::InsertCd { image, .. }) => {
                    Some(image.to_string())
                }
                _ => None,
            };
            blocks![
                properties([("CD/DVD attachment", vbd.to_string())]),
                if choices.is_empty() {
                    let reason = super::tabs::unavailable(i, "VDI", "ISO image records")
                        .or_else(|| super::tabs::unavailable(i, "SR", "Storage repositories"))
                        .unwrap_or_else(|| {
                            "No eligible ISO images are available in the visible ISO storage repositories."
                                .into()
                        });
                    blocks![caption(reason).key("device-change-empty")]
                } else {
                    blocks![
                        choice(
                            "device-change-image",
                            choices
                                .into_iter()
                                .map(|(image, label)| option(image.to_string(), label)),
                            selected.as_deref(),
                            |m, id| {
                                if let Ok(image) = VdiId::parse(id) {
                                    actions::choose_image(m, image);
                                }
                            },
                        )
                        .label("ISO image")
                        .dropdown()
                    ]
                },
                stale.map(|reason| caption(reason)
                    .tone(Tone::Warning)
                    .key("device-change-stale")),
            ]
        }
        _ => blocks![caption(actions::STALE).key("device-change-stale")],
    };
    let error = form.error.clone().or_else(|| m.error.clone());
    let cancel = button("cancel-device-change", "Cancel", actions::close);
    let footer = if form.review {
        vec![
            cancel,
            button("apply-device-change", "Apply change", actions::apply)
                .primary()
                .disabled_opt(stale)
                .disabled_opt(unavailable)
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_if(!actions::ready(m, form.pool), actions::RECONNECT),
        ]
    } else {
        vec![
            cancel,
            button("review-device-change", "Review change", actions::review)
                .primary()
                .disabled_opt(stale)
                .disabled_if(!actions::ready(m, form.pool), actions::RECONNECT),
        ]
    };
    let surface = Surface::dialog(
        "device-change",
        if form.review {
            "Review device change"
        } else {
            "Insert ISO media"
        },
    )
    .experimental(true)
    .body(blocks![
        identity(&form.vm_name, form.vm),
        content,
        error.map(|error| alert(error).key("device-change-error")),
    ])
    .footer(footer)
    .on_close(actions::close);
    noted(surface, permission.as_ref())
}

// -- device lifecycle ---------------------------------------------------------

fn edit_lifecycle(m: &mut Model, change: impl FnOnce(&mut lifecycle::Form)) {
    if let Some(form) = &mut m.areas.vm_devices.lifecycle {
        change(form);
        form.error = None;
    }
}

fn pick<T: ToString>(
    key: &'static str,
    label: &str,
    items: Vec<(T, String)>,
    selected: Option<T>,
    on_pick: fn(&mut lifecycle::Form, &str),
) -> Vec<Block> {
    if items.is_empty() {
        return blocks![heading(label), caption(lifecycle::NO_CHOICES)];
    }
    let selected = selected.map(|id| id.to_string());
    blocks![
        choice(
            key,
            items.into_iter().map(|(id, label)| {
                let id = id.to_string();
                option(id.clone(), label).key(format!("{key}-{id}"))
            }),
            selected.as_deref(),
            move |m, id| {
                let id = id.to_owned();
                edit_lifecycle(m, |form| on_pick(form, &id));
            },
        )
        .label(label)
    ]
}

fn lifecycle_editor(form: &lifecycle::Form, inventory: &Inventory) -> Vec<Block> {
    use lifecycle::Kind;
    let mut body = Vec::new();
    if matches!(form.kind, Kind::CreateDisk) {
        body.extend(blocks![
            input("lifecycle-name", "Disk name", &form.name, |m, text| {
                edit_lifecycle(m, |form| form.name = text.to_owned())
            }),
            input(
                "lifecycle-size",
                "Capacity (bytes, KiB, MiB or GiB)",
                &form.size,
                |m, text| edit_lifecycle(m, |form| form.size = text.to_owned()),
            )
            .placeholder("16 GiB"),
            pick(
                "lifecycle-sr",
                "Storage repository",
                lifecycle::repositories(form, inventory),
                form.sr,
                |form, id| form.sr = SrId::parse(id).ok(),
            ),
        ]);
    }
    if matches!(form.kind, Kind::AttachDisk) {
        body.extend(blocks![
            checkbox(
                "lifecycle-read-only",
                "Attach read-only",
                form.read_only,
                |m| edit_lifecycle(m, |form| form.read_only = !form.read_only),
            ),
            pick(
                "lifecycle-disk",
                "Unattached disks",
                lifecycle::attachable(form, inventory),
                form.disk,
                |form, id| form.disk = VdiId::parse(id).ok(),
            ),
            caption("Only managed, non-shared disks with no consumers can be attached here. This preserves existing disk contents."),
        ]);
    }
    if matches!(form.kind, Kind::AddNic | Kind::EditNic { .. }) {
        body.extend(blocks![
            pick(
                "lifecycle-network",
                "Network",
                lifecycle::networks(inventory),
                form.network,
                |form, id| form.network = NetworkId::parse(id).ok(),
            ),
            input(
                "lifecycle-mac",
                if matches!(form.kind, Kind::AddNic) {
                    "MAC address (blank generates one)"
                } else {
                    "MAC address"
                },
                &form.mac,
                |m, text| edit_lifecycle(m, |form| form.mac = text.to_owned()),
            ),
        ]);
    }
    if matches!(
        form.kind,
        Kind::CreateDisk | Kind::AttachDisk | Kind::AddCd | Kind::AddNic
    ) {
        body.extend(blocks![
            input("lifecycle-slot", "Device slot", &form.device, |m, text| {
                edit_lifecycle(m, |form| form.device = text.to_owned())
            })
            .help("A currently unused slot is suggested. The host validates the slot before changing devices.")
        ]);
    }
    body
}

fn lifecycle_dialog(m: &Model, form: &lifecycle::Form) -> Surface {
    let inventory = lifecycle::inventory(m, form);
    let synthetic = m.is_synthetic_pool(form.pool);
    let stale = (!lifecycle::target_current(m, form)).then_some(lifecycle::STALE);
    let unavailable = lifecycle::unavailable(m, form);
    let permission = form.review.as_ref().map(|review| {
        permissions::assess_for(m, form.pool, &review.operation.permission_requirements())
    });
    let content = match (&form.review, &inventory) {
        (Some(reviewed), _) => blocks![
            review(
                "device-lifecycle-review",
                reviewed.lines.iter().flat_map(|line| line.split('\n')),
            )
            .warnings(unavailable.clone())
            .stale(stale)
        ],
        (None, Some(inventory)) => blocks![
            lifecycle_editor(form, inventory),
            stale.map(|reason| caption(reason)
                .tone(Tone::Warning)
                .key("device-lifecycle-stale")),
        ],
        (None, None) => blocks![caption(lifecycle::STALE).key("device-lifecycle-stale")],
    };
    let error = form.error.clone().or_else(|| m.error.clone());
    let cancel = button("cancel-device-lifecycle", "Cancel", lifecycle::close);
    let footer = match &form.review {
        Some(reviewed) => {
            let label = if reviewed.destructive {
                "Confirm change"
            } else {
                "Apply change"
            };
            let apply = button("apply-device-lifecycle", label, lifecycle::apply)
                .disabled_opt(stale)
                .disabled_opt(unavailable)
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_if(synthetic, lifecycle::PREVIEW)
                .disabled_if(!m.can_mutate_pool(form.pool), actions::RECONNECT);
            let apply = if reviewed.destructive {
                apply.danger()
            } else {
                apply.primary()
            };
            if matches!(form.kind, lifecycle::Kind::Fixed(_)) {
                vec![cancel, apply]
            } else {
                vec![
                    cancel,
                    button("back-device-lifecycle", "Back", lifecycle::back),
                    apply,
                ]
            }
        }
        None => vec![
            cancel,
            button("review-device-lifecycle", "Review change", lifecycle::review)
                .primary()
                .disabled_opt(stale)
                .disabled_if(
                    !synthetic && !m.can_mutate_pool(form.pool),
                    actions::RECONNECT,
                ),
        ],
    };
    let title = if form.review.is_some() {
        "Review device change"
    } else {
        form.kind.title()
    };
    let surface = Surface::dialog("device-lifecycle", title)
        .experimental(true)
        .body(blocks![
            identity(&form.vm_name, form.vm),
            content,
            synthetic.then(|| notice(NoticeKind::Preview, lifecycle::PREVIEW)
                .key("device-lifecycle-preview")),
            error.map(|error| alert(error).key("device-lifecycle-error")),
        ])
        .footer(footer)
        .on_close(lifecycle::close);
    noted(surface, permission.as_ref())
}

// -- device properties ----------------------------------------------------------

fn edit_properties(m: &mut Model, change: impl FnOnce(&mut properties::Form)) {
    if let Some(form) = &mut m.areas.vm_devices.properties {
        change(form);
    }
}

fn value_input(f: &properties::Form, label: &str) -> Field {
    input("property-value", label, &f.value, |m, text| {
        edit_properties(m, |f| f.value = text.to_owned())
    })
    .on_submit(properties::review)
}

fn property_permission(m: &Model, f: &properties::Form) -> Option<PermissionAssessment> {
    f.review.as_ref().map(|change| {
        permissions::assess_for(
            m,
            f.pool,
            &properties::operation(change).permission_requirements(),
        )
    })
}

/// The chooser, the editor of the chosen change, or its review.
fn property_body(m: &Model, f: &properties::Form) -> Vec<Block> {
    let inventory = properties::inventory(m, f);
    let stale = (!properties::current(m, f)).then_some(properties::STALE);
    let content = if let Some(change) = &f.review {
        blocks![
            heading(change.label()),
            review(
                "property-review-lines",
                properties::summary(change)
                    .split('\n')
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
            )
            .stale(stale),
        ]
    } else if let Some(choice) = &f.choice {
        let editor = match choice {
            Change::DiskMetadata { .. } => blocks![
                input("property-name", "Disk name", &f.name, |m, text| {
                    edit_properties(m, |f| f.name = text.to_owned())
                }),
                input(
                    "property-description",
                    "Description",
                    &f.description,
                    |m, text| edit_properties(m, |f| f.description = text.to_owned()),
                ),
            ],
            Change::DiskPosition { previous, .. } => blocks![
                properties([("Current position", previous.clone())]),
                value_input(f, "New unused device position (0–255)"),
            ],
            Change::DiskPriority { .. } | Change::NicRate { .. } => blocks![
                checkbox("property-enabled", "Policy enabled", f.enabled, |m| {
                    edit_properties(m, |f| f.enabled = !f.enabled)
                }),
                f.enabled.then(|| value_input(
                    f,
                    if matches!(choice, Change::DiskPriority { .. }) {
                        "I/O priority: 0 (highest) to 7 (lowest)"
                    } else {
                        "Rate limit (kilobytes per second, KB/s)"
                    },
                )),
            ],
            Change::DisableCbt { vdi, .. } => blocks![
                caption(properties::summary(choice)),
                properties([("Disk UUID to confirm", vdi.to_string())]),
                value_input(f, "Type the disk UUID"),
            ],
            _ => blocks![caption(properties::summary(choice))],
        };
        blocks![heading(choice.label()), editor]
    } else {
        let options: Vec<OptionItem> = f
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let item = option(index.to_string(), choice.label())
                    .key(format!("property-choice-{index}"));
                match inventory
                    .as_ref()
                    .map(|i| properties::operation(choice).availability(i, f.vm))
                {
                    Some(a) if a.is_available() => item,
                    Some(a) => item.disabled_if(true, a.explanation()),
                    None => item.disabled_if(true, properties::STALE),
                }
            })
            .collect();
        blocks![
            (!options.is_empty()).then(|| choice("property-choice", options, None, |m, id| {
                let picked = id.parse::<usize>().ok().and_then(|index| {
                    m.areas
                        .vm_devices
                        .properties
                        .as_ref()
                        .and_then(|f| f.choices.get(index).cloned())
                });
                if let Some(picked) = picked {
                    properties::choose(m, picked);
                }
            })
            .label("Property to change")),
            f.notes.iter().map(caption).collect::<Vec<_>>(),
        ]
    };
    let error = f.error.clone().or_else(|| m.error.clone());
    blocks![
        properties([
            ("VM", f.vm_name.clone()),
            ("VM UUID", f.vm.to_string()),
            ("Device", f.device_name.clone()),
        ]),
        content,
        stale
            .filter(|_| f.review.is_none())
            .map(|reason| caption(reason).tone(Tone::Warning).key("property-stale")),
        m.is_synthetic_pool(f.pool)
            .then(|| notice(NoticeKind::Preview, properties::PREVIEW).key("property-preview")),
        error.map(|error| alert(error).key("property-error")),
    ]
}

/// The buttons after Cancel or Close: Back, then Review or Apply.
fn property_steps(
    m: &Model,
    f: &properties::Form,
    permission: Option<&PermissionAssessment>,
) -> Vec<Button> {
    let synthetic = m.is_synthetic_pool(f.pool);
    let writable = m.can_mutate_pool(f.pool);
    let stale = (!properties::current(m, f)).then_some(properties::STALE);
    let mut buttons = Vec::new();
    if f.choice.is_none() {
        return buttons;
    }
    if !f.page || f.review.is_some() {
        buttons.push(button("property-back", "Back", properties::back));
    }
    match &f.review {
        Some(change) => {
            let apply = button("property-apply", "Apply change", properties::apply)
                .disabled_opt(stale)
                .disabled_if(
                    !properties::allowed(m, f, change),
                    "This change is no longer available.",
                )
                .disabled_opt(permission.and_then(permissions::denial))
                .disabled_if(synthetic, properties::PREVIEW)
                .disabled_if(!writable, actions::RECONNECT);
            buttons.push(if matches!(change, Change::DisableCbt { .. }) {
                apply.danger()
            } else {
                apply.primary()
            });
        }
        None => buttons.push(
            button("property-review", "Review change", properties::review)
                .primary()
                .disabled_opt(stale)
                .disabled_if(!synthetic && !writable, actions::RECONNECT),
        ),
    }
    buttons
}

fn properties_dialog(m: &Model, f: &properties::Form) -> Surface {
    let permission = property_permission(m, f);
    let mut footer = vec![button("property-cancel", "Cancel", properties::close)];
    footer.extend(property_steps(m, f, permission.as_ref()));
    let surface = Surface::dialog(
        "device-properties",
        if f.review.is_some() {
            "Review device properties"
        } else {
            "Device properties"
        },
    )
    .experimental(true)
    .body(property_body(m, f))
    .footer(footer)
    .on_close(properties::close);
    noted(surface, permission.as_ref())
}

// -- the pages of a virtual disk's Properties dialog ----------------------------

const GENERAL: &str = "general";
const SIZE_LOCATION: &str = "size-location";

/// The disk of a Properties target, with the inventory that has it.
fn disk_target(m: &Model, target: &Target) -> Option<(PoolId, VdiId, std::sync::Arc<Inventory>)> {
    let object = target.single_of(ObjectKind::VirtualDisk)?;
    let pool = object.pool?;
    let disk = VdiId::from(object.uuid()?);
    let inventory = m.inventory_for(pool)?;
    inventory
        .details
        .vdis
        .contains_key(&disk)
        .then_some((pool, disk, inventory))
}

pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    let Some((_, disk, inventory)) = disk_target(m, target) else {
        return Vec::new();
    };
    let mut pages = Vec::new();
    if properties::metadata_route(&inventory, disk).is_some() {
        pages.push(PropertyPage::planned(PropertiesKind::Disk, GENERAL).experimental(true));
    }
    pages.push(PropertyPage::planned(PropertiesKind::Disk, SIZE_LOCATION).experimental(true));
    pages
}

pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(GENERAL) => {
            if let Some((pool, disk, _)) = disk_target(m, target) {
                properties::open_page(m, pool, disk);
            }
        }
        PageEvent::Left(GENERAL)
            if m.areas
                .vm_devices
                .properties
                .as_ref()
                .is_some_and(|form| form.page) =>
        {
            m.areas.vm_devices.properties = None;
        }
        _ => {}
    }
}

pub fn page(m: &Model, target: &Target, id: &str) -> PageView {
    match id {
        GENERAL => {
            let Some(f) = m.areas.vm_devices.properties.as_ref().filter(|f| f.page) else {
                return PageView::default();
            };
            let permission = property_permission(m, f);
            let mut footer = vec![button("property-close", "Close", |m| {
                m.close_properties();
            })];
            footer.extend(property_steps(m, f, permission.as_ref()));
            PageView {
                body: property_body(m, f),
                footer,
                footer_note: permission.as_ref().map(permissions::footer_note),
            }
        }
        SIZE_LOCATION => size_location(m, target),
        _ => PageView::default(),
    }
}

/// A button from the page into one of this area's disk commands.
fn link(m: &Model, target: &Target, key: &str, label: &str, command: CommandId) -> Option<Button> {
    let reason = match m.eligibility(command, target) {
        Eligibility::Hidden => return None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Enabled => None,
    };
    let target = target.clone();
    Some(
        button(key, label, move |m| {
            m.invoke_from_properties(command, &target);
        })
        .disabled_opt(reason),
    )
}

fn size_location(m: &Model, target: &Target) -> PageView {
    let Some((_, disk, inventory)) = disk_target(m, target) else {
        return PageView::default();
    };
    let Some(record) = inventory.details.vdis.get(&disk) else {
        return PageView::default();
    };
    let storage = record
        .sr
        .and_then(|id| inventory.storage.iter().find(|sr| sr.uuid == id.uuid()))
        .map_or_else(
            || {
                record
                    .sr
                    .map_or_else(|| "Unknown".into(), |id| id.to_string())
            },
            |sr| format!("{} ({})", sr.name, sr.uuid),
        );
    PageView::new(blocks![
        properties([
            property("Virtual size", storage_bytes(record.virtual_size)),
            property("Physical usage", storage_bytes(record.physical_utilisation)),
            property("Storage repository", storage),
            property("Disk UUID", disk.to_string()).copyable(),
        ]),
        caption("A disk grows through a reviewed operation of its own; a copy on another repository is how its data moves."),
        actions(
            [
                link(m, target, "disk-resize-open", "Resize…", GROW),
                link(m, target, "disk-copy-open", "Copy…", COPY),
            ]
            .into_iter()
            .flatten()
        ),
    ])
    .footer([button("disk-properties-close", "Close", |m| {
        m.close_properties();
    })])
}

// -- one virtual disk: resize, copy, delete -------------------------------------

fn edit_panel(m: &mut Model, change: impl FnOnce(&mut Panel)) {
    if let Some(panel) = &mut m.areas.vm_devices.disks {
        change(panel);
    }
}

fn disk_draft(m: &Model, p: &Panel, inventory: &Inventory) -> Vec<Block> {
    let Some(disk) = inventory.details.vdis.get(&p.disk) else {
        return blocks![caption("This disk is no longer present in the current inventory.")];
    };
    let facts = properties([
        ("UUID", disk.id.to_string()),
        (
            "Storage",
            disk.sr
                .map_or_else(|| "Unknown".into(), |sr| sr.to_string()),
        ),
        (
            "Size",
            disk.virtual_size
                .map_or_else(|| "Unknown".into(), |n| format!("{n} bytes")),
        ),
    ]);
    let draft = match p.mode {
        Mode::Grow => blocks![
            input("disk-size", "New total capacity", &p.size, |m, text| {
                edit_panel(m, |p| p.size = text.to_owned())
            })
            .help("New total capacity: bytes, KiB, MiB or GiB. Every attachment must be inactive and unreserved. A running VM may retain an unplugged attachment.")
            .on_submit(disks::review)
        ],
        Mode::Copy => {
            let selected = p.destination.map(|id| id.to_string());
            blocks![
                input("disk-copy-name", "Copy name", &p.name, |m, text| {
                    edit_panel(m, |p| p.name = text.to_owned())
                }),
                choice(
                    "disk-destination",
                    inventory.storage.iter().map(|sr| {
                        let id = SrId::from(sr.uuid).to_string();
                        option(id.clone(), sr.name.clone()).key(format!("disk-destination-{id}"))
                    }),
                    selected.as_deref(),
                    |m, id| {
                        let id = SrId::parse(id).ok();
                        edit_panel(m, |p| p.destination = id);
                    },
                )
                .label("Destination storage"),
            ]
        }
        Mode::Cleanup => blocks![caption(match disks::source(m, p.pool, disk.id) {
            Some(origin) => format!(
                "Recorded creation: {}. Cleanup is available only after the disk is fully detached. Unknown originating operations must be resolved or explicitly acknowledged first.",
                origin.operation()
            ),
            None => "No terminal local creation record proves ownership. Untracked detached disks cannot be deleted here.".into(),
        })
        .key("disk-ownership")],
    };
    blocks![facts, draft]
}

fn disk_review(m: &Model, p: &Panel, r: &disks::Review, stale: Option<&'static str>) -> Vec<Block> {
    let expected = r.operation.expected();
    let mut lines = Vec::new();
    let mut confirmation = None;
    match &r.operation {
        DiskOperation::Grow { size, expected } => {
            lines.push(format!("Grow to at least {size} bytes. The storage driver may round upward. Guest partitions and filesystems must be expanded separately. Shrinking and online resize are not offered."));
            for c in &expected.consumers {
                lines.push(format!(
                    "Inactive attachment {} on VM {}",
                    c.attachment, c.vm
                ));
            }
        }
        DiskOperation::Copy {
            destination, name, ..
        } => lines.push(format!("Create a detached copy named {name} on storage {destination}. The original disk and attachments remain. Copy and naming are separate tracked steps; an interrupted copy may retain a disk even if naming fails.")),
        DiskOperation::Cleanup { source, .. } => {
            lines.push(format!("Permanently delete this detached disk and its data. Ownership comes from local operation {}. The backend rechecks every attachment, snapshot and suspend-image reference. This is not reversible.", source.operation()));
            confirmation = Some(input(
                "disk-delete-confirmation",
                "Type the disk UUID to confirm",
                &p.confirmation,
                |m, text| edit_panel(m, |p| p.confirmation = text.to_owned()),
            ));
        }
    }
    let refusal = match disks::available(m, p, r) {
        Some(Err(explanation)) => Some(explanation),
        _ => None,
    };
    blocks![
        heading(r.operation.label()),
        review("disk-review-lines", lines)
            .affected([
                ("Disk", r.name.clone()),
                ("UUID", r.disk.to_string()),
                ("Pool", p.pool.to_string()),
                ("Storage", expected.sr.to_string()),
                ("Current size", format!("{} bytes", expected.size)),
            ])
            .warnings(refusal)
            .stale(stale),
        confirmation,
    ]
}

fn disk_dialog(m: &Model, p: &Panel) -> Surface {
    let inventory = disks::inventory(m, p);
    let stale = (!disks::current(m, p)).then_some(disks::STALE);
    let permission = p.review.as_ref().map(|r| {
        permissions::assess_for(m, p.pool, &r.operation.permission_requirements())
    });
    let content = match (&p.review, &inventory) {
        (Some(r), _) => disk_review(m, p, r, stale),
        (None, Some(inventory)) => blocks![
            disk_draft(m, p, inventory),
            stale.map(|reason| caption(reason).tone(Tone::Warning).key("disk-stale")),
        ],
        (None, None) => blocks![caption(disks::STALE).key("disk-stale")],
    };
    let results: Vec<Block> = m
        .resource_rows
        .iter()
        .filter(|r| r.pool == p.pool && matches!(r.action, ResourceAction::Disk(_)))
        .take(15)
        .map(|r| {
            let created = match r.created {
                Some(ResourceTarget::Disk(id)) => Some(format!("Created disk: {id}")),
                _ => None,
            };
            Block::Outcome(
                outcome(
                    format!("disk-operation-{}", r.id),
                    super::tabs::outcome_state(&r.state),
                    format!("{} · {:?}", r.action.label(), r.state),
                )
                .line(r.detail.clone())
                .lines(created),
            )
        })
        .collect();
    let mut footer = vec![
        button("disks-close", "Close", disks::close),
        button("disk-history", "Operation history", disks::history).disabled_if(
            inventory.is_none(),
            "The pool of this disk is no longer shown.",
        ),
    ];
    match &p.review {
        Some(r) => {
            let cleanup = matches!(r.operation, DiskOperation::Cleanup { .. });
            let refusal = match disks::available(m, p, r) {
                Some(Ok(())) => None,
                Some(Err(explanation)) => Some(explanation),
                None => Some(disks::STALE.to_owned()),
            };
            let confirm = button("disk-confirm", "Confirm", disks::apply)
                .disabled_opt(stale)
                .disabled_if(m.is_synthetic_pool(p.pool), DEMO)
                .disabled_if(!m.can_mutate_pool(p.pool), actions::RECONNECT)
                .disabled_if(r.target.is_none(), actions::RECONNECT)
                .disabled_opt(refusal)
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_if(m.resource_operations.len() >= 128, disks::BUSY)
                .disabled_if(
                    cleanup && p.confirmation != r.disk.to_string(),
                    "Type the disk UUID to confirm",
                );
            footer.push(button("disk-review-back", "Back", disks::back));
            footer.push(if cleanup {
                confirm.danger()
            } else {
                confirm.primary()
            });
        }
        None => footer.push(
            button("disk-review", "Review", disks::review)
                .primary()
                .disabled_opt(stale),
        ),
    }
    let name = inventory
        .as_ref()
        .and_then(|i| i.details.vdis.get(&p.disk))
        .map_or_else(String::new, |disk| disk.name.clone());
    let surface = Surface::dialog("virtual-disk", p.mode.title())
        .subtitle(format!("{name} ({})", p.disk))
        .experimental(true)
        .body(blocks![
            content,
            p.error
                .clone()
                .map(|error| alert(error).key("disk-error")),
            results,
            caption("Unknown outcomes reserve the pool. Operation history provides rechecks, stop controls and explicit acknowledgement. Stop remaining steps does not cancel in-flight disk I/O."),
        ])
        .footer(footer)
        .on_close(disks::close);
    noted(surface, permission.as_ref())
}
