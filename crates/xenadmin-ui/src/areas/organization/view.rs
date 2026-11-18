//! The surfaces of the area, described: the dialog of one organization
//! change (`organization`), the Custom Fields page of a Properties dialog,
//! and the vApps dialog (`appliances`). Nothing here changes the model.
//!
//! Keys of Rune: `organization-review`, `organization-apply`,
//! `appliance-create-review`, `appliance-apply`. Every other element of the
//! two Rune dialogs had no key; the new ones are:
//!
//! - organization: `organization-name`, `organization-value`,
//!   `organization-date`, `organization-field-<name>` (the buttons that copy
//!   a defined field's name), `organization-action` (the four actions of the
//!   page), `organization-pool`, `organization-selected`,
//!   `organization-values`, `organization-values-unavailable`,
//!   `organization-blocked`, `organization-change-review` (and so
//!   `organization-change-review-stale`), `organization-error`,
//!   `organization-preview`, `organization-not-current`,
//!   `organization-close`, `organization-back`, `organization-refresh`;
//! - vApps: `appliance-find`, `appliance-count`, `appliance-list`,
//!   `appliance-new`, `appliance-name`, `appliance-description`,
//!   `appliance-selected`, `appliance-rename-review`,
//!   `appliance-delete-review`, `appliance-start`, `appliance-shutdown`,
//!   `appliance-force-shutdown`, `appliance-incomplete`,
//!   `appliance-vm-find`, `appliance-vm-count`, `appliance-vms`,
//!   `appliance-membership-review`, `appliance-order`,
//!   `appliance-start-delay`, `appliance-shutdown-delay`,
//!   `appliance-order-review`, `appliance-change-review`,
//!   `appliance-confirmation`, `appliance-error`, `appliance-preview`,
//!   `appliance-not-current`, `appliance-close`, `appliance-back`,
//!   `appliance-refresh`.
//!
//! The Rune lists of buttons (one per vApp, one per VM) are tables with a
//! single selection.

use xenadmin_core::{
    ApplianceOperation as Op, AppliancePower, CustomFieldType, OrganizationPatch,
    PermissionAssessment,
};

use super::appliance;
use super::edit::{self, Action, Workspace};
use crate::prelude::*;

/// The id of the dialog of an organization change (the Rune label).
pub const ORGANIZATION: &str = "organization";
/// The id of the vApps dialog.
pub const APPLIANCES: &str = "appliances";

pub const REVIEW: &str = "organization-change-review";
pub const APPLIANCE_REVIEW: &str = "appliance-change-review";

/// The Rune caption of both workspaces in the demo.
pub const PREVIEW: &str = "Synthetic preview; submission is disabled.";

/// The reason a synthetic pool gives a disabled apply button (the text
/// every area uses).
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

/// New texts, for buttons Rune disabled without a reason.
pub const NAME_NEEDED: &str = "Enter a vApp name.";
pub const NOT_EMPTY: &str =
    "Only a vApp with known, empty membership can be deleted. Remove its members first.";

pub fn surfaces(m: &Model) -> Vec<Surface> {
    let state = &m.areas.organization;
    let mut surfaces = Vec::new();
    if let Some(w) = state.editor.as_ref().filter(|w| !w.page) {
        surfaces.push(change_dialog(m, w));
    }
    if let Some(w) = &state.appliances {
        surfaces.push(vapps_dialog(m, w));
    }
    surfaces
}

fn pool_label(m: &Model, pool: xenadmin_core::PoolId) -> String {
    match m.inventory_for(pool) {
        Some(inventory) => format!("{} ({pool})", inventory.pool_name),
        None => pool.to_string(),
    }
}

// -- Tags, folders and custom fields -------------------------------------------

/// The permission line belongs to the review.
fn permission(m: &Model, w: &Workspace) -> Option<PermissionAssessment> {
    w.review
        .as_ref()
        .map(|patch| permissions::assess_for(m, w.pool, &patch.permission_requirements()))
}

fn change_dialog(m: &Model, w: &Workspace) -> Surface {
    let permission = permission(m, w);
    let surface = Surface::dialog(ORGANIZATION, w.action.title())
        .subtitle(pool_label(m, w.pool))
        .experimental(true)
        .body(body(m, w))
        .footer(footer(m, w, permission.as_ref()))
        .on_close(super::close);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

/// The Custom Fields page: one object of a kind that carries metadata, in
/// a pool that is shown. The page says for itself what it cannot change.
pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    let Some(kind) = PropertiesKind::of(target) else {
        return Vec::new();
    };
    // Several VMs or templates: the bulk page of another area.
    let Some(object) = target.single() else {
        return Vec::new();
    };
    if edit::object_id(object).is_none() || !m.exists(object) {
        return Vec::new();
    }
    vec![PropertyPage::planned(kind, edit::PAGE).experimental(true)]
}

pub fn page(m: &Model, _target: &Target, id: &str) -> PageView {
    let editor = m.areas.organization.editor.as_ref();
    let Some(w) = editor.filter(|w| w.page && id == edit::PAGE) else {
        return PageView::default();
    };
    let permission = permission(m, w);
    PageView {
        body: body(m, w),
        footer: footer(m, w, permission.as_ref()),
        footer_note: permission.as_ref().map(permissions::footer_note),
    }
}

fn body(m: &Model, w: &Workspace) -> Vec<Block> {
    let synthetic = edit::synthetic(m, w);
    blocks![
        match &w.review {
            Some(patch) => reviewed(m, w, patch),
            None => draft(w),
        },
        w.error
            .clone()
            .map(|error| alert(error).key("organization-error")),
        synthetic.then(|| notice(NoticeKind::Preview, PREVIEW).key("organization-preview")),
        // Under a review the stale line says it.
        (!synthetic && w.review.is_none() && !edit::current(m, w))
            .then(|| caption(edit::NOT_CURRENT).key("organization-not-current")),
    ]
}

fn reviewed(m: &Model, w: &Workspace, patch: &OrganizationPatch) -> Vec<Block> {
    let mut out =
        blocks![review(REVIEW, edit::review_lines(w, patch)).stale(edit::stale(m, w))];
    for change in patch.edits() {
        let [name, identity, what] = edit::change_lines(w, change);
        out.extend(blocks![group(
            name,
            blocks![caption(identity), caption(what)]
        )]);
    }
    out
}

fn name_label(action: Action) -> &'static str {
    match action {
        Action::MoveFolder => "Folder path (empty removes grouping)",
        Action::CreateFolder | Action::RenameFolder | Action::DeleteFolder => "Folder path",
        Action::DefineField | Action::DeleteField | Action::SetField | Action::RemoveField => {
            "Custom field name"
        }
        _ => "Tag",
    }
}

fn draft(w: &Workspace) -> Vec<Block> {
    let action = w.action;
    let name = input("organization-name", name_label(action), &w.name, |m, text| {
        if let Some(w) = &mut m.areas.organization.editor {
            w.name = text.to_owned();
        }
    })
    .on_submit(edit::review);
    let value = matches!(
        action,
        Action::RenameTag | Action::RenameFolder | Action::SetField
    )
    .then(|| {
        input(
            "organization-value",
            match action {
                Action::RenameTag => "New tag",
                Action::RenameFolder => "New full folder path",
                _ => "Value (dates use YYYY-MM-DD)",
            },
            &w.value,
            |m, text| {
                if let Some(w) = &mut m.areas.organization.editor {
                    w.value = text.to_owned();
                }
            },
        )
        .on_submit(edit::review)
    });
    let date = (action == Action::DefineField).then(|| {
        checkbox(
            "organization-date",
            "Date field (otherwise text)",
            w.date,
            |m| {
                if let Some(w) = &mut m.areas.organization.editor {
                    w.date = !w.date;
                }
            },
        )
    });
    // One button per defined field, which copies its name into the draft.
    let fields: Vec<Button> = if matches!(
        action,
        Action::SetField | Action::RemoveField | Action::DeleteField
    ) {
        w.inventory
            .definitions
            .iter()
            .flatten()
            .map(|definition| {
                let name = definition.name.clone();
                button(
                    format!("organization-field-{name}"),
                    name.clone(),
                    move |m| {
                        if let Some(w) = &mut m.areas.organization.editor {
                            w.name = name.clone();
                        }
                    },
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    blocks![
        w.page.then(|| on_page(w)),
        (!w.page).then(|| in_dialog(w)),
        name,
        value,
        date,
        (!fields.is_empty()).then(|| actions(fields)),
        edit::blocked(w).map(|reason| caption(reason)
            .tone(Tone::Warning)
            .key("organization-blocked")),
    ]
}

/// The head of the Custom Fields page: the values of the object, and which
/// of the four field actions the draft below is for.
fn on_page(w: &Workspace) -> Vec<Block> {
    let values = match (edit::selected(w).next(), &w.inventory.definitions) {
        (None, _) => blocks![
            caption(match w.missing.first() {
                Some(name) => format!("Custom field metadata is unavailable for {name}"),
                None => "Custom field metadata is unavailable for this object".to_owned(),
            })
            .tone(Tone::Warning)
            .key("organization-values-unavailable")
        ],
        (Some(_), None) => blocks![
            caption(edit::NO_DEFINITIONS)
                .tone(Tone::Warning)
                .key("organization-values-unavailable")
        ],
        (Some(_), Some(definitions)) if definitions.is_empty() => {
            blocks![caption("No custom field is defined in this pool.").key("organization-values")]
        }
        (Some(object), Some(definitions)) => blocks![Block::Properties(
            definitions
                .iter()
                .map(|definition| {
                    property(
                        match definition.kind {
                            CustomFieldType::Date => format!("{} (date)", definition.name),
                            _ => definition.name.clone(),
                        },
                        object
                            .custom_fields
                            .get(&definition.name)
                            .map(String::as_str)
                            .unwrap_or("unset"),
                    )
                    .key(format!("organization-value-of-{}", definition.name))
                })
                .collect()
        )],
    };
    blocks![
        values,
        choice(
            "organization-action",
            Action::FIELDS.map(|action| option(action.id(), action.title())),
            Some(w.action.id()),
            |m, id| {
                if let Some(action) = Action::FIELDS.into_iter().find(|a| a.id() == id) {
                    edit::select_action(m, action);
                }
            },
        ),
    ]
}

/// The head of the dialog of one action: what it acts on.
fn in_dialog(w: &Workspace) -> Vec<Block> {
    let selection = w.action.on_selection().then(|| {
        let mut names: Vec<&str> = edit::selected(w).map(|o| o.name.as_str()).collect();
        names.extend(w.missing.iter().map(String::as_str));
        let count = names.len();
        let shown = names.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = match count.saturating_sub(8) {
            0 => String::new(),
            more => format!(" and {more} more"),
        };
        caption(format!("{count} selected: {shown}{more}")).key("organization-selected")
    });
    let chosen = w.pool.to_string();
    let pool = (w.pools.len() > 1).then(|| {
        select(
            "organization-pool",
            "Pool",
            w.pools
                .iter()
                .map(|(pool, name)| option(pool.to_string(), name.clone())),
            Some(&chosen),
            edit::set_pool,
        )
    });
    blocks![
        selection,
        pool,
        caption("Reviews use this captured inventory; refresh to include later changes."),
    ]
}

fn footer(m: &Model, w: &Workspace, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let close = button("organization-close", "Close", super::close);
    if w.review.is_none() {
        return vec![
            close,
            button("organization-refresh", "Refresh", edit::refresh),
            button("organization-review", "Review changes", edit::review).primary(),
        ];
    }
    vec![
        close,
        button("organization-back", "Back", edit::back),
        button("organization-apply", "Apply changes", edit::submit)
            .primary()
            // The first reason wins: stale, then a known denial, then the demo.
            .disabled_opt(edit::stale(m, w))
            .disabled_opt(permission.and_then(permissions::denial))
            .disabled_if(edit::synthetic(m, w), DEMO),
    ]
}

// -- vApps ----------------------------------------------------------------------

fn close_vapps(m: &mut Model) {
    m.areas.organization.appliances = None;
}

fn vapps_dialog(m: &Model, w: &appliance::Workspace) -> Surface {
    let permission = w
        .review
        .as_ref()
        .map(|op| permissions::assess_for(m, w.pool, &op.permission_requirements()));
    let synthetic = appliance::synthetic(m, w);
    let body = blocks![
        match &w.review {
            Some(op) => vapp_review(m, w, op),
            None => vapp_editor(w),
        },
        w.error
            .clone()
            .map(|error| alert(error).key("appliance-error")),
        synthetic.then(|| notice(NoticeKind::Preview, PREVIEW).key("appliance-preview")),
        (!synthetic && w.review.is_none() && !appliance::current(m, w))
            .then(|| caption(appliance::NOT_CURRENT).key("appliance-not-current")),
    ];
    let close = button("appliance-close", "Close", close_vapps);
    let footer = match &w.review {
        Some(op) => {
            let apply = button("appliance-apply", "Apply reviewed change", appliance::apply);
            let apply = if appliance::confirmation(op).is_some() {
                apply.danger()
            } else {
                apply.primary()
            };
            vec![
                close,
                button("appliance-back", "Back", appliance::back),
                apply
                    .disabled_opt(appliance::stale(m, w))
                    .disabled_opt(permission.as_ref().and_then(permissions::denial))
                    .disabled_if(synthetic, DEMO)
                    .disabled_if(appliance::unconfirmed(w), appliance::CONFIRM),
            ]
        }
        None => vec![
            close,
            button("appliance-refresh", "Refresh", appliance::refresh),
        ],
    };
    let surface = Surface::dialog(APPLIANCES, "vApps")
        .subtitle(pool_label(m, w.pool))
        .experimental(true)
        .large()
        .body(body)
        .footer(footer)
        .on_close(close_vapps);
    match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    }
}

fn vapp_review(m: &Model, w: &appliance::Workspace, op: &Op) -> Vec<Block> {
    let confirmation = appliance::confirmation(op).map(|required| {
        blocks![
            caption(format!("Enter vApp UUID to confirm: {required}")),
            input(
                "appliance-confirmation",
                "Confirm vApp UUID",
                &w.confirmation,
                |m, text| {
                    if let Some(w) = &mut m.areas.organization.appliances {
                        w.confirmation = text.to_owned();
                    }
                }
            ),
        ]
    });
    blocks![
        heading(op.label()),
        review(APPLIANCE_REVIEW, appliance::lines(w, op)).stale(appliance::stale(m, w)),
        confirmation,
    ]
}

fn draft_input(
    key: &'static str,
    label: &'static str,
    value: &str,
    field: fn(&mut appliance::Workspace) -> &mut String,
) -> Field {
    input(key, label, value, move |m, text| {
        if let Some(w) = &mut m.areas.organization.appliances {
            *field(w) = text.to_owned();
        }
    })
}

fn vapp_editor(w: &appliance::Workspace) -> Vec<Block> {
    let query = w.group_search.to_lowercase();
    let matches: Vec<_> = w
        .inventory
        .details
        .appliances
        .values()
        .filter(|g| g.name.to_lowercase().contains(&query) || g.id.to_string().contains(&query))
        .collect();
    let chosen = w.selected.map(|id| id.to_string());
    let list = table(
        "appliance-list",
        [
            column("name", "vApp"),
            column("vms", "VMs").right(),
            column("description", "Description"),
        ],
        matches.iter().take(256).map(|group| {
            row(
                group.id,
                [
                    group.name.clone(),
                    group.vms.len().to_string(),
                    group.description.clone(),
                ],
            )
        }),
    )
    .single(chosen.as_deref(), |m, id| {
        let found = m.areas.organization.appliances.as_ref().and_then(|w| {
            w.inventory
                .details
                .appliances
                .keys()
                .copied()
                .find(|group| group.to_string() == id)
        });
        if let Some(group) = found {
            appliance::select(m, group);
        }
    })
    .empty("No vApp matches.");
    blocks![
        caption(
            "Group existing VMs and set their start/shutdown order. Refresh captures the current pool before preparing another change."
        ),
        draft_input("appliance-find", "Find vApp", &w.group_search, |w| {
            &mut w.group_search
        })
        .placeholder("Find vApp by name or UUID"),
        caption(format!(
            "{} matching vApps; showing up to 256",
            matches.len()
        ))
        .key("appliance-count"),
        list,
        actions([button("appliance-new", "New vApp", appliance::new_group)]),
        draft_input("appliance-name", "vApp name", &w.name, |w| &mut w.name)
            .placeholder("vApp name"),
        draft_input(
            "appliance-description",
            "vApp description",
            &w.description,
            |w| &mut w.description
        )
        .placeholder("Description"),
        match appliance::selected(w) {
            Some(group) => vapp_selected(w, group),
            None => blocks![actions([button(
                "appliance-create-review",
                "Review new vApp",
                appliance::stage_create
            )
            .primary()
            .disabled_if(w.name.trim().is_empty(), NAME_NEEDED)])],
        },
    ]
}

fn vapp_selected(w: &appliance::Workspace, group: &xenadmin_core::VmAppliance) -> Vec<Block> {
    let no_members = !group.vms_known || group.vms.is_empty();
    let power = |key: &'static str, title: &'static str, action: AppliancePower| {
        button(key, title, move |m| appliance::stage_power(m, action))
            .disabled_if(no_members, appliance::NO_MEMBERS)
    };
    let query = w.search.to_lowercase();
    let mut vms: Vec<_> = w
        .inventory
        .vms
        .values()
        .filter(|v| {
            !v.is_template
                && (v.name.to_lowercase().contains(&query) || v.id.to_string().contains(&query))
        })
        .collect();
    let count = vms.len();
    vms.sort_by_key(|v| (!group.vms.contains(&v.id), v.name.to_lowercase(), v.id));
    let chosen = w.vm.map(|id| id.to_string());
    let list = table(
        "appliance-vms",
        [column("name", "VM"), column("member", "Membership")],
        vms.into_iter().take(100).map(|vm| {
            row(
                vm.id,
                [
                    vm.name.clone(),
                    if group.vms.contains(&vm.id) {
                        "member".to_owned()
                    } else {
                        String::new()
                    },
                ],
            )
        }),
    )
    .single(chosen.as_deref(), |m, id| {
        let found = m.areas.organization.appliances.as_ref().and_then(|w| {
            w.inventory
                .vms
                .keys()
                .copied()
                .find(|vm| vm.to_string() == id)
        });
        if let Some(vm) = found {
            appliance::select_vm(m, vm);
        }
    })
    .empty("No VM matches.");
    let vm = w.vm.map(|id| {
        let member = group.vms.contains(&id);
        blocks![
            heading(format!("Selected VM: {}", appliance::vm_name(w, id))),
            actions([button(
                "appliance-membership-review",
                if member {
                    "Review removal from vApp"
                } else {
                    "Review addition to vApp"
                },
                appliance::stage_membership
            )]),
            member.then(|| blocks![
                caption("Order (ascending start, descending shutdown); equal orders run together."),
                draft_input("appliance-order", "vApp order", &w.order, |w| &mut w.order),
                draft_input(
                    "appliance-start-delay",
                    "Start delay seconds",
                    &w.start_delay,
                    |w| &mut w.start_delay
                )
                .help("Start delay in seconds"),
                draft_input(
                    "appliance-shutdown-delay",
                    "Shutdown delay seconds",
                    &w.shutdown_delay,
                    |w| &mut w.shutdown_delay
                )
                .help("Shutdown delay in seconds"),
                actions([button(
                    "appliance-order-review",
                    "Review order and delays",
                    appliance::stage_order
                )]),
            ]),
        ]
    });
    blocks![
        caption(format!("Selected: {} · {}", group.name, group.id)).key("appliance-selected"),
        actions([
            button(
                "appliance-rename-review",
                "Review name / description",
                appliance::stage_rename
            ),
            button(
                "appliance-delete-review",
                "Delete empty vApp",
                appliance::stage_delete
            )
            .danger()
            .disabled_if(!group.vms_known || !group.vms.is_empty(), NOT_EMPTY),
        ]),
        actions([
            power("appliance-start", "Start vApp", AppliancePower::Start),
            power(
                "appliance-shutdown",
                "Shut down vApp",
                AppliancePower::CleanShutdown
            ),
            power(
                "appliance-force-shutdown",
                "Force shutdown",
                AppliancePower::HardShutdown
            )
            .danger(),
        ]),
        (!group.vms_known).then(|| caption(
            "Membership is incomplete; refresh with permission to read this group and its VMs."
        )
        .tone(Tone::Warning)
        .key("appliance-incomplete")),
        draft_input(
            "appliance-vm-find",
            "Find vApp members or ungrouped VMs",
            &w.search,
            |w| &mut w.search
        )
        .placeholder("Filter VMs by name or UUID"),
        caption(format!(
            "{count} matching VMs; showing up to 100. Members are marked below."
        ))
        .key("appliance-vm-count"),
        list,
        vm,
    ]
}
