//! What the area shows, described: the HA tab of a pool, the surfaces of
//! the four editors, and the two pages of the pool's Properties dialog.
//! Nothing here changes the model.
//!
//! Keys and labels are the Rune ones where the control still exists:
//! `pool-confirmation`, `pool-confirm`, `pool-compression`, `pool-https`,
//! `pool-tls`, `pool-secret`, `pool-ha-enable`, `pool-eject`,
//! `ha-timeout-input`, `ha-timeout-review`, `ha-timeout-reset`,
//! `ha-startup-<i>`, `ha-startup-review-<i>`, `pool-dissolve-identity`,
//! `pool-dissolve-confirm`, `join-destination-<i>`, `join-username`,
//! `join-password`, `join-confirmation`, `join-submit`,
//! `root-password-new`, `-repeat`, `-save`, `-confirmation`, `-close`,
//! `-review`, `-submit`.
//!
//! New keys, for elements Rune left without one: `pool-close`, `pool-back`,
//! `pool-change-review` (and `-stale`), `pool-error`, `pool-preview`,
//! `pool-confirmation-request`, `pool-member`, `ha-failures-input`,
//! `ha-failures-review`, `ha-heartbeat-<sr>`, `ha-priority-<value>`,
//! `ha-vm`, `ha-state`, `ha-heartbeat`, `ha-priorities`, `ha-vm-filter`,
//! `ha-vms`; `pool-dissolve-review`, `-preview`, `-error`, `-close`,
//! `-request`; `join-source`, `join-review`, `join-error`, `join-cancel`,
//! `join-back`, `join-request`; `root-password-review-lines`,
//! `root-password-request`, `root-password-message`, `root-password-back`.
//!
//! Rune keys that are commands now and so no elements: `pool-ha-disable`,
//! `ha-overcommit`, `pool-handover`, `pool-dissolve-open`, `pool-join-open`,
//! `pool-root-password`, `pool-security-open`.

use xenadmin_core::{
    HaRestartPriority, Inventory, PermissionAssessment, PoolAdministration, PoolId, PoolOperation,
    VmId,
};

use super::commands::HA_RESTART_PRIORITY;
use super::policy::{self, Editor, Kind};
use super::{DEMO, HA_FIRST, dissolve, join, root_password};
use crate::prelude::*;

/// The key of the review of a pool operation.
pub const REVIEW: &str = "pool-change-review";

/// The Configure HA wizard and its steps.
pub const CONFIGURE: &str = "pool-ha-configure";
pub const CONFIGURE_STEPS: [&str; 2] = ["Heartbeat storage", "Review"];

/// The Add Server to Pool wizard and its steps.
pub const JOIN: &str = "pool-join";
pub const JOIN_STEPS: [&str; 2] = ["Destination", "Review"];

pub const DISSOLVE: &str = "pool-dissolve";

/// The Rune key of the password dialog.
pub const ROOT_PASSWORD: &str = "root-password-dialog";

/// The id of the pool's Properties dialog, which is the kernel's.
pub const PROPERTIES: &str = PropertiesKind::Pool.surface_id();

const PREVIEW: &str = "Synthetic preview · submission disabled";

// -- The HA tab ---------------------------------------------------------------------

/// The HA tab of a pool: the state of high availability, the heartbeat
/// repositories while it is on, and the VMs with their restart priority and
/// startup settings. What changes any of it is on the local action row.
pub fn tab(m: &Model, tab: TabId, scope: &Target) -> Option<TabBody> {
    if tab != TabId::Ha {
        return None;
    }
    let pool = scope.single_of(ObjectKind::Pool)?.pool?;
    let inventory = m.inventory_for(pool)?;
    let a = policy::administration(&inventory);
    Some(TabBody::Blocks(blocks![
        group("High availability", state(&a)).key("ha-state"),
        heartbeat(&inventory, &a),
        group("VM restart priorities", vms(m, &inventory, pool)).key("ha-priorities"),
    ]))
}

fn count(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "Unknown".into())
}

fn state(a: &PoolAdministration) -> Vec<Block> {
    blocks![
        properties([
            property("HA", policy::known(a.ha_enabled)),
            property("Configured failures", count(a.ha_failures_to_tolerate)),
            property("Current plan", count(a.ha_plan_exists_for)),
            property(
                "Overcommitted",
                match a.ha_overcommitted {
                    Some(true) => "Yes",
                    Some(false) => "No",
                    None => "Unknown",
                }
            ),
            property("Allow HA overcommit", policy::known(a.ha_allow_overcommit)),
            property("Persistent timeout default", timeout_default(a)),
        ]),
        precedence(a),
    ]
}

fn timeout_default(a: &PoolAdministration) -> &str {
    if a.ha_timeout_known {
        a.ha_timeout_override.as_deref().unwrap_or("host default")
    } else {
        "unknown"
    }
}

fn precedence(a: &PoolAdministration) -> Option<Text> {
    a.ha_configuration_timeout.as_ref().map(|value| {
        caption(format!(
            "HA configuration timeout takes precedence: {value}. This view does not replace it."
        ))
    })
}

/// The heartbeat repositories, as Rune listed them: only while HA is on.
fn heartbeat(inventory: &Inventory, a: &PoolAdministration) -> Option<Group> {
    if a.ha_enabled != Some(true) {
        return None;
    }
    let body = if a.heartbeat_srs_known {
        let lines = a.heartbeat_srs.iter().map(|sr| {
            let name = inventory
                .storage
                .iter()
                .find(|r| r.uuid == sr.uuid())
                .map(|r| r.name.as_str())
                .unwrap_or("Unknown storage");
            property("Heartbeat", format!("{name} ({sr})")).copyable()
        });
        blocks![properties(lines)]
    } else {
        blocks![caption("Heartbeat storage identity is unavailable")]
    };
    Some(group("Heartbeat repositories", body).key("ha-heartbeat"))
}

fn vms(m: &Model, inventory: &Inventory, pool: PoolId) -> Vec<Block> {
    let state = &m.areas.pool;
    let needle = state.filter.to_lowercase();
    let mut guests: Vec<_> = inventory
        .guests()
        .filter(|vm| shown(vm, &needle))
        .collect();
    guests.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)));
    let rows: Vec<Row> = guests
        .into_iter()
        .map(|vm| {
            let startup = policy::startup_values(inventory, vm.id);
            let setting = |i: usize| {
                startup
                    .map(|v| v[i].to_string())
                    .unwrap_or_else(|| "Unknown".into())
            };
            row(
                vm.id,
                [
                    vm.name.clone(),
                    policy::current_priority(inventory, vm.id)
                        .map(HaRestartPriority::label)
                        .unwrap_or("Unknown / unsupported")
                        .to_owned(),
                    setting(0),
                    setting(1),
                    setting(2),
                ],
            )
            .object(ObjectRef::vm(pool, vm.id))
        })
        .collect();
    let selected = state
        .selected_vm
        .as_deref()
        .filter(|id| rows.iter().any(|row| row.id == *id));
    let columns = [
        column("name", "Name"),
        column("priority", "Restart priority"),
        column("order", "Order").right(),
        column("start-delay", "Start delay").right(),
        column("shutdown-delay", "Shutdown delay").right(),
    ];
    blocks![
        input(
            "ha-vm-filter",
            "Filter virtual machines",
            &state.filter,
            |m, text| {
                m.areas.pool.filter = text.to_owned();
                drop_hidden_row(m);
            }
        ),
        table("ha-vms", columns, rows)
            .single(selected, select_vm_row)
            .on_activate(|m, id| {
                select_vm_row(m, id);
                let target = m.target();
                m.invoke(HA_RESTART_PRIORITY, &target);
            })
            .on_clear(|m| {
                m.areas.pool.selected_vm = None;
                m.select_in_grid(Target::none());
            })
            .empty("No virtual machine matches the filter."),
    ]
}

/// Whether a VM is in the grid under a filter: by name or UUID.
fn shown(vm: &xenadmin_core::Vm, needle: &str) -> bool {
    vm.name.to_lowercase().contains(needle) || vm.id.to_string().contains(needle)
}

/// A chosen row that the filter hides is no target of the local row any
/// more.
fn drop_hidden_row(m: &mut Model) {
    let Some(selected) = m.areas.pool.selected_vm.clone() else {
        return;
    };
    let needle = m.areas.pool.filter.to_lowercase();
    let pool = m.scope().single_of(ObjectKind::Pool).and_then(|o| o.pool);
    let visible = pool.and_then(|pool| m.inventory_for(pool)).is_some_and(|i| {
        i.guests()
            .any(|vm| vm.id.to_string() == selected && shown(vm, &needle))
    });
    if !visible {
        m.areas.pool.selected_vm = None;
        m.select_in_grid(Target::none());
    }
}

/// A row of the VM grid is chosen: it is the target of the local row.
pub(super) fn select_vm_row(m: &mut Model, id: &str) {
    m.areas.pool.selected_vm = Some(id.to_owned());
    let pool = m.scope().single_of(ObjectKind::Pool).and_then(|o| o.pool);
    let target = pool
        .zip(VmId::parse(id).ok())
        .map(|(pool, vm)| Target::one(ObjectRef::vm(pool, vm)))
        .unwrap_or_else(Target::none);
    m.select_in_grid(target);
}

// -- Surfaces -----------------------------------------------------------------------

/// Back to front. In practice one is open: each `open` closes the others.
pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.pool;
    let mut out = Vec::new();
    // An editor that is a page is the kernel's Properties dialog ([`page`]).
    if let Some(e) = state.editor.as_ref().filter(|e| e.kind.page().is_none()) {
        out.push(change(m, e));
    }
    if let Some(e) = &state.dissolve {
        out.push(dissolve_dialog(m, e));
    }
    if let Some(e) = &state.join {
        out.push(join_wizard(m, e));
    }
    if let Some(e) = &state.root_password {
        out.push(password_dialog(m, e));
    }
    out
}

// -- Pool operations ----------------------------------------------------------------

fn surface_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Security | Kind::Network => PROPERTIES,
        Kind::Configure => CONFIGURE,
        Kind::Disable => "pool-change-ha-disable",
        Kind::Failures => "pool-change-ha-failures",
        Kind::Overcommit => "pool-change-ha-overcommit",
        Kind::Timeout => "pool-change-ha-timeout",
        Kind::Protection => "pool-change-vm-protection",
        Kind::Handover => "pool-change-handover",
        Kind::Eject => "pool-change-eject",
    }
}

/// The permission line belongs to the review: before it there is no
/// operation to assess.
fn permission(m: &Model, e: &Editor) -> Option<PermissionAssessment> {
    e.review
        .as_ref()
        .map(|op| permissions::assess_for(m, e.pool, &op.permission_requirements()))
}

fn change(m: &Model, e: &Editor) -> Surface {
    let permission = permission(m, e);
    let id = surface_id(e.kind);
    let surface = match e.kind {
        Kind::Configure => Surface::wizard(id, e.kind.title(), CONFIGURE_STEPS, e.step()),
        _ => Surface::dialog(id, e.kind.title()),
    }
    .subtitle(format!("{} ({})", e.name, e.pool))
    .experimental(true)
    .body(body(m, e))
    .footer(footer(m, e, permission.as_ref()))
    .on_close(super::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

/// The pages this area has for the Properties dialog of a pool whose
/// inventory is there. Both are outside the supported part.
pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    let known = target
        .single_of(ObjectKind::Pool)
        .and_then(|object| object.pool)
        .is_some_and(|pool| m.inventory_for(pool).is_some());
    if !known {
        return Vec::new();
    }
    [Kind::Network, Kind::Security]
        .into_iter()
        .filter_map(Kind::page)
        .map(|id| {
            if id == super::policy::ADVANCED_PAGE {
                // Outside the plan's table: it comes last.
                PropertyPage::new(id, "Advanced", PropertyPage::UNPLANNED)
            } else {
                PropertyPage::planned(PropertiesKind::Pool, id)
            }
            .experimental(true)
        })
        .collect()
}

/// The page that is showing: the editor `page_event` created when the page
/// was entered, described as a dialog of this area is.
pub fn page(m: &Model, _target: &Target, id: &str) -> PageView {
    let editor = m.areas.pool.editor.as_ref();
    let Some(e) = editor.filter(|e| e.kind.page() == Some(id)) else {
        return PageView::default();
    };
    let permission = permission(m, e);
    PageView {
        body: body(m, e),
        footer: footer(m, e, permission.as_ref()),
        footer_note: permission.as_ref().map(permissions::footer_note),
    }
}

fn body(m: &Model, e: &Editor) -> Vec<Block> {
    blocks![
        // A page says which it is; a dialog has it as its title.
        e.kind.page().map(|_| heading(e.kind.title())),
        match &e.review {
            Some(op) => reviewed(m, e, op),
            None => draft(e),
        },
        e.error.clone().map(|error| alert(error).key("pool-error")),
        policy::synthetic(m, e).then(|| notice(NoticeKind::Preview, PREVIEW).key("pool-preview")),
    ]
}

fn reviewed(m: &Model, e: &Editor, op: &PoolOperation) -> Vec<Block> {
    let confirmation = match policy::confirmation(op, e.pool) {
        Some(required) => blocks![
            caption(format!("Type this UUID to confirm: {required}"))
                .key("pool-confirmation-request"),
            input(
                "pool-confirmation",
                "Confirmation UUID",
                &e.confirmation,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.editor {
                        e.confirmation = text.to_owned();
                    }
                }
            ),
        ],
        None => Vec::new(),
    };
    blocks![
        heading(op.label()),
        review(REVIEW, policy::describe(e, op)).stale(policy::stale(m, e)),
        confirmation,
        caption(policy::CLOSING),
    ]
}

/// A button that stages a prebuilt operation (the Rune `action`).
fn stage(key: &str, label: impl Into<String>, op: PoolOperation, disabled: Option<&str>) -> Button {
    button(key, label, move |m| policy::stage(m, op.clone())).disabled_opt(disabled)
}

fn draft(e: &Editor) -> Vec<Block> {
    let a = e.administration();
    match e.kind {
        Kind::Network => match a.migration_compression {
            Some(expected) => blocks![stage(
                "pool-compression",
                format!(
                    "Migration compression: {} · change",
                    policy::known(Some(expected))
                ),
                PoolOperation::SetMigrationCompression {
                    expected,
                    enabled: !expected,
                },
                None,
            )],
            None => blocks![caption(format!(
                "Migration compression: {}",
                policy::known(None)
            ))],
        },
        Kind::Security => blocks![
            match a.https_only {
                Some(expected) => blocks![stage(
                    "pool-https",
                    format!("HTTPS only: {} · change", policy::known(Some(expected))),
                    PoolOperation::SetHttpsOnly {
                        expected,
                        enabled: !expected,
                    },
                    None,
                )],
                None => blocks![caption(format!("HTTPS only: {}", policy::known(None)))],
            },
            caption(format!(
                "Pool certificate verification: {}",
                policy::known(a.tls_verification_enabled)
            )),
            (a.tls_verification_enabled == Some(false)).then(|| stage(
                "pool-tls",
                "Enable pool TLS verification…",
                PoolOperation::EnableTlsVerification,
                None,
            )),
            (a.secret_rotation_pending == Some(false)).then(|| stage(
                "pool-secret",
                "Rotate pool authentication secret…",
                PoolOperation::RotateSecret,
                (a.ha_enabled != Some(false)).then_some(HA_FIRST),
            )),
        ],
        Kind::Configure => {
            let candidates = policy::heartbeat_candidates(&e.inventory);
            let none = candidates.is_empty().then(|| {
                caption("No shared storage repository with user content is in the readable inventory.")
            });
            let boxes: Vec<Field> = candidates
                .into_iter()
                .map(|(id, name)| {
                    checkbox(
                        format!("ha-heartbeat-{id}"),
                        name,
                        e.heartbeat.contains(&id),
                        move |m| policy::toggle_heartbeat(m, id),
                    )
                })
                .collect();
            blocks![
                caption(
                    "Select heartbeat storage. Every host must already be attached; the server validates eligibility."
                ),
                none,
                boxes,
            ]
        }
        Kind::Failures => blocks![
            input(
                "ha-failures-input",
                "Host failures to tolerate",
                &e.failures,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.editor {
                        e.failures = text.to_owned();
                    }
                }
            )
            .on_submit(policy::review_failures)
        ],
        Kind::Timeout => blocks![
            caption(format!(
                "Persistent timeout default: {}",
                timeout_default(&a)
            )),
            precedence(&a),
            if policy::timeout_editable(&a) {
                blocks![
                    input(
                        "ha-timeout-input",
                        "HA timeout default (seconds)",
                        &e.timeout,
                        |m, text| {
                            if let Some(e) = &mut m.areas.pool.editor {
                                e.timeout = text.to_owned();
                            }
                        }
                    )
                    .placeholder("10–86400 seconds")
                    .on_submit(|m| policy::timeout(m, false))
                ]
            } else {
                blocks![caption(
                    "Timeout defaults are editable with HA disabled and a fully readable configuration."
                )]
            },
        ],
        Kind::Protection => protection(e),
        Kind::Eject => members(e, &a),
        Kind::Disable | Kind::Overcommit | Kind::Handover => Vec::new(),
    }
}

fn priority_key(value: HaRestartPriority) -> &'static str {
    match value {
        HaRestartPriority::Restart => "ha-priority-restart",
        HaRestartPriority::BestEffort => "ha-priority-best-effort",
        HaRestartPriority::Disabled => "ha-priority-disabled",
    }
}

/// The chosen VM: its priority with the three it can be given, then its
/// startup order and delays, one reviewed operation per setting.
fn protection(e: &Editor) -> Vec<Block> {
    let Some(vm) = e.vm else {
        return blocks![caption("Select a VM")];
    };
    let name = e
        .inventory
        .vms
        .get(&vm)
        .map(|v| v.name.as_str())
        .unwrap_or("Unknown");
    let current = policy::current_priority(&e.inventory, vm);
    let priorities = policy::PRIORITIES.map(|value| {
        button(priority_key(value), value.label(), move |m| {
            policy::priority(m, value)
        })
        .disabled_if(
            current.is_none(),
            "The current priority is unknown or unsupported.",
        )
        .disabled_if(current == Some(value), "This is the current priority.")
    });
    blocks![
        caption(format!("VM: {name} ({vm})")).key("ha-vm"),
        caption(format!(
            "Current priority: {}",
            current
                .map(HaRestartPriority::label)
                .unwrap_or("Unknown / unsupported")
        )),
        actions(priorities),
        heading("Startup order and delays"),
        startup(e, vm),
    ]
}

fn startup(e: &Editor, vm: VmId) -> Vec<Block> {
    let details = e.inventory.details.vms.get(&vm);
    let old = policy::startup_values(&e.inventory, vm);
    let Some(old) = old.filter(|_| details.is_some_and(|d| d.appliance_known)) else {
        return blocks![caption(
            "Startup settings or vApp membership are unknown; editing is unavailable."
        )];
    };
    let mut out = blocks![caption(
        "These values are also used by vApps. Lower order values start first; delay values are seconds. Changing them does not start or stop the VM."
    )];
    for (i, field) in policy::FIELDS.iter().enumerate() {
        out.extend(blocks![
            caption(format!("{} · current {}", field.label(), old[i])),
            input(
                format!("ha-startup-{i}"),
                field.label(),
                &e.startup[i],
                move |m, text| {
                    if let Some(e) = &mut m.areas.pool.editor {
                        e.startup[i] = text.to_owned();
                    }
                }
            )
            .on_submit(move |m| policy::startup(m, i)),
            button(
                format!("ha-startup-review-{i}"),
                format!("Review {}…", field.label().to_lowercase()),
                move |m| policy::startup(m, i)
            ),
        ]);
    }
    out
}

/// Pool > Remove Server: the member is chosen here (the Rune list of the
/// pool's servers, at most 256).
fn members(e: &Editor, a: &PoolAdministration) -> Vec<Block> {
    let options: Vec<OptionItem> = e
        .inventory
        .hosts
        .values()
        .take(256)
        .map(|host| {
            let coordinator = if a.coordinator == Some(host.id) {
                " · coordinator"
            } else {
                ""
            };
            option(host.id.to_string(), format!("{}{coordinator}", host.name))
        })
        .collect();
    let selected = e.host.map(|host| host.to_string());
    blocks![
        caption(
            "Choose a member for planned coordinator handover or ejection. Both require HA to be disabled first."
        ),
        choice("pool-member", options, selected.as_deref(), |m, id| {
            if let Some(e) = &mut m.areas.pool.editor {
                e.host = e
                    .inventory
                    .hosts
                    .keys()
                    .copied()
                    .find(|host| host.to_string() == id);
                e.error = None;
            }
        })
        .label("Pool membership"),
    ]
}

fn footer(m: &Model, e: &Editor, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let close = button("pool-close", "Close", super::close);
    let Some(op) = &e.review else {
        let a = e.administration();
        return match e.kind {
            Kind::Configure => vec![
                close,
                button("pool-ha-enable", "Review enabling HA", policy::review_enable)
                    .primary()
                    .disabled_if(
                        a.ha_failures_to_tolerate.is_none(),
                        "HA failure tolerance is unavailable",
                    )
                    .disabled_if(
                        e.heartbeat.is_empty(),
                        "Select heartbeat storage first.",
                    ),
            ],
            Kind::Failures => vec![
                close,
                button(
                    "ha-failures-review",
                    "Review failure tolerance",
                    policy::review_failures,
                )
                .primary(),
            ],
            Kind::Timeout if policy::timeout_editable(&a) => vec![
                close,
                button("ha-timeout-reset", "Restore host timeout default…", |m| {
                    policy::timeout(m, true)
                })
                .disabled_if(
                    a.ha_timeout_override.is_none(),
                    "The host timeout default is already in use.",
                ),
                button("ha-timeout-review", "Review timeout default…", |m| {
                    policy::timeout(m, false)
                })
                .primary(),
            ],
            Kind::Eject => {
                let refusal = match e.host {
                    Some(host) => policy::member_refusal(&e.inventory, host, false),
                    None => Some("Select a member"),
                };
                vec![
                    close,
                    button("pool-eject", "Eject selected member…", policy::review_eject)
                        .danger()
                        .disabled_opt(refusal),
                ]
            }
            _ => vec![close],
        };
    };
    let confirm = button("pool-confirm", "Confirm change", policy::apply);
    // Danger exactly when the review asks for a typed UUID, as in Rune.
    let confirm = if policy::confirmation(op, e.pool).is_some() {
        confirm.danger()
    } else {
        confirm.primary()
    }
    // The first reason wins: stale, a known denial, the demo, the UUID.
    .disabled_opt(policy::stale(m, e))
    .disabled_opt(permission.and_then(permissions::denial))
    .disabled_if(policy::synthetic(m, e), DEMO)
    .disabled_if(policy::unconfirmed(e), policy::CONFIRM);
    if e.direct {
        vec![close, confirm]
    } else {
        vec![close, button("pool-back", "Back", policy::back), confirm]
    }
}

// -- Make into Standalone Server ----------------------------------------------------

fn dissolve_dialog(m: &Model, e: &dissolve::Editor) -> Surface {
    let permission = permissions::assess_for(m, e.pool, &e.operation.permission_requirements());
    let synthetic = dissolve::synthetic(m, e);
    let current = dissolve::current(m, e);
    let stale = (!synthetic && !current).then_some(dissolve::NOT_CURRENT);
    Surface::dialog(DISSOLVE, "Make into Standalone Server")
        .subtitle(format!("{} ({})", e.operation.expected_name, e.pool))
        .experimental(true)
        .body(blocks![
            heading("Dissolve pool"),
            review("pool-dissolve-review", dissolve::lines(e))
                .warnings(dissolve::enabled_warning(e))
                .stale(stale),
            synthetic.then(|| {
                notice(
                    NoticeKind::Preview,
                    "Synthetic preview; submission is disabled.",
                )
                .key("pool-dissolve-preview")
            }),
            e.error
                .clone()
                .map(|error| alert(error).key("pool-dissolve-error")),
            caption(format!("Enter the pool UUID to confirm: {}", e.pool))
                .key("pool-dissolve-request"),
            input(
                "pool-dissolve-identity",
                "Confirmation pool UUID",
                &e.confirmation,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.dissolve {
                        e.confirmation = text.to_owned();
                    }
                }
            ),
        ])
        .footer([
            button("pool-dissolve-close", "Close", |m| {
                m.areas.pool.dissolve = None;
            }),
            button(
                "pool-dissolve-confirm",
                "Dissolve reviewed pool",
                dissolve::apply,
            )
            .danger()
            .disabled_if(synthetic, DEMO)
            .disabled_opt(stale)
            .disabled_opt(dissolve::enabled_warning(e))
            .disabled_if(
                e.confirmation != e.pool.to_string(),
                dissolve::UNCONFIRMED,
            )
            .disabled_opt(permissions::denial(&permission)),
        ])
        .footer_note(permissions::footer_note(&permission))
        .on_close(|m| m.areas.pool.dissolve = None)
}

// -- Add Server to Pool -------------------------------------------------------------

fn join_wizard(m: &Model, e: &join::Editor) -> Surface {
    let synthetic = join::synthetic(m, e);
    let reviewing = join::chosen(e).filter(|_| e.review);
    let stale = (reviewing.is_some()
        && !synthetic
        && !(m.workspace.writable && join::current(m, e)))
    .then_some(join::CHANGED);
    let step = match reviewing {
        Some(d) => blocks![
            heading("Review destination"),
            review(
                "join-review",
                [
                    format!(
                        "{}\nPool: {}\nCoordinator: {} ({})\nHost-to-host address: {}\nDesktop connection: {}",
                        d.name, d.pool, d.host_name, d.host, d.address, d.profile.endpoint
                    ),
                    "The standalone host and its halted guests move into this pool. Both pools must already have HA disabled; source guests must be halted, with no shared guest storage, bonds or tunnels. These prerequisites are checked again; this action does not prepare them automatically.".to_owned(),
                    "The destination account is checked using its existing desktop connection's trust settings first. Credentials are used only for this join and are never saved. The old source connection remains bound to its original pool; a disconnect or lost reply may require rechecking through the destination pool.".to_owned(),
                ]
            )
            .warning("Destination credentials are sent to the source host for XAPI's pool join. XAPI initially bootstraps the host-to-host connection without certificate validation before exchanging pool certificates. Desktop certificate trust does not secure that bootstrap. Proceed only on a trusted management network.")
            .stale(stale),
            input(
                "join-username",
                "Destination local administrator",
                &e.username,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.join {
                        e.username = text.to_owned();
                    }
                }
            ),
            secret_input(
                "join-password",
                "Destination password · this join only",
                &e.password,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.join {
                        e.password.set(text);
                    }
                }
            )
            .placeholder("Password"),
            caption(format!(
                "Type source host UUID to confirm: {}",
                e.source.host
            ))
            .key("join-request"),
            input(
                "join-confirmation",
                "Source host UUID",
                &e.confirmation,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.join {
                        e.confirmation = text.to_owned();
                    }
                }
            ),
        ],
        None => {
            let destinations: Vec<Block> = e
                .destinations
                .iter()
                .enumerate()
                .map(|(index, d)| {
                    actions([button(
                        format!("join-destination-{index}"),
                        format!("{} · {}", d.name, d.pool),
                        move |m| join::choose(m, index),
                    )
                    .disabled_if(!d.current(m), super::NOT_WRITABLE)])
                })
                .collect();
            blocks![
                caption(
                    "Choose an independently connected destination pool. Both connections must be writable. Only a standalone source host can join."
                ),
                e.destinations.is_empty().then(|| caption(
                    "Connect the destination pool first, then reopen this view."
                )),
                destinations,
            ]
        }
    };
    let cancel = button("join-cancel", "Cancel", join::close);
    let footer = if reviewing.is_some() {
        vec![
            cancel,
            button("join-back", "Back", join::back),
            button("join-submit", "Join pool", join::apply)
                .danger()
                .disabled_if(synthetic, DEMO)
                .disabled_opt(stale)
                .disabled_if(
                    e.username.trim().is_empty(),
                    "Enter the destination local administrator.",
                )
                .disabled_if(e.password.is_empty(), "Enter the destination password.")
                .disabled_if(
                    e.confirmation != e.source.host.to_string(),
                    join::CONFIRM,
                ),
        ]
    } else {
        vec![cancel]
    };
    Surface::wizard(
        JOIN,
        "Add Server to Pool",
        JOIN_STEPS,
        usize::from(reviewing.is_some()),
    )
    .subtitle(format!("{} ({})", e.source.host_name, e.source.host))
    .experimental(true)
    .body(blocks![
        heading("Join another pool"),
        caption(format!(
            "Source: {}\nHost: {} ({})\nOriginal pool: {}",
            e.source.name, e.source.host_name, e.source.host, e.source.pool
        ))
        .key("join-source"),
        step,
        e.error.clone().map(|error| alert(error).key("join-error")),
    ])
    .footer(footer)
    .on_close(join::close)
}

// -- Change Server Password ---------------------------------------------------------

fn password_dialog(m: &Model, e: &root_password::Editor) -> Surface {
    let pending = e.operation.is_none();
    let permission = (e.review && pending).then(|| root_password::permission(m, e));
    let synthetic = root_password::synthetic(m, e);
    let current = root_password::current(m, e);
    let stale = (!synthetic && !current).then_some(root_password::STALE);
    let entry = if !pending {
        Vec::new()
    } else if e.review {
        blocks![
            heading("Review password change"),
            review(
                "root-password-review-lines",
                [if e.save {
                    "After verified completion, update the existing saved root password for this connection profile only."
                } else {
                    "Use the new password for this session only. The desktop password store is unchanged."
                }]
            )
            .stale(stale),
            caption(format!("Type the pool UUID to confirm: {}", e.pool))
                .key("root-password-request"),
            input(
                "root-password-confirmation",
                "Confirmation pool UUID",
                &e.confirmation,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.root_password {
                        e.confirmation = text.to_owned();
                    }
                }
            ),
        ]
    } else {
        blocks![
            secret_input(
                "root-password-new",
                "New root password",
                &e.password,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.root_password {
                        e.password.set(text);
                    }
                }
            ),
            secret_input(
                "root-password-repeat",
                "Repeat new root password",
                &e.repeated,
                |m, text| {
                    if let Some(e) = &mut m.areas.pool.root_password {
                        e.repeated.set(text);
                    }
                }
            )
            .on_submit(root_password::review),
            root_password::saves(e).then(|| checkbox(
                "root-password-save",
                "Update this profile's saved password after verified completion",
                e.save,
                |m| {
                    if let Some(e) = &mut m.areas.pool.root_password {
                        e.save = !e.save;
                    }
                }
            )),
        ]
    };
    // Rune gave the message the alert role and the caption's colour: it is
    // an error, a status or the outcome.
    let message = e.message.clone().map(|message| {
        let mut line = text(message).key("root-password-message");
        line.alert = true;
        line
    });
    let close = button("root-password-close", "Close", root_password::close);
    let footer = if !pending {
        vec![close]
    } else if e.review {
        vec![
            close,
            button("root-password-back", "Back", root_password::back),
            button("root-password-submit", "Change password", root_password::submit)
                .danger()
                .disabled_if(synthetic, DEMO)
                .disabled_opt(stale)
                .disabled_opt(permission.as_ref().and_then(permissions::denial))
                .disabled_if(
                    e.confirmation.trim() != e.pool.to_string(),
                    root_password::CONFIRM,
                ),
        ]
    } else {
        vec![
            close,
            button("root-password-review", "Review change", root_password::review).primary(),
        ]
    };
    let surface = Surface::dialog(ROOT_PASSWORD, "Change Server Password")
        .subtitle(format!("{} ({})", e.name, e.pool))
        .experimental(true)
        .body(blocks![
            heading("Change pool root password"),
            caption(format!(
                "{}\nPool: {}\nCoordinator: {}\n{} pool host(s) · local account: root",
                e.name,
                e.pool,
                e.change.coordinator,
                e.change.members.len()
            )),
            caption(
                "This changes the coordinator's local root password and requests synchronization to every pool member. Individual member failures are not reported by XCP-ng. A successful coordinator login does not verify the other hosts."
            ),
            caption(
                "The current client's reconnect password changes only after a verified coordinator login. Existing connections that share this pool's old credentials close. Other saved profiles are not updated."
            ),
            entry,
            message,
        ])
        .footer(footer)
        .on_close(root_password::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}
