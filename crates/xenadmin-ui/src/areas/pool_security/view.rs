//! The Users tab and the open dialog, described. Nothing here changes the
//! model: both are rebuilt on every render from the editor as it is.
//!
//! Rune keys: `security-confirm`, and `session-permission-explanation`
//! (`session_permissions_view::notice`). Rune gave no other control of this
//! dialog a key; the labels are the Rune ones. New keys:
//!
//! - tab: `session-permission-summary`, `session-permission-roles`,
//!   `security-unread`, `security-preview`, `security-session`,
//!   `security-reconnect`, `security-section-error-<section>`,
//!   `security-directory` (grid), `security-rbac`,
//!   `security-subject-filter`, `security-subjects` (grid),
//!   `security-reading`, `security-error`;
//! - dialogs: `security-domain`, `security-account`, `security-password`,
//!   `security-subject-name`, `security-resolve`, `security-resolved`,
//!   `security-subject`, `security-role-<role>`,
//!   `security-current-fingerprint`, `security-choose-certificate`,
//!   `security-choose-key`, `security-choose-chain`, `security-file-<n>`,
//!   `security-validate`, `security-validating`,
//!   `security-replacement-fingerprint`, `security-review-join`,
//!   `security-review-leave`, `security-review-leave-accounts`,
//!   `security-review-add`, `security-review-install`,
//!   `security-review-reset`, `security-review` (and so
//!   `security-review-stale`), `security-confirmation-prompt`,
//!   `security-confirmation`, `security-close`, `security-back`.

use xenadmin_core::{PoolId, PoolSecurityOperation as SecurityOp, SecurityOverview};

use super::DEMO;
use super::commands::{RECONNECT_AS, SUBJECT_REMOVE, SUBJECT_ROLE};
use super::security::{self, Editor, Kind};
use crate::prelude::*;

/// The key of the review, so `security-review-stale` is its stale line.
pub const REVIEW: &str = "security-review";

const PREVIEW: &str = "Synthetic preview · no security state is read and no change can be submitted";

// -- The tab -----------------------------------------------------------------

pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Users {
        return None;
    }
    let pool = scope.single_of(ObjectKind::Pool)?.pool?;
    let editor = m.areas.pool_security.editor.as_ref();
    Some(TabBody::Blocks(blocks![
        session(m, pool),
        match editor.filter(|e| e.pool == pool) {
            Some(e) => status(m, e),
            // Not read: why it cannot be, or that it has not been yet.
            None => blocks![
                caption(security::refusal(m, pool).unwrap_or(security::UNREAD))
                    .key("security-unread")
            ],
        },
    ]))
}

/// The display of the session's permissions (Rune
/// `session_permissions_view::notice`, which showed only why they were
/// unknown): what the session of this pool is and the standard roles it
/// holds, or the reason nothing is known.
fn session(m: &Model, pool: PoolId) -> Group {
    let permissions = m
        .snapshot_for(pool)
        .map(|snapshot| snapshot.session_permissions())
        .unwrap_or_default();
    let known = permissions.is_local_superuser() || permissions.permission_count().is_some();
    let roles = permissions.roles();
    group(
        "Session permissions",
        if known {
            blocks![Block::Properties(
                [
                    Some(
                        property("Session", permissions.summary())
                            .key("session-permission-summary")
                    ),
                    (!roles.is_empty()).then(|| {
                        property("Roles", roles.join(", ")).key("session-permission-roles")
                    }),
                ]
                .into_iter()
                .flatten()
                .collect()
            )]
        } else {
            blocks![caption(permissions.summary()).key(permissions::SESSION_KEY)]
        },
    )
}

fn status(m: &Model, e: &Editor) -> Vec<Block> {
    let synthetic = m.is_synthetic_pool(e.pool);
    blocks![
        synthetic.then(|| notice(NoticeKind::Preview, PREVIEW).key("security-preview")),
        e.overview.as_ref().map(|v| overview(m, e, v)),
        e.query
            .is_some()
            .then(|| caption(security::READING).key("security-reading")),
        (e.overview.is_none() && e.query.is_none() && e.error.is_none() && !synthetic)
            .then(|| caption(security::UNREAD).key("security-unread")),
        // While a dialog is open the error is its own.
        e.error
            .clone()
            .filter(|_| e.dialog.is_none())
            .map(|error| alert(error).key("security-error")),
    ]
}

fn overview(m: &Model, e: &Editor, v: &SecurityOverview) -> Vec<Block> {
    let host_name = |host| {
        e.inventory
            .hosts
            .get(host)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| "Unknown host".into())
    };
    let search = e.subject_name.to_lowercase();
    let role_names = |roles: &[xenadmin_core::RoleId]| {
        roles
            .iter()
            .map(|id| {
                v.roles
                    .iter()
                    .find(|role| role.id == *id)
                    .map(|role| role.name.clone())
                    .unwrap_or_else(|| id.to_string())
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let selected = e.subject.map(|id| id.to_string());
    blocks![
        caption(if v.is_local_superuser {
            security::SUPERUSER
        } else {
            security::READ_ONLY
        })
        .key("security-session"),
        (!v.is_local_superuser).then(|| reconnect(m, e)),
        v.errors
            .iter()
            .map(|(section, error)| {
                caption(format!("{section}: {error}"))
                    .tone(Tone::Warning)
                    .key(format!("security-section-error-{section}"))
            })
            .collect::<Vec<_>>(),
        group(
            "Active Directory",
            blocks![
                table(
                    "security-directory",
                    [
                        ("server", "Server"),
                        ("authentication", "Authentication"),
                        ("service", "Service"),
                    ],
                    v.directory_hosts.iter().map(|h| {
                        row(
                            h.host,
                            [
                                host_name(&h.host),
                                if h.auth_type.is_empty() {
                                    "Local authentication".into()
                                } else {
                                    h.auth_type.clone()
                                },
                                h.service_name.clone(),
                            ],
                        )
                    }),
                )
                .empty(security::HOSTS_UNKNOWN),
                caption(match v.rbac_available {
                    Some(true) => security::RBAC_AVAILABLE,
                    Some(false) => security::RBAC_RESTRICTED,
                    None => security::RBAC_UNKNOWN,
                })
                .key("security-rbac"),
            ],
        ),
        group(
            "Directory users and groups",
            blocks![
                input(
                    "security-subject-filter",
                    "Find directory user or group",
                    &e.subject_name,
                    set_subject_name,
                )
                .placeholder("Directory user or group"),
                table(
                    "security-subjects",
                    [
                        ("name", "Name"),
                        ("identifier", "Identifier"),
                        ("roles", "Roles"),
                    ],
                    v.subjects
                        .iter()
                        .filter(|r| {
                            search.is_empty()
                                || r.name.to_lowercase().contains(&search)
                                || r.identifier.to_lowercase().contains(&search)
                        })
                        .map(|r| {
                            row(
                                r.id,
                                [r.name.clone(), r.identifier.clone(), role_names(&r.roles)],
                            )
                        }),
                )
                .single(selected.as_deref(), |m, id| {
                    let Some(e) = &mut m.areas.pool_security.editor else {
                        return;
                    };
                    let chosen = e
                        .overview
                        .as_ref()
                        .and_then(|v| v.subjects.iter().find(|r| r.id.to_string() == id))
                        .map(|r| r.id);
                    e.subject = chosen;
                })
                .on_clear(|m| {
                    if let Some(e) = &mut m.areas.pool_security.editor {
                        e.subject = None;
                    }
                })
                .row_menu([SUBJECT_ROLE, SUBJECT_REMOVE])
                .empty(if v.subjects.is_empty() {
                    "No directory user or group has access to this pool."
                } else {
                    "No directory user or group matches."
                }),
            ],
        ),
    ]
}

fn set_subject_name(m: &mut Model, text: &str) {
    if let Some(e) = &mut m.areas.pool_security.editor {
        e.subject_name = text.to_owned();
    }
}

/// Rune: "Reconnect as local administrator…", disabled in the demo and on
/// a stale editor. It runs the `connections` area's command on the pool and
/// follows its eligibility as well. This area's editor is closed when the
/// command takes over (AREAS decision 11) and kept when it refuses.
fn reconnect(m: &Model, e: &Editor) -> Option<Button> {
    let target = Target::one(ObjectRef::pool(e.pool));
    let refused = match m.eligibility(RECONNECT_AS, &target) {
        Eligibility::Hidden => return None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Enabled => None,
    };
    Some(
        button(
            "security-reconnect",
            "Reconnect as local administrator…",
            move |m| {
                let mine = m.areas.pool_security.editor.take();
                if !m.invoke(RECONNECT_AS, &target) {
                    m.areas.pool_security.editor = mine;
                }
            },
        )
        .disabled_if(m.is_synthetic_pool(e.pool), DEMO)
        .disabled_if(!security::current(m, e), security::REOPEN)
        .disabled_opt(refused),
    )
}

// -- The dialog --------------------------------------------------------------

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(e) = &m.areas.pool_security.editor else {
        return Vec::new();
    };
    let Some(kind) = e.dialog else {
        return Vec::new();
    };
    vec![
        Surface::dialog(kind.surface_id(), kind.title())
            .subtitle(subtitle(e, kind))
            .experimental(true)
            .body(body(m, e, kind))
            .footer(footer(m, e, kind))
            .on_close(super::close),
    ]
}

/// The object of the change: the server of a certificate, else the pool
/// (Rune: "{name}\n{pool}").
fn subtitle(e: &Editor, kind: Kind) -> String {
    match e.host.filter(|_| kind.certificate()) {
        Some(host) => format!(
            "{} ({host})",
            e.inventory
                .hosts
                .get(&host)
                .map(|h| h.name.as_str())
                .unwrap_or("Unknown host")
        ),
        None => format!("{} ({})", e.name, e.pool),
    }
}

fn body(m: &Model, e: &Editor, kind: Kind) -> Vec<Block> {
    blocks![
        match &e.review {
            Some(op) => reviewing(m, e, kind, op),
            None => draft(e, kind),
        },
        m.is_synthetic_pool(e.pool)
            .then(|| notice(NoticeKind::Preview, PREVIEW).key("security-preview")),
        e.error.clone().map(|error| alert(error).key("security-error")),
    ]
}

/// Rune: the label of the operation, its lines, and the pool's UUID to
/// type. The label is the dialog's title except for a role change.
fn reviewing(m: &Model, e: &Editor, kind: Kind, op: &SecurityOp) -> Vec<Block> {
    blocks![
        (op.label() != kind.title()).then(|| heading(op.label())),
        review(REVIEW, security::review_lines(e, op)).stale(security::stale(m, e)),
        caption(format!("Type the pool UUID to confirm: {}", e.pool))
            .key("security-confirmation-prompt"),
        input(
            "security-confirmation",
            "Pool UUID confirmation",
            &e.confirmation,
            |m, text| {
                if let Some(e) = &mut m.areas.pool_security.editor {
                    e.confirmation = text.to_owned();
                }
            },
        ),
    ]
}

fn draft(e: &Editor, kind: Kind) -> Vec<Block> {
    match kind {
        Kind::Join => blocks![
            input("security-domain", "Directory domain", &e.domain, |m, text| {
                if let Some(e) = &mut m.areas.pool_security.editor {
                    e.domain = text.to_owned();
                }
            })
            .placeholder("example.test"),
            credentials(e),
        ],
        Kind::Leave => blocks![credentials(e)],
        Kind::AddSubject => add_subject(e),
        Kind::RemoveSubject => Vec::new(),
        Kind::Role => roles(e),
        Kind::InstallCertificate | Kind::ResetCertificate => certificate(e, kind),
    }
}

/// The directory account: needed to join, and to remove machine accounts
/// on leaving (Rune `directory`).
fn credentials(e: &Editor) -> Vec<Block> {
    blocks![
        input(
            "security-account",
            "Directory account",
            &e.username,
            |m, text| {
                if let Some(e) = &mut m.areas.pool_security.editor {
                    e.username = text.to_owned();
                }
            }
        )
        .placeholder("Directory account"),
        secret_input(
            "security-password",
            "Directory password",
            &e.password,
            |m, text| {
                if let Some(e) = &mut m.areas.pool_security.editor {
                    e.password.set(text);
                }
            }
        )
        .placeholder("Directory password · session only"),
    ]
}

fn add_subject(e: &Editor) -> Vec<Block> {
    let resolve = |m: &mut Model| {
        let Some(e) = &m.areas.pool_security.editor else {
            return;
        };
        if e.query.is_some() || e.subject_name.is_empty() {
            return;
        }
        let name = e.subject_name.clone();
        security::query(m, xenadmin_core::PoolSecurityQuery::ResolveSubject { name });
    };
    blocks![
        input(
            "security-subject-name",
            "Find directory user or group",
            &e.subject_name,
            set_subject_name,
        )
        .placeholder("Directory user or group")
        .on_submit(resolve),
        button("security-resolve", "Resolve directory identity", resolve)
            .disabled_if(e.query.is_some(), security::READING)
            .disabled_if(
                e.subject_name.is_empty(),
                "Enter a directory user or group."
            ),
        resolved(e).map(|(name, identifier)| {
            caption(format!("Resolved: {name} · {identifier}")).key("security-resolved")
        }),
        e.query
            .is_some()
            .then(|| caption(security::READING).key("security-reading")),
    ]
}

/// The identity the directory resolved for the text as it is now: (display
/// name, identifier).
fn resolved(e: &Editor) -> Option<(&str, &str)> {
    e.resolved
        .as_ref()
        .filter(|(query, _, _)| query == &e.subject_name)
        .map(|(_, identifier, name)| (name.as_str(), identifier.as_str()))
}

/// Rune: one "Grant {role}" or "Remove {role}" button per role of the pool
/// for the chosen subject; each stages its review.
fn roles(e: &Editor) -> Vec<Block> {
    let (Some(v), Some(subject)) = (&e.overview, security::subject(e)) else {
        return blocks![caption(security::NO_SUBJECT).key("security-subject")];
    };
    blocks![
        caption(format!("{} · {}", subject.name, subject.identifier)).key("security-subject"),
        actions(v.roles.iter().map(|role| {
            let grant = !subject.roles.contains(&role.id);
            let op = SecurityOp::ChangeSubjectRole {
                subject: subject.id,
                expected_identifier: subject.identifier.clone(),
                expected_roles: subject.roles.clone(),
                role: role.id,
                grant,
            };
            button(
                format!("security-role-{}", role.id),
                format!("{} {}", if grant { "Grant" } else { "Remove" }, role.name),
                move |m| security::stage(m, op.clone()),
            )
            .disabled_opt(security::rbac(v))
            .disabled_opt(security::unsupported(v, security::role_method(grant)))
        })),
    ]
}

const FILE_BUTTONS: [(&str, &str); 3] = [
    ("security-choose-certificate", "Choose certificate…"),
    ("security-choose-key", "Choose private key…"),
    ("security-choose-chain", "Choose optional chain…"),
];

/// The certificate the server has now, read when the dialog opened.
fn fingerprint(e: &Editor) -> Option<&xenadmin_core::CertificateFingerprint> {
    e.fingerprint
        .as_ref()
        .filter(|(host, _)| e.host == Some(*host))
        .map(|(_, fingerprint)| fingerprint)
}

fn certificate(e: &Editor, kind: Kind) -> Vec<Block> {
    let Some(current) = fingerprint(e) else {
        return blocks![
            e.query
                .is_some()
                .then(|| caption(security::READING).key("security-reading")),
            (e.query.is_none() && e.error.is_none())
                .then(|| caption(security::NO_FINGERPRINT).key("security-current-fingerprint")),
        ];
    };
    blocks![
        caption(format!("Current SHA-256: {current}")).key("security-current-fingerprint"),
        (kind == Kind::InstallCertificate).then(|| blocks![
            (0..3)
                .map(|index| {
                    let (key, label) = FILE_BUTTONS[index];
                    blocks![
                        button(key, label, move |m| security::choose_file(m, index)),
                        e.files[index].as_ref().map(|path| {
                            caption(path.file_name().unwrap_or_default().to_string_lossy())
                                .key(format!("security-file-{index}"))
                        }),
                    ]
                })
                .collect::<Vec<_>>(),
            button(
                "security-validate",
                "Validate selected PEM files",
                security::load_files
            )
            .disabled_if(e.file_result.is_some(), security::VALIDATING)
            .disabled_if(
                e.files[0].is_none() || e.files[1].is_none(),
                "Choose a certificate file and a private key file."
            ),
            e.file_result
                .is_some()
                .then(|| caption(security::VALIDATING).key("security-validating")),
            e.certificate.as_ref().map(|certificate| {
                caption(format!("Replacement SHA-256: {}", certificate.fingerprint()))
                    .key("security-replacement-fingerprint")
            }),
        ]),
    ]
}

fn footer(m: &Model, e: &Editor, kind: Kind) -> Vec<Button> {
    let close = button("security-close", "Close", super::close);
    if e.review.is_some() {
        let confirm = button("security-confirm", "Confirm change", security::apply)
            .danger()
            // The first reason wins: stale, then the demo, then the UUID.
            .disabled_opt(security::stale(m, e))
            .disabled_if(m.is_synthetic_pool(e.pool), DEMO)
            .disabled_if(e.confirmation != e.pool.to_string(), security::CONFIRMATION);
        // A review that was opened by its command has no draft to go back to.
        return if kind.opens_on_review() {
            vec![close, confirm]
        } else {
            let back = button("security-back", "Back", |m| {
                if let Some(e) = &mut m.areas.pool_security.editor {
                    e.review = None;
                }
            });
            vec![close, back, confirm]
        };
    }
    let gate = security::gate(e, kind);
    match kind {
        Kind::Join => vec![
            close,
            button("security-review-join", "Review joining directory", |m| {
                security::directory(m, true, false)
            })
            .primary()
            .disabled_opt(gate),
        ],
        Kind::Leave => vec![
            close,
            button(
                "security-review-leave-accounts",
                "Review leaving and remove machine accounts",
                |m| security::directory(m, false, true),
            )
            .disabled_opt(gate.clone()),
            button(
                "security-review-leave",
                "Review leaving · local configuration only",
                |m| security::directory(m, false, false),
            )
            .primary()
            .disabled_opt(gate),
        ],
        Kind::AddSubject => {
            let identifier = resolved(e).map(|(_, identifier)| identifier.to_owned());
            let unresolved = identifier.is_none();
            vec![
                close,
                button("security-review-add", "Review adding access", move |m| {
                    if let Some(identifier) = identifier.clone() {
                        security::stage(m, SecurityOp::AddSubject { identifier });
                    }
                })
                .primary()
                .disabled_opt(gate)
                .disabled_if(unresolved, security::UNRESOLVED),
            ]
        }
        Kind::Role | Kind::RemoveSubject => vec![close],
        Kind::InstallCertificate => {
            let op = e.host.zip(fingerprint(e)).zip(e.certificate.as_ref()).map(
                |((host, expected), certificate)| SecurityOp::InstallCertificate {
                    host,
                    expected: expected.clone(),
                    fingerprint: certificate.fingerprint(),
                },
            );
            let missing = if fingerprint(e).is_none() {
                Some(security::NO_FINGERPRINT)
            } else if e.certificate.is_none() {
                Some(security::NO_CERTIFICATE)
            } else if e.certificate.as_ref().map(|c| c.fingerprint()).as_ref() == fingerprint(e) {
                Some(security::SAME_CERTIFICATE)
            } else {
                None
            };
            vec![
                close,
                button(
                    "security-review-install",
                    "Review installing certificate",
                    move |m| {
                        if let Some(op) = op.clone() {
                            security::stage(m, op);
                        }
                    },
                )
                .primary()
                .disabled_opt(gate)
                .disabled_opt(missing),
            ]
        }
        Kind::ResetCertificate => {
            let op = e
                .host
                .zip(fingerprint(e))
                .map(|(host, expected)| SecurityOp::ResetCertificate {
                    host,
                    expected: expected.clone(),
                });
            let missing = op.is_none();
            vec![
                close,
                button(
                    "security-review-reset",
                    "Review resetting to self-signed",
                    move |m| {
                        if let Some(op) = op.clone() {
                            security::stage(m, op);
                        }
                    },
                )
                .primary()
                .disabled_opt(gate)
                .disabled_if(missing, security::NO_FINGERPRINT),
            ]
        }
    }
}
