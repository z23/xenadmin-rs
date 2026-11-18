//! The open surfaces, described. Nothing here changes the model.
//!
//! Surfaces (all outside the supported core, so all carry the notice):
//!
//! | Id | Title | State |
//! | --- | --- | --- |
//! | `host-power-settings` | Host power-on settings | the power editor: draft, or review |
//! | `host-backup` | Filesystem backup | the panel in mode Backup: file, or review |
//! | `host-support` | Support bundle | the panel in mode Support: categories, crashdumps, file, or review |
//! | `host-restore` | Stage recovery | the panel in mode Restore: file, or review with the typed host UUID |
//! | `host-crashdump-delete` | Delete retained host crashdump | the panel with a crashdump under review |
//!
//! Rune keys kept: `host-power-mode-0|1|2` (the options of the mode choice;
//! they were three buttons), `host-power-address`, `host-power-username`,
//! `host-power-password`, `host-power-cleanup-<id>`, `host-power-back`,
//! `host-power-confirm`, `host-power-review`; `host-support-cap-<key>` (a
//! checkbox; it was a toggle button), `host-crashdump-previous`,
//! `host-crashdump-next`, `host-crashdump-delete-<uuid>`,
//! `host-support-file`, `host-crashdump-confirmation`,
//! `host-restore-confirmation`, `host-support-close`,
//! `host-crashdump-submit`, `host-support-submit`, `host-support-review`.
//!
//! New keys, for elements Rune left without one: `host-power-close`,
//! `host-power-mode` (the choice), `host-power-current`,
//! `host-power-change-review` (and so `-stale`), `host-power-preview`,
//! `host-power-error`, `host-power-retained-<id>`; `host-support-stale`,
//! `host-support-reading`, `host-support-capabilities-error`,
//! `host-crashdump-count`, `host-crashdump-page`, `host-support-chosen`,
//! `host-archive-review` and `host-crashdump-review` (and so their
//! `-stale`), `host-support-error`, `host-support-preview`,
//! `host-support-history`, `host-support-back`.
//!
//! No successor: `host-support-backup`, `host-support-bundle` and
//! `host-support-restore` (the mode row: three commands) and
//! `host-support-open` (the button of the details pane).

use xenadmin_core::{HostArchiveOperation, TransferAction, TransferFile};

use super::commands::NOT_WRITABLE;
use super::power::{self, Editor};
use super::support::{self, Mode, Panel};
use crate::prelude::*;

pub const POWER: &str = "host-power-settings";
pub const BACKUP: &str = "host-backup";
pub const SUPPORT: &str = "host-support";
pub const RESTORE: &str = "host-restore";
pub const CRASHDUMP: &str = "host-crashdump-delete";

/// The keys of the reviews, so `<key>-stale` is the stale line of each.
pub const POWER_REVIEW: &str = "host-power-change-review";
pub const ARCHIVE_REVIEW: &str = "host-archive-review";
pub const CRASHDUMP_REVIEW: &str = "host-crashdump-review";

const POWER_PREVIEW: &str = "Synthetic preview · no host changes can be submitted";
const SUPPORT_PREVIEW: &str = "Synthetic preview · no host requests are submitted";

/// The reason a synthetic pool gives a disabled confirm button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// The prompts of the two typed confirmations (Rune captions), which are
/// also the reasons of their disabled buttons.
pub const TYPE_HOST: &str = "Type the host UUID to confirm backup partition replacement";
pub const TYPE_DUMP: &str = "Type the crashdump UUID to confirm permanent deletion";

/// Reasons Rune's disabled buttons did not give. New texts.
pub const NO_FILE: &str = "Choose the archive file first.";
pub const NO_CATEGORY: &str = "Select at least one advertised diagnostic category.";
pub const INSPECTING: &str = "The chosen file is being inspected.";
/// The session keeps 64 resource operations at most (Rune refused
/// silently; the words are the core's for a full queue).
pub fn busy() -> String {
    xenadmin_core::Error::Busy.to_string()
}

/// Back to front: the panel, then the power editor (Rune's ranking).
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let a = &m.areas.host_services;
    a.support
        .as_ref()
        .map(|p| support_surface(m, p))
        .into_iter()
        .chain(a.power.as_ref().map(|e| power_surface(m, e)))
        .collect()
}

// -- Power-on settings ------------------------------------------------------

fn power_surface(m: &Model, e: &Editor) -> Surface {
    let permission = e
        .review
        .as_ref()
        .map(|op| permissions::assess_for(m, e.pool(), &op.permission_requirements()));
    let synthetic = power::synthetic(m, e);
    let body = blocks![
        properties([
            property("Host", e.target.uuid().to_string()).copyable(),
            property("Pool", e.pool().to_string()).copyable(),
        ]),
        match &e.review {
            Some(op) => {
                blocks![review(POWER_REVIEW, power::lines(op)).stale(power::stale(m, e))]
            }
            None => power_draft(m, e),
        },
        synthetic.then(|| notice(NoticeKind::Preview, POWER_PREVIEW).key("host-power-preview")),
        e.error
            .clone()
            .map(|error| alert(error).key("host-power-error")),
    ];
    let close = button("host-power-close", "Close", power::close);
    let footer = if e.review.is_some() {
        vec![
            close,
            button("host-power-back", "Back", power::back),
            button("host-power-confirm", "Confirm host change", power::submit)
                .danger()
                // The first reason wins: stale, then a known denial, then
                // the demo.
                .disabled_opt(power::stale(m, e))
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_if(synthetic, DEMO),
        ]
    } else {
        vec![
            close,
            button("host-power-review", "Review", power::review).primary(),
        ]
    };
    let surface = Surface::dialog(POWER, "Host power-on settings")
        .subtitle(format!("{} ({})", e.label(), e.target.uuid()))
        .experimental(true)
        .body(body)
        .footer(footer)
        .on_close(power::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn power_draft(m: &Model, e: &Editor) -> Vec<Block> {
    let current = e
        .expected
        .as_ref()
        .map(power::summary)
        .unwrap_or_else(|| "Unavailable or unsupported custom configuration".into());
    let mode = |mode: u8, label: &str| {
        option(mode.to_string(), label).key(format!("host-power-mode-{mode}"))
    };
    let selected = e.mode.to_string();
    let ipmi = if e.mode == 2 {
        blocks![
            input("host-power-address", "BMC IP address", &e.address, |m, t| {
                power::edit(m, |e| e.address = t.to_owned());
            })
            .on_submit(power::review),
            input(
                "host-power-username",
                "BMC username",
                &e.username,
                |m, t| {
                    power::edit(m, |e| e.username = t.to_owned());
                }
            )
            .on_submit(power::review),
            secret_input(
                "host-power-password",
                "New BMC password",
                &e.password,
                |m, t| {
                    power::edit(m, |e| e.password.set(t));
                }
            )
            .on_submit(power::review),
            caption(
                "Uses the XCP-ng IPMI plugin with LAN-plus. IPMI plugin and ipmitool availability are supplied by the host package; saving settings is not a power-on test. Legacy iLO/DRAC and custom plugin maps are not edited here."
            ),
        ]
    } else {
        Vec::new()
    };
    let retained: Vec<Block> = power::retained(m, e)
        .into_iter()
        .flat_map(|record| {
            let id = record.id;
            blocks![
                caption(format!("{} · {:?}", record.action.label(), record.state))
                    .key(format!("host-power-retained-{id}")),
                caption(record.detail.clone()),
                record.state.terminal().then(|| {
                    button(
                        format!("host-power-cleanup-{id}"),
                        "Review unused Secret cleanup",
                        move |m| power::cleanup(m, id),
                    )
                }),
            ]
        })
        .collect();
    blocks![
        caption(format!("Current: {current}")).key("host-power-current"),
        choice(
            "host-power-mode",
            [
                mode(0, "Disabled"),
                mode(1, "Wake-on-LAN"),
                mode(2, "IPMI")
            ],
            Some(selected.as_str()),
            |m, id| {
                let Ok(mode) = id.parse::<u8>() else { return };
                // Rune disabled the button of the mode that is set: choosing
                // it again must not drop the typed password.
                let same = m
                    .areas
                    .host_services
                    .power
                    .as_ref()
                    .is_none_or(|e| e.mode == mode);
                if !same {
                    power::change_mode(m, mode);
                }
            }
        )
        .label("Remote power-on")
        .segmented(),
        ipmi,
        retained,
    ]
}

// -- Backup, status report, staging, crashdump deletion -----------------------

fn support_surface(m: &Model, p: &Panel) -> Surface {
    let (id, title) = if p.deletion.is_some() {
        (CRASHDUMP, "Delete retained host crashdump")
    } else {
        match p.mode {
            Mode::Backup => (BACKUP, "Filesystem backup"),
            Mode::Support => (SUPPORT, "Support bundle"),
            Mode::Restore => (RESTORE, "Stage recovery"),
        }
    };
    let synthetic = support::synthetic(m, p);
    let stale = (!support::current(m, p)).then(|| support::STALE.to_owned());
    let body = blocks![
        if let Some(dump) = &p.deletion {
            deletion_review(p, dump, stale.clone())
        } else if let Some(action) = &p.review {
            archive_review(p, action, stale.clone())
        } else {
            chooser(m, p, stale.is_some())
        },
        p.error
            .clone()
            .map(|error| alert(error).key("host-support-error")),
        synthetic.then(|| notice(NoticeKind::Preview, SUPPORT_PREVIEW).key("host-support-preview")),
    ];
    Surface::dialog(id, title)
        .subtitle(format!("{} · {} · Host {}", p.name, p.version, p.host))
        .experimental(true)
        .body(body)
        .footer(support_footer(m, p, synthetic, stale))
        .on_close(support::close)
}

fn deletion_review(
    p: &Panel,
    dump: &xenadmin_core::HostCrashdump,
    stale: Option<String>,
) -> Vec<Block> {
    blocks![
        review(
            CRASHDUMP_REVIEW,
            [
                format!("Crashdump {}", dump.uuid),
                format!("{} · {}", dump.timestamp, bytes(dump.size)),
                "This asks the captured host to permanently remove this retained crashdump directory and its XAPI record. It can remove raw memory and logs needed for investigation. It does not create an export, affect another dump or remove previously downloaded support files. There is no undo.".to_owned(),
                "A support log bundle does not include the raw memory dump. Raw dumps cannot be downloaded here. Retain any evidence you need before deleting.".to_owned(),
                "A lost or failed response remains uncertain even if the record disappears. Recheck never repeats deletion. Successful verification establishes API completion and record absence, not an independent inspection of the host filesystem.".to_owned(),
            ]
        )
        .stale(stale),
        caption(TYPE_DUMP),
        input(
            "host-crashdump-confirmation",
            "Confirm crashdump UUID",
            &p.confirmation,
            confirm
        ),
    ]
}

fn confirm(m: &mut Model, text: &str) {
    if let Some(p) = &mut m.areas.host_services.support {
        p.confirmation = text.to_owned();
    }
}

fn archive_review(p: &Panel, action: &TransferAction, stale: Option<String>) -> Vec<Block> {
    let file = p.file.as_ref().map(TransferFile::name).unwrap_or_default();
    let mut lines = vec![format!(
        "{}: {file}",
        if p.mode == Mode::Restore {
            "Source archive"
        } else {
            "Save as"
        }
    )];
    if p.mode == Mode::Restore {
        lines.push("This reformats the host's configured BACKUP_PARTITION and replaces its contents with the selected filesystem backup. The API selects that partition; its current device path is not available to this client. Running host configuration is not restored, and no reboot or pool database restoration is performed.".into());
        lines.push("Use only a backup you trust. Identity, checksum and archive structure checks do not establish that arbitrary system files are authentic or safe to install.".into());
        if let TransferAction::Host {
            operation: HostArchiveOperation::StageRestore { archive: a },
            ..
        } = action
        {
            lines.push(format!(
                "{} entries · {} logical contents",
                a.entries,
                bytes(a.logical_bytes)
            ));
            lines.push(format!(
                "SHA-256 {}",
                a.sha256
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ));
        }
    } else {
        lines.push("Creates a new private file (0600). An existing file is never overwritten. The selected host's task and archive format are verified before the file is published.".into());
    }
    if let TransferAction::Host {
        operation: HostArchiveOperation::Support { entries },
        ..
    } = action
    {
        lines.push(format!("Requested categories: {}", entries.join(", ")));
    }
    blocks![
        heading(action.label()),
        review(ARCHIVE_REVIEW, lines).stale(stale),
        (p.mode == Mode::Restore).then(|| {
            input(
                "host-restore-confirmation",
                TYPE_HOST,
                &p.confirmation,
                confirm,
            )
        }),
    ]
}

fn chooser(m: &Model, p: &Panel, stale: bool) -> Vec<Block> {
    let about = match p.mode {
        Mode::Support => report(m, p, stale),
        Mode::Restore => blocks![caption(
            "Select a trusted gzip-tar filesystem backup from this exact host and product version. The archive is inspected without extraction. Staging will replace the configured backup partition for later installer-assisted recovery; it does not restore the running host."
        )],
        Mode::Backup => blocks![caption(
            "Downloads the host's dom0 root filesystem, including sensitive system files and a fresh pool database copy. VM disks are not included. This archive is intended for installer-assisted recovery; it is not a running-host configuration restore."
        )],
    };
    blocks![
        stale.then(|| text(support::STALE)
            .tone(Tone::Danger)
            .key("host-support-stale")),
        about,
        button(
            "host-support-file",
            if p.pending {
                "Inspecting archive file…"
            } else if p.mode == Mode::Restore {
                "Choose trusted backup…"
            } else {
                "Choose new archive file…"
            },
            support::choose
        )
        .disabled_if(p.pending, INSPECTING),
        p.file.as_ref().map(|f| {
            caption(format!(
                "{}: {}",
                if p.mode == Mode::Restore {
                    "Source"
                } else {
                    "Destination"
                },
                f.name()
            ))
            .key("host-support-chosen")
        }),
    ]
}

/// The status report: the diagnostic categories the server offers, and the
/// crashdumps it keeps, each with the way to the review of its deletion.
fn report(m: &Model, p: &Panel, stale: bool) -> Vec<Block> {
    let mut body = blocks![
        caption(
            "Support bundles contain selected diagnostics and may include private names, addresses and logs. They are saved locally; nothing is uploaded to a support service."
        ),
        p.query
            .is_some()
            .then(|| caption("Reading advertised diagnostic categories…")
                .key("host-support-reading")),
    ];
    let Some(v) = &p.overview else {
        return body;
    };
    if let Some(e) = &v.capabilities_error {
        body.extend(blocks![
            text(e.clone())
                .tone(Tone::Danger)
                .key("host-support-capabilities-error")
        ]);
    }
    for capability in &v.capabilities {
        let key = capability.key.clone();
        body.extend(blocks![
            checkbox(
                format!("host-support-cap-{}", capability.key),
                capability.key.clone(),
                p.selected.contains(&capability.key),
                move |m| support::toggle(m, &key),
            )
            .help(format!(
                "Private data: {} · maximum {} · up to {} seconds",
                capability.privacy,
                capability
                    .maximum_bytes
                    .map(bytes)
                    .unwrap_or_else(|| "unknown size".into()),
                capability
                    .maximum_seconds
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ))
        ]);
    }
    body.extend(blocks![caption(
        "Crashdump logs exclude the raw memory dump (coredump.bin). Raw dump download is not supplied by this host API."
    )]);
    if let Some(e) = &v.crashdumps_error {
        body.extend(blocks![
            caption(format!("Crashdump catalogue unavailable: {e}")).key("host-crashdump-count")
        ]);
        return body;
    }
    body.extend(blocks![
        caption(format!("{} crashdump records", v.crashdumps.len())).key("host-crashdump-count")
    ]);
    let (page, pages) = support::dump_pages(p);
    if pages > 1 {
        body.extend(blocks![
            actions([
                button(
                    "host-crashdump-previous",
                    "Previous dumps",
                    support::previous_dumps
                )
                .disabled_if(page == 0, "This is the first page."),
                button("host-crashdump-next", "Next dumps", support::next_dumps)
                    .disabled_if(page + 1 == pages, "This is the last page."),
            ]),
            caption(format!("Page {} of {}", page + 1, pages)).key("host-crashdump-page"),
        ]);
    }
    let synthetic = support::synthetic(m, p);
    let writable = m.can_mutate_pool(p.pool_id);
    for dump in v
        .crashdumps
        .iter()
        .skip(page * support::DUMPS_PER_PAGE)
        .take(support::DUMPS_PER_PAGE)
    {
        let reviewed = dump.clone();
        body.extend(blocks![
            caption(format!(
                "{} · {} · {}",
                dump.uuid,
                dump.timestamp,
                bytes(dump.size)
            )),
            button(
                format!("host-crashdump-delete-{}", dump.uuid),
                "Review deletion…",
                move |m| support::review_deletion(m, reviewed.clone()),
            )
            .disabled_if(!synthetic && !writable, NOT_WRITABLE)
            .disabled_if(!synthetic && stale, support::STALE),
        ]);
    }
    body
}

fn support_footer(m: &Model, p: &Panel, synthetic: bool, stale: Option<String>) -> Vec<Button> {
    let writable = m.can_mutate_pool(p.pool_id);
    let close = button("host-support-close", "Close", support::close);
    let history = button(
        "host-support-history",
        if p.deletion.is_some() {
            "Activity history"
        } else {
            "Transfer history"
        },
        support::history,
    );
    let back = button("host-support-back", "Back", support::back);
    if let Some(dump) = &p.deletion {
        let submit = button(
            "host-crashdump-submit",
            "Permanently delete crashdump",
            support::submit_deletion,
        )
        .danger()
        // The first reason wins. A synthetic pool behaves like a real one
        // up to the last step: it asks for the UUID, then says it is a demo.
        .disabled_opt(stale)
        .disabled_if(!synthetic && !writable, NOT_WRITABLE)
        .disabled_if(m.resource_operations.len() >= 64, busy())
        .disabled_if(p.confirmation != dump.uuid.to_string(), TYPE_DUMP)
        .disabled_if(synthetic, DEMO);
        return vec![close, history, back, submit];
    }
    if p.review.is_some() {
        let restore = p.mode == Mode::Restore;
        let submit = if restore {
            button(
                "host-support-submit",
                "Replace backup partition",
                support::submit,
            )
            .danger()
        } else {
            button(
                "host-support-submit",
                "Download private archive",
                support::submit,
            )
            .primary()
        }
        .disabled_opt(stale)
        .disabled_if(restore && !synthetic && !writable, NOT_WRITABLE)
        .disabled_if(restore && p.confirmation != p.host.to_string(), TYPE_HOST)
        .disabled_if(synthetic, DEMO);
        return vec![close, history, back, submit];
    }
    let review = button(
        "host-support-review",
        if p.mode == Mode::Restore {
            "Review recovery staging"
        } else {
            "Review download"
        },
        support::review,
    )
    .primary()
    .disabled_opt(stale)
    .disabled_if(support::action(p).is_none() && p.mode == Mode::Support, NO_CATEGORY)
    .disabled_if(p.file.is_none() || support::action(p).is_none(), NO_FILE);
    vec![close, history, review]
}
