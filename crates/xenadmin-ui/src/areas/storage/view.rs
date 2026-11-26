//! What the area shows, described: the Storage tab of a pool or a server,
//! and the surface of the open editor. Nothing here changes the model.
//!
//! One editor, four shapes (the Rune module had one dialog, "Storage
//! administration", for all of them):
//!
//! - [`WIZARD`]: the New Storage Repository wizard (create or introduce),
//!   steps Type, Name, Location, Review. The discovery results replace the
//!   Location step while they are shown.
//! - [`ATTACHMENT`]: the form of a new host attachment, then its review.
//! - [`CHANGE`]: a review opened by its command (forget, destroy, detach,
//!   reattach, remove an attachment, restore a connection), and the chooser
//!   of the host to reclaim blocks through. No Back: there is no draft.
//! - [`CLEANUP`]: the review of deleting retained temporary Secrets.
//!
//! Keys and labels are the Rune ones: `storage-name`, `storage-kind-<n>`
//! (now options of the choice `storage-kind`), `storage-server`,
//! `storage-local-device`, `storage-local-identity`, `storage-local-ack`,
//! `storage-port`, `storage-smb-share`, `storage-allocation-<label>` (options
//! of `storage-allocation`), `storage-auth`, `storage-username`,
//! `storage-password`, `storage-result-<index>`, `storage-confirm-device`,
//! `storage-confirm-lun`, `storage-confirm-uuid`, `storage-close`,
//! `storage-review`, `storage-confirm`, `storage-cleanup-apply`.
//!
//! New keys, for elements Rune left without one: `storage-mode`,
//! `storage-description`, `storage-iqn`, `storage-scsi`, `storage-smb-path`,
//! `storage-nfs-path`, `storage-nfs-version`, `storage-mutual`,
//! `storage-incoming-username`, `storage-incoming-password`, `storage-host`,
//! `storage-probe`, `storage-discover-<step>`, `storage-uuid`,
//! `storage-driver`, `storage-hba-selected`, `storage-local-probe`,
//! `storage-results-previous`, `storage-results-next`, `storage-results-back`,
//! `storage-next`, `storage-back`, `storage-change-review` (and its
//! `-stale` line), `storage-cleanup-review`, `storage-cleanup-back`,
//! `storage-cleanup-review-<operation>`, `storage-trim-<host>`,
//! `storage-discovering`, `storage-error`, `storage-read-only`,
//! `storage-preview`. The Rune heading `storage-heading` is the title of the
//! surface now and has no element.
//!
//! The tab keeps the keys of the information-only tab it replaces
//! (`general`): `storage-repositories`, `storage-repositories-table`,
//! `storage-attachments`, `storage-pbds`, `class-notice-pbd`.

use xenadmin_core::{
    ClassAvailability, HostId, Inventory, NfsVersion, PbdId, PermissionAssessment, PoolId,
    ResourceAction, ResourceTarget, SrId, StorageAllocation, StorageDiscoveryResult,
    StorageLocation, StorageOperation,
};

use super::editor::{self, Editor, KINDS, Mode};
use crate::prelude::*;

pub const WIZARD: &str = "storage-new";
pub const ATTACHMENT: &str = "storage-attachment";
pub const CHANGE: &str = "storage-change";
pub const CLEANUP: &str = "storage-secret-cleanup";

/// The key of the review, so `storage-change-review-stale` is its stale line.
pub const REVIEW: &str = "storage-change-review";

pub const STEPS: [&str; 4] = ["Type", "Name", "Location", "Review"];

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The reason for what Rune disabled without saying so on a connection
/// that cannot change anything (the words of `host_admin`).
pub const NOT_WRITABLE: &str = "This connection is read-only, disconnected or stale.";

const TOO_MANY: &str = "Only the first 100 disk UUIDs are shown. Review a smaller repository before removing it here.";
const RESOLVE: &str =
    "Resolve or acknowledge the uncertain source operation on the Events tab of the Notifications view before separate cleanup.";
const NO_DISCOVERY: &str = "Synthetic preview · discovery is not available.";
const DISCOVERING: &str = "Reading storage discovery…";

// -- The tab ------------------------------------------------------------------------

/// The Storage tab of a pool (its repositories) or of a server (the
/// repositories it has an attachment of). `None` for every other scope and
/// while the pool has no inventory: the next provider answers.
pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Storage {
        return None;
    }
    let object = scope.single()?;
    let pool = object.pool?;
    let inventory = m.inventory_for(pool)?;
    let selected = m.areas.storage.selected.as_deref();
    match object.kind {
        ObjectKind::Pool => Some(TabBody::Blocks(pool_grid(&inventory, pool, selected))),
        ObjectKind::Host => {
            let host = object.host_id()?;
            inventory
                .hosts
                .contains_key(&host)
                .then(|| TabBody::Blocks(host_grid(&inventory, pool, host, selected)))
        }
        _ => None,
    }
}

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}

fn grid(key: &str, columns: Vec<Column>, rows: Vec<Row>, selected: Option<&str>) -> Table {
    let selected = selected.filter(|id| rows.iter().any(|row| row.id == *id));
    table(key, columns, rows)
        .single(selected, select_row)
        .on_activate(|m, id| {
            if let Some(object) = row_object(m, id) {
                m.select(Target::one(object));
            }
        })
        .on_clear(|m| {
            m.areas.storage.selected = None;
            m.select_in_grid(Target::none());
        })
}

fn pool_grid(i: &Inventory, pool: PoolId, selected: Option<&str>) -> Vec<Block> {
    let mut repositories: Vec<_> = i.storage.iter().collect();
    repositories.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.uuid.cmp(&b.uuid)));
    let rows = repositories
        .into_iter()
        .map(|repository| {
            let details = i.details.storage.get(&SrId::from(repository.uuid));
            row(
                repository.uuid,
                [
                    repository.name.clone(),
                    repository.kind.clone(),
                    yes_no(details.and_then(|d| d.shared)).to_owned(),
                    storage_bytes(repository.used_bytes),
                    storage_bytes(repository.capacity_bytes),
                ],
            )
            .object(ObjectRef::storage(pool, repository.uuid))
        })
        .collect();
    let columns = vec![
        column("name", "Name"),
        column("type", "Type"),
        column("shared", "Shared"),
        column("used", "Used").right(),
        column("capacity", "Capacity").right(),
    ];
    blocks![
        group(
            "Storage repositories",
            blocks![
                grid("storage-repositories-table", columns, rows, selected)
                    .empty("No storage repository of this pool is in the readable inventory."),
            ]
        )
        .key("storage-repositories")
    ]
}

/// One row per attachment of the server: the repository it attaches, and
/// whether it is attached. The row is the repository (its context menu, the
/// commands of the local row); its id is the attachment, which is what
/// Detach and Reattach act on.
fn host_grid(i: &Inventory, pool: PoolId, host: HostId, selected: Option<&str>) -> Vec<Block> {
    let mut attachments: Vec<_> = i
        .details
        .pbds
        .values()
        .filter(|a| a.host == Some(host))
        .map(|a| {
            let repository = a
                .sr
                .and_then(|sr| i.storage.iter().find(|r| r.uuid == sr.uuid()));
            let name = match (repository, a.sr) {
                (Some(repository), _) => repository.name.clone(),
                (None, Some(sr)) => sr.to_string(),
                (None, None) => "Unresolved SR".into(),
            };
            (name, a, repository)
        })
        .collect();
    attachments.sort_by(|a, b| natural_cmp(&a.0, &b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let rows = attachments
        .into_iter()
        .map(|(name, a, repository)| {
            let details = a.sr.and_then(|sr| i.details.storage.get(&sr));
            let line = row(
                a.id,
                [
                    name,
                    repository.map(|r| r.kind.clone()).unwrap_or_default(),
                    yes_no(details.and_then(|d| d.shared)).to_owned(),
                    storage_bytes(repository.and_then(|r| r.used_bytes)),
                    storage_bytes(repository.and_then(|r| r.capacity_bytes)),
                    yes_no(a.currently_attached).to_owned(),
                    a.id.to_string(),
                ],
            );
            match repository {
                Some(repository) => line.object(ObjectRef::storage(pool, repository.uuid)),
                None => line,
            }
        })
        .collect();
    let columns = vec![
        column("name", "Storage repository"),
        column("type", "Type"),
        column("shared", "Shared"),
        column("used", "Used").right(),
        column("capacity", "Capacity").right(),
        column("attached", "Attached"),
        column("uuid", "Attachment UUID"),
    ];
    blocks![
        group(
            "Storage attachments",
            blocks![
                class_notice(i, "PBD"),
                grid("storage-pbds", columns, rows, selected)
                    .empty("No storage attachment of this server is in the readable inventory."),
            ]
        )
        .key("storage-attachments")
    ]
}

/// What the tab says when the inventory class behind it was not read (the
/// notice of the information-only tab, word for word).
fn class_notice(inventory: &Inventory, class: &str) -> Option<Notice> {
    let state = inventory.details.classes.get(&class.to_ascii_lowercase());
    let message = match state {
        Some(ClassAvailability::Available) => return None,
        Some(ClassAvailability::PermissionDenied) => {
            "Your account cannot read this inventory class"
        }
        Some(ClassAvailability::Unsupported) => "The host does not support this inventory class",
        Some(ClassAvailability::InvalidData) => "This inventory class could not be decoded",
        _ => "Availability of this inventory class is unknown",
    };
    Some(
        notice(NoticeKind::Warning, format!("{class}: {message}"))
            .key(format!("class-notice-{}", class.to_ascii_lowercase())),
    )
}

/// The repository a row of the grid stands for.
pub(super) fn row_object(m: &Model, id: &str) -> Option<ObjectRef> {
    let scope = m.scope().single()?;
    let pool = scope.pool?;
    match scope.kind {
        ObjectKind::Pool => Some(ObjectRef::storage(pool, uuid::Uuid::parse_str(id).ok()?)),
        ObjectKind::Host => {
            let inventory = m.inventory_for(pool)?;
            let pbd = PbdId::parse(id).ok()?;
            let sr = inventory.details.pbds.get(&pbd)?.sr?;
            Some(ObjectRef::storage(pool, sr.uuid()))
        }
        _ => None,
    }
}

/// The grid selection: the commands of the local row and the kernel's
/// context menu act on the repository of the selected row.
fn select_row(m: &mut Model, id: &str) {
    m.areas.storage.selected = Some(id.to_owned());
    let target = row_object(m, id)
        .map(Target::one)
        .unwrap_or_else(Target::none);
    m.select_in_grid(target);
}

// -- The surface --------------------------------------------------------------------

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(e) = &m.areas.storage.editor else {
        return Vec::new();
    };
    let permission = editor::permission(m, e);
    let surface = if e.cleanup_review.is_some() {
        Surface::dialog(CLEANUP, "Review temporary Secret cleanup")
    } else {
        match e.mode {
            Mode::Create | Mode::Introduce => Surface::wizard(
                WIZARD,
                "New Storage Repository",
                STEPS,
                if e.review.is_some() { 3 } else { e.step.min(2) },
            ),
            Mode::Attachment => Surface::dialog(ATTACHMENT, "Create missing host attachment"),
            Mode::Manage => Surface::dialog(
                CHANGE,
                match &e.review {
                    Some((_, _, operation)) => operation.label(),
                    None if e.trim => "Reclaim unused blocks",
                    None => "Storage change",
                },
            ),
        }
    };
    let surface = surface
        .subtitle(format!("Pool {}\nTarget {}", e.pool, e.target.uuid()))
        .experimental(true)
        .body(body(m, e))
        .footer(footer(m, e, permission.as_ref()))
        .on_close(super::close);
    vec![match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }]
}

/// A handler that edits the open editor with the new text of a field.
fn edit(apply: impl Fn(&mut Editor, &str) + 'static) -> impl Fn(&mut Model, &str) + 'static {
    move |m, text| {
        if let Some(e) = &mut m.areas.storage.editor {
            apply(e, text);
        }
    }
}

/// A handler that changes the open editor.
fn change(apply: impl Fn(&mut Editor) + 'static) -> impl Fn(&mut Model) + 'static {
    move |m| {
        if let Some(e) = &mut m.areas.storage.editor {
            apply(e);
        }
    }
}

fn body(m: &Model, e: &Editor) -> Vec<Block> {
    let state = if let Some((_, source, secrets)) = &e.cleanup_review {
        let mut lines = vec![format!("Source operation: {source}")];
        lines.extend(secrets.iter().map(|id| format!("Temporary Secret: {id}")));
        lines.push("Deletes only the selected operation's owned, unused temporary Secrets. The backend refuses cleanup when provisioning may still be active or any storage configuration still references a Secret. Attachment-owned credential copies are preserved.".into());
        blocks![review("storage-cleanup-review", lines)]
    } else if let Some((_, target, operation)) = &e.review {
        reviewing(m, e, *target, operation)
    } else if e.mode == Mode::Manage {
        trim_choices(m, e)
    } else if e.discovered.is_some() {
        results(m, e)
    } else if e.mode == Mode::Attachment {
        blocks![kind_choice(e), location_fields(m, e)]
    } else {
        match e.step {
            0 => blocks![
                choice(
                    "storage-mode",
                    [
                        option("create", "Create new"),
                        option("introduce", "Introduce existing"),
                    ],
                    Some(if e.mode == Mode::Introduce {
                        "introduce"
                    } else {
                        "create"
                    }),
                    edit(|e, id| {
                        e.mode = if id == "introduce" {
                            Mode::Introduce
                        } else {
                            Mode::Create
                        };
                    })
                )
                .segmented(),
                kind_choice(e),
            ],
            1 => blocks![
                input(
                    "storage-name",
                    "Name",
                    &e.name,
                    edit(|e, text| e.name = text.to_owned())
                ),
                input(
                    "storage-description",
                    "Description",
                    &e.description,
                    edit(|e, text| e.description = text.to_owned())
                ),
            ],
            _ => location_fields(m, e),
        }
    };
    let read_only = m.connection_for(e.pool).is_some_and(|c| c.is_read_only());
    blocks![
        state,
        retained(m, e),
        e.discovery
            .is_some()
            .then(|| caption(DISCOVERING).key("storage-discovering")),
        e.error
            .clone()
            .map(|error| alert(error).key("storage-error")),
        read_only.then(|| {
            caption("Read-only connection: discovery and inspection are available; changes are disabled.")
                .key("storage-read-only")
        }),
        m.is_synthetic_pool(e.pool).then(|| {
            notice(
                NoticeKind::Preview,
                "Synthetic preview · changes cannot be submitted.",
            )
            .key("storage-preview")
        }),
    ]
}

fn kind_choice(e: &Editor) -> Vec<Block> {
    let selected = e.kind.to_string();
    blocks![
        caption("Choose a storage backend. Credentials are session-only inputs; the host stores its attachment credentials in server Secret objects."),
        choice(
            "storage-kind",
            KINDS.map(|(kind, label)| {
                option(kind.to_string(), label).key(format!("storage-kind-{kind}"))
            }),
            Some(selected.as_str()),
            |m, id| {
                if let Ok(kind) = id.parse() {
                    editor::set_kind(m, kind);
                }
            }
        ),
    ]
}

/// Everything about where the storage is, in the Rune order: the server,
/// the fields of the backend, allocation, credentials, the host, probing
/// and discovery, the UUID of an existing repository, the driver.
fn location_fields(m: &Model, e: &Editor) -> Vec<Block> {
    let synthetic = m.is_synthetic_pool(e.pool);
    let busy = e.discovery.is_some();
    let discover = |step: u8, label: &str| {
        button(format!("storage-discover-{step}"), label, move |m| {
            editor::query(m, step)
        })
        .disabled_if(busy, DISCOVERING)
        .disabled_if(synthetic, NO_DISCOVERY)
    };
    let backend = match e.kind {
        5..=7 => local_fields(e),
        8 => blocks![
            input(
                "storage-smb-share",
                "SMB share name",
                &e.share,
                edit(|e, text| e.share = text.to_owned())
            )
            .help("Root share name only; subdirectories are not used by the SMB disk driver"),
        ],
        4 => blocks![
            caption(if e.scsi.is_empty() {
                "Discover HBA devices on the selected host, then select a LUN.".into()
            } else {
                format!("Selected LUN: {}", e.scsi)
            })
            .key("storage-hba-selected"),
            e.hba_device.as_ref().map(|(host, device)| {
                caption(format!(
                    "Discovered on host {host}\n{} · {} · serial {}",
                    bytes(device.size_bytes),
                    device.vendor,
                    device.serial
                ))
            }),
        ],
        2 => blocks![
            input(
                "storage-port",
                "iSCSI port",
                &e.port,
                edit(|e, text| e.port = text.to_owned())
            ),
            input(
                "storage-iqn",
                "Target IQN",
                &e.iqn,
                edit(|e, text| e.iqn = text.to_owned())
            ),
            input(
                "storage-scsi",
                "Exact LUN SCSI ID",
                &e.scsi,
                edit(|e, text| e.scsi = text.to_owned())
            ),
        ],
        3 => blocks![
            input(
                "storage-smb-share",
                "SMB share name",
                &e.share,
                edit(|e, text| e.share = text.to_owned())
            ),
            input(
                "storage-smb-path",
                "Path inside SMB share",
                &e.path,
                edit(|e, text| e.path = text.to_owned())
            )
            .help("Optional path inside share"),
        ],
        _ => blocks![
            input(
                "storage-nfs-path",
                "NFS export path",
                &e.path,
                edit(|e, text| e.path = text.to_owned())
            ),
            choice(
                "storage-nfs-version",
                [
                    option("automatic", "Automatic"),
                    option("3", "NFS 3"),
                    option("4", "NFS 4"),
                    option("4.1", "NFS 4.1"),
                ],
                Some(match e.version {
                    NfsVersion::Automatic => "automatic",
                    NfsVersion::V3 => "3",
                    NfsVersion::V4 => "4",
                    NfsVersion::V41 => "4.1",
                }),
                edit(|e, id| {
                    e.version = match id {
                        "3" => NfsVersion::V3,
                        "4" => NfsVersion::V4,
                        "4.1" => NfsVersion::V41,
                        _ => NfsVersion::Automatic,
                    };
                })
            )
            .label("NFS version")
            .segmented(),
        ],
    };
    let scans: Vec<Button> = match e.kind {
        2 => vec![
            discover(3, "Discover iSCSI targets"),
            discover(4, "Discover LUNs for selected IQN"),
        ],
        4 => vec![discover(5, "Discover HBA devices")],
        3 | 5..=8 => Vec::new(),
        _ => vec![discover(2, "Discover NFS exports")],
    };
    let local = editor::is_local(e);
    blocks![
        (!matches!(e.kind, 4..=7)).then(|| {
            input(
                "storage-server",
                "Server / iSCSI portal",
                &e.server,
                edit(|e, text| e.server = text.to_owned()),
            )
            .help("Server hostname or IP address")
        }),
        backend,
        allocation_choice(e),
        credential_fields(e),
        host_choice(m, e),
        actions([button(
            "storage-probe",
            if local {
                "Probe this device for EXT, LVM and XFS repositories"
            } else {
                "Probe selected location for existing repositories"
            },
            move |m| editor::query(m, if local { 6 } else { 1 })
        )
        .disabled_if(busy, DISCOVERING)
        .disabled_if(synthetic, NO_DISCOVERY)]),
        caption("Discovery asks the selected host to contact storage. The backend can temporarily mount an export or open an iSCSI session; no repository is created."),
        (!scans.is_empty()).then(|| actions(scans)),
        (e.mode == Mode::Introduce).then(|| {
            input(
                "storage-uuid",
                "Existing repository UUID",
                &e.uuid,
                edit(|e, text| e.uuid = text.to_owned()),
            )
        }),
        e.backends
            .iter()
            .find(|b| b.kind == editor::location(e).kind())
            .map(|backend| {
                caption(format!(
                    "Host storage driver: {} · Discovery: {}",
                    backend.name,
                    if backend.features.iter().any(|f| f == "SR_PROBE") {
                        "supported"
                    } else {
                        "not advertised"
                    }
                ))
                .key("storage-driver")
            }),
    ]
}

/// The fields of a local EXT or LVM repository (`storage_local_view.rs`).
/// Editing the device path, not focusing it, invalidates the probe and the
/// acknowledgement.
fn local_fields(e: &Editor) -> Vec<Block> {
    blocks![
        input(
            "storage-local-device",
            "Local /dev/disk/by-id device path",
            &e.path,
            edit(|e, text| {
                e.path = text.to_owned();
                e.local_probe = None;
                e.local_acknowledged = false;
            })
        )
        .placeholder("/dev/disk/by-id/ata-...")
        .help("Single persistent device path on the selected host"),
        caption("Enter one /dev/disk/by-id path verified by you. Device lists, mapper/RAID volumes and path traversal are not accepted. The application does not enumerate physical local disks."),
        (e.mode == Mode::Create).then(|| blocks![
            input(
                "storage-local-identity",
                "Operator device identity statement",
                &e.declaration,
                edit(|e, text| e.declaration = text.to_owned())
            )
            .help("Operator statement: physical disk/partition identity and intended use"),
            caption("This is your declaration, not server-verified evidence. XAPI does not fully verify device identity, capacity, partitions, signatures or current use. An empty repository probe is not proof that a device is blank, unused or readable; aliases may refer to the same disk."),
            checkbox(
                "storage-local-ack",
                "I verified this device; erase its existing data",
                e.local_acknowledged,
                change(|e| e.local_acknowledged = !e.local_acknowledged)
            ),
        ]),
        (e.mode == Mode::Introduce).then(|| {
            let confirmed = e
                .local_introduction
                .as_ref()
                .is_some_and(|(kind, uuid)| *kind == e.kind && uuid == e.uuid.trim());
            blocks![
                caption("EXT and XFS probes inspect the same volume-group prefix. A result does not establish filesystem type. Choose the backend you independently verified for this exact repository UUID."),
                checkbox(
                    "storage-local-driver-confirm",
                    "I independently verified this repository's selected driver",
                    confirmed,
                    change(|e| {
                        let selected = (e.kind, e.uuid.trim().to_owned());
                        e.local_introduction = if e.local_introduction.as_ref() == Some(&selected) {
                            None
                        } else {
                            Some(selected)
                        };
                    })
                ),
            ]
        }),
        e.local_probe
            .as_ref()
            .filter(|(host, path, _)| Some(*host) == e.host && path == e.path.trim())
            .map(|(_, _, empty)| {
                caption(if *empty {
                    "The local-storage probe returned no known repository. This does not establish device safety."
                } else {
                    "The probe found existing storage. Select it to introduce it; formatting is refused."
                })
                .key("storage-local-probe")
            }),
    ]
}

fn allocation_choice(e: &Editor) -> Vec<Block> {
    if !matches!(e.kind, 2 | 4) || e.mode != Mode::Create {
        return Vec::new();
    }
    const UNADVERTISED: &str =
        "Allocation choices require the host driver to advertise the allocation option.";
    let qualified = editor::allocation_advertised(e);
    let all = [
        StorageAllocation::DriverDefault,
        StorageAllocation::Thick,
        StorageAllocation::Thin,
    ];
    blocks![
        choice(
            "storage-allocation",
            all.map(|allocation| {
                option(allocation.label(), allocation.label())
                    .key(format!("storage-allocation-{}", allocation.label()))
                    .disabled_if(
                        !qualified && allocation != StorageAllocation::DriverDefault,
                        UNADVERTISED,
                    )
            }),
            Some(e.allocation.label()),
            move |m, id| {
                let picked = all.into_iter().find(|allocation| allocation.label() == id);
                if let (Some(e), Some(picked)) = (&mut m.areas.storage.editor, picked) {
                    e.allocation = picked;
                }
            }
        )
        .label("Allocation for the new repository")
        .segmented(),
        (!qualified).then(|| caption(UNADVERTISED)),
    ]
}

/// Every edit or toggle here moves `auth_epoch`, so a discovery made with
/// other credentials is refused.
fn credential_fields(e: &Editor) -> Vec<Block> {
    if !matches!(e.kind, 2 | 3 | 8) {
        return Vec::new();
    }
    blocks![
        checkbox(
            "storage-auth",
            "Use storage credentials",
            e.authenticate,
            change(|e| {
                e.authenticate = !e.authenticate;
                e.auth_epoch += 1;
            })
        ),
        e.authenticate.then(|| blocks![
            (e.kind == 2).then(|| checkbox(
                "storage-mutual",
                "Mutual CHAP",
                e.mutual,
                change(|e| {
                    e.mutual = !e.mutual;
                    e.auth_epoch += 1;
                })
            )),
            input(
                "storage-username",
                "Storage username",
                &e.username,
                edit(|e, text| {
                    e.auth_epoch += 1;
                    e.username = text.to_owned();
                })
            )
            .help(if matches!(e.kind, 3 | 8) {
                "SMB username (DOMAIN\\user if needed)"
            } else {
                "CHAP username"
            }),
            secret_input(
                "storage-password",
                "Storage password",
                &e.password,
                edit(|e, text| {
                    e.auth_epoch += 1;
                    e.password.set(text);
                })
            )
            .placeholder("Storage password"),
            (e.kind == 2 && e.mutual).then(|| blocks![
                caption("Incoming CHAP credentials"),
                input(
                    "storage-incoming-username",
                    "Incoming CHAP username",
                    &e.incoming_username,
                    edit(|e, text| {
                        e.auth_epoch += 1;
                        e.incoming_username = text.to_owned();
                    })
                ),
                secret_input(
                    "storage-incoming-password",
                    "Incoming CHAP password",
                    &e.incoming_password,
                    edit(|e, text| {
                        e.auth_epoch += 1;
                        e.incoming_password.set(text);
                    })
                ),
            ]),
        ]),
    ]
}

fn host_choice(m: &Model, e: &Editor) -> Option<Choice> {
    let inventory = m.inventory_for(e.pool)?;
    let options: Vec<OptionItem> = inventory
        .hosts
        .values()
        .take(128)
        .map(|host| {
            let existing = e.mode == Mode::Attachment
                && inventory.details.pbds.values().any(|p| {
                    p.host == Some(host.id)
                        && p.sr
                            .is_some_and(|sr| e.target == ResourceTarget::Storage(sr))
                });
            option(
                host.id.to_string(),
                format!(
                    "{}{}",
                    host.name,
                    if existing {
                        " · attachment exists"
                    } else {
                        ""
                    }
                ),
            )
            .disabled_if(
                existing,
                "This host already has an attachment of this repository.",
            )
        })
        .collect();
    let selected = e.host.map(|host| host.to_string());
    Some(
        choice("storage-host", options, selected.as_deref(), |m, id| {
            if let Ok(host) = HostId::parse(id) {
                editor::set_host(m, host);
            }
        })
        .label("Host for creation, attachment or discovery")
        .dropdown(),
    )
}

/// The discovery results. Selecting a row only fills the form.
fn results(m: &Model, e: &Editor) -> Vec<Block> {
    let source = format!(
        "{} · host {}",
        if editor::is_local(e) {
            "Local device repository probe".into()
        } else if e.kind == 4 {
            "HBA discovery".into()
        } else {
            format!("Discovered through {}", e.server)
        },
        e.host
            .map(|id| id.to_string())
            .unwrap_or_else(|| "unknown".into())
    );
    blocks![
        caption(source).key("storage-discovery-source"),
        discovery_results(m, e),
        actions([button(
            "storage-results-back",
            "Back to storage form",
            change(|e| e.discovered = None)
        )]),
    ]
}

fn discovery_results(m: &Model, e: &Editor) -> Vec<Block> {
    let Some(result) = &e.discovered else {
        return Vec::new();
    };
    if !editor::results_current(m, e) {
        return blocks![caption(
            "Inputs changed; discover again before selecting a result."
        )];
    }
    let (title, rows): (&str, Vec<String>) = match result {
        StorageDiscoveryResult::LocalRepositories(rows) => (
            "Known local repositories",
            rows.iter()
                .map(|r| {
                    format!(
                        "Reported by {} · {}\n{} · driver must be independently verified",
                        r.probe_kinds
                            .iter()
                            .map(|k| k.label())
                            .collect::<Vec<_>>()
                            .join(" / "),
                        r.repository.name,
                        r.repository.uuid
                    )
                })
                .collect(),
        ),
        StorageDiscoveryResult::HbaDevices(rows) => (
            "Available HBA devices",
            rows.iter()
                .map(|r| {
                    format!(
                        "{}\n{} · {} · Serial {}",
                        r.scsi_id,
                        bytes(r.size_bytes),
                        r.vendor,
                        r.serial
                    )
                })
                .collect(),
        ),
        StorageDiscoveryResult::NfsExports(rows) => (
            "NFS exports",
            rows.iter()
                .map(|r| format!("{}\nAccess reported by server: {}", r.path, r.access))
                .collect(),
        ),
        StorageDiscoveryResult::IscsiTargets(rows) => (
            "iSCSI targets",
            rows.iter()
                .map(|r| format!("{}\nPortal {} · port {}", r.iqn, r.portal, r.port))
                .collect(),
        ),
        StorageDiscoveryResult::IscsiLuns(rows) => (
            "iSCSI LUNs",
            rows.iter()
                .map(|r| {
                    format!(
                        "{}\nLUN {} · {} · {} {}",
                        r.scsi_id,
                        r.lun.map(|n| n.to_string()).unwrap_or("unknown".into()),
                        r.size_bytes.map(bytes).unwrap_or("unknown capacity".into()),
                        r.vendor,
                        r.serial
                    )
                })
                .collect(),
        ),
        StorageDiscoveryResult::Repositories(rows) => (
            "Existing repositories",
            rows.iter()
                .map(|r| format!("{} · {}", r.name, r.uuid))
                .collect(),
        ),
        StorageDiscoveryResult::Backends(_) => return Vec::new(),
    };
    let empty = rows.is_empty().then(|| {
        caption(if matches!(result, StorageDiscoveryResult::LocalRepositories(_)) {
            "The available EXT, LVM and XFS probes reported no known repository. This is NOT proof that the device is valid, blank or unused. Return to the form to review your device declaration."
        } else if matches!(result, StorageDiscoveryResult::HbaDevices(_)) {
            "The backend reported no eligible HBA devices. Check zoning, host visibility and existing storage use, then discover again."
        } else {
            "The backend reported no matching results. You can enter a known location manually."
        })
    });
    let page = e.result_page;
    let total = rows.len();
    let listed: Vec<Block> = rows
        .iter()
        .enumerate()
        .skip(page * 20)
        .take(20)
        .flat_map(|(index, text)| {
            blocks![
                caption(text.clone()),
                actions([button(
                    format!("storage-result-{index}"),
                    "Use this result",
                    move |m| editor::select_result(m, index)
                )]),
            ]
        })
        .collect();
    blocks![
        heading(title),
        caption(format!(
            "{total} results. Selecting a row only fills the form; review remains required."
        )),
        empty,
        listed,
        (total > 20).then(|| actions([
            button(
                "storage-results-previous",
                "Previous",
                change(|e| e.result_page = e.result_page.saturating_sub(1))
            )
            .disabled_if(page == 0, "This is the first page."),
            button(
                "storage-results-next",
                "Next",
                change(|e| e.result_page += 1)
            )
            .disabled_if((page + 1) * 20 >= total, "This is the last page."),
        ])),
    ]
}

/// What the user types to confirm a destructive review: the prompt, the key
/// and the label of its field.
fn confirmation(operation: &StorageOperation) -> Option<(String, &'static str, &'static str)> {
    match operation {
        StorageOperation::Create {
            location: StorageLocation::Local { device, .. },
            ..
        } => Some((
            format!("Type the complete device path to confirm permanent data loss: {device}"),
            "storage-confirm-device",
            "Device path to confirm permanent data loss",
        )),
        StorageOperation::Create {
            location: StorageLocation::Hba { scsi_id, .. },
            ..
        } => Some((
            format!("Type the selected LUN SCSI ID to confirm formatting: {scsi_id}"),
            "storage-confirm-lun",
            "LUN SCSI ID to confirm formatting",
        )),
        StorageOperation::Forget { .. } | StorageOperation::Destroy { .. } => Some((
            "Type the repository UUID to confirm".into(),
            "storage-confirm-uuid",
            "Type the repository UUID to confirm",
        )),
        _ => None,
    }
}

fn reviewing(
    m: &Model,
    e: &Editor,
    target: ResourceTarget,
    operation: &StorageOperation,
) -> Vec<Block> {
    let mut lines = vec![format!("Target UUID: {}", target.uuid())];
    lines.extend(editor::describe(operation));
    if let Some(label) = &e.review_auth_label {
        lines.push(label.clone());
        lines.push("Credentials are sent through temporary server Secret objects and cleared from this form after submission. Verified provisioning removes unused temporary Secrets; uncertain outcomes retain their identities for explicit cleanup.".into());
    }
    let disks = match operation {
        StorageOperation::Forget { expected_disks, .. }
        | StorageOperation::Destroy { expected_disks, .. } => expected_disks.as_slice(),
        _ => &[],
    };
    let described = review(REVIEW, lines)
        .affected(
            disks
                .iter()
                .take(100)
                .map(|disk| property("Disk", disk.to_string())),
        )
        .warnings(editor::too_many_disks(operation).then(|| TOO_MANY.to_owned()))
        .stale(editor::stale(m, e));
    blocks![
        heading("Review storage change"),
        described,
        confirmation(operation).map(|(prompt, key, label)| {
            let field = input(
                key,
                label,
                &e.confirmation,
                edit(|e, text| e.confirmation = text.to_owned()),
            );
            if prompt == label {
                field
            } else {
                field.help(prompt)
            }
        }),
    ]
}

/// The hosts unused blocks can be reclaimed through: the attached ones,
/// once the driver of the repository advertises `SR_TRIM`.
fn trim_choices(m: &Model, e: &Editor) -> Vec<Block> {
    let ResourceTarget::Storage(sr) = e.target else {
        return Vec::new();
    };
    if !e.trim {
        return Vec::new();
    }
    if !editor::trim_supported(m, e) {
        return blocks![e.discovery.is_none().then(|| {
            caption("The storage driver of this repository has not advertised reclaiming unused blocks (SR_TRIM).")
                .key("storage-trim-unsupported")
        })];
    }
    let Some(inventory) = m.inventory_for(e.pool) else {
        return Vec::new();
    };
    let writable = m.is_synthetic_pool(e.pool) || m.can_mutate_pool(e.pool);
    let hosts: Vec<Button> = inventory
        .details
        .pbds
        .values()
        .filter(|p| p.sr == Some(sr) && p.currently_attached == Some(true))
        .take(128)
        .filter_map(|p| p.host)
        .map(|host| {
            button(
                format!("storage-trim-{host}"),
                format!("Reclaim unused blocks via host {host}…"),
                move |m| editor::review_trim(m, host),
            )
            .disabled_if(!writable, NOT_WRITABLE)
        })
        .collect();
    blocks![heading(e.name.clone()), actions(hosts)]
}

/// Temporary storage credentials an authenticated operation of this pool
/// left behind, with the way to review their cleanup. Not while a review is
/// open.
fn retained(m: &Model, e: &Editor) -> Vec<Block> {
    if e.review.is_some() || e.cleanup_review.is_some() {
        return Vec::new();
    }
    let writable = m.can_mutate_pool(e.pool);
    m.resource_rows
        .iter()
        .filter(|r| {
            r.pool == e.pool
                && matches!(r.action, ResourceAction::AuthenticatedStorage { .. })
                && r.storage_secrets.iter().any(|s| !s.cleaned)
        })
        .take(20)
        .flat_map(|record| {
            let id = record.id;
            let terminal = record.state.terminal();
            let secrets: Vec<Text> = record
                .storage_secrets
                .iter()
                .filter(|v| !v.cleaned)
                .map(|v| caption(format!("Secret {}", v.secret)))
                .collect();
            blocks![
                caption(format!(
                    "Temporary storage credentials retained · operation {id}"
                )),
                secrets,
                actions([button(
                    format!("storage-cleanup-review-{id}"),
                    "Review unused Secret cleanup…",
                    move |m| editor::stage_cleanup(m, id)
                )
                .disabled_if(!terminal, RESOLVE)
                .disabled_if(!writable, NOT_WRITABLE)]),
                (!terminal).then(|| caption(RESOLVE)),
            ]
        })
        .collect()
}

fn footer(m: &Model, e: &Editor, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let close = button("storage-close", "Close", super::close);
    let synthetic = m.is_synthetic_pool(e.pool);
    let writable = synthetic || m.can_mutate_pool(e.pool);
    let denial = permission.and_then(permissions::denial);
    if e.cleanup_review.is_some() {
        return vec![
            close,
            button(
                "storage-cleanup-back",
                "Back",
                change(|e| e.cleanup_review = None),
            ),
            button(
                "storage-cleanup-apply",
                "Delete unused temporary Secrets",
                editor::apply_cleanup,
            )
            .danger()
            .disabled_if(synthetic, DEMO)
            .disabled_if(!writable, NOT_WRITABLE)
            .disabled_opt(denial),
        ];
    }
    if let Some((_, _, operation)) = &e.review {
        let expected = editor::confirmation_text(e, operation);
        let confirm = button("storage-confirm", "Confirm storage change", editor::apply);
        let confirm = if expected.is_some() {
            confirm.danger()
        } else {
            confirm.primary()
        };
        // The first reason wins: nothing typed helps in a synthetic pool or
        // on a connection that cannot write.
        let confirm = confirm
            .disabled_if(synthetic, DEMO)
            .disabled_if(!writable, NOT_WRITABLE)
            .disabled_opt(editor::stale(m, e))
            .disabled_opt(denial)
            .disabled_if(editor::too_many_disks(operation), TOO_MANY)
            .disabled_opt(
                expected
                    .filter(|expected| e.confirmation != *expected)
                    .and_then(|_| confirmation(operation))
                    .map(|(prompt, _, _)| prompt),
            );
        // A review opened by its command has no draft to go back to.
        return if e.mode == Mode::Manage {
            vec![close, confirm]
        } else {
            vec![
                close,
                button("storage-back", "Back", editor::back),
                confirm,
            ]
        };
    }
    if e.mode == Mode::Manage || e.discovered.is_some() {
        return vec![close];
    }
    let review = button("storage-review", "Review", editor::review).primary();
    if e.mode == Mode::Attachment {
        return vec![close, review];
    }
    let back = button(
        "storage-back",
        "Back",
        change(|e| e.step = e.step.saturating_sub(1)),
    );
    let next = button("storage-next", "Next", change(|e| e.step = (e.step + 1).min(2))).primary();
    match e.step {
        0 => vec![close, next],
        1 => vec![close, back, next],
        _ => vec![close, back, review],
    }
}
