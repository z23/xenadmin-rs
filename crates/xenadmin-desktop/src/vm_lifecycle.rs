use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{
    GuestFirmware, NetworkId, ObservedVm, SnapshotDeletionSelection, SnapshotId, SrId, VdiId,
    VmCopyMode, VmLifecycleOperation,
};

#[path = "vm_creation.rs"]
mod creation;
#[path = "vm_creation_cloud.rs"]
mod creation_cloud;
#[path = "vm_deletion.rs"]
mod deletion;

pub(super) enum Panel {
    Editor(Box<Editor>),
    History,
    CloudCreation(Box<creation_cloud::Pending>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Create,
    Clone,
    Convert,
    Metadata,
    Delete,
}
struct Review {
    operation: VmLifecycleOperation,
    lines: Vec<String>,
}
pub(super) struct Editor {
    target: Option<ObservedVm>,
    pool: PoolId,
    source: Vm,
    kind: Kind,
    step: u8,
    name: TextEdit,
    description: TextEdit,
    vcpus: TextEdit,
    memory: TextEdit,
    disk_size: TextEdit,
    sr: Option<SrId>,
    network: Option<NetworkId>,
    iso: Option<VdiId>,
    firmware: GuestFirmware,
    bios_host: Option<xenadmin_core::HostId>,
    cloud: bool,
    cloud_input: creation_cloud::Draft,
    full_copy: bool,
    as_template: bool,
    disks: BTreeSet<VdiId>,
    snapshots: BTreeSet<SnapshotId>,
    snapshot_disks: BTreeSet<VdiId>,
    initial: Arc<Inventory>,
    review: Option<Review>,
    error: Option<String>,
}
fn editor(s: &State) -> Option<&Editor> {
    match s.vm_lifecycle.as_ref()? {
        Panel::Editor(editor) => Some(editor),
        Panel::History | Panel::CloudCreation(_) => None,
    }
}
fn editor_mut(s: &mut State) -> Option<&mut Editor> {
    match s.vm_lifecycle.as_mut()? {
        Panel::Editor(editor) => Some(editor),
        Panel::History | Panel::CloudCreation(_) => None,
    }
}
fn close(s: &mut State) {
    s.vm_lifecycle = None;
}
fn current(s: &State, form: &Editor) -> bool {
    s.inventory.as_ref().is_some_and(|inventory| {
        inventory.pool_id == form.pool
            && inventory
                .vms
                .get(&form.source.id)
                .is_some_and(|vm| vm.is_template == form.source.is_template)
    }) && (s.demo
        || form.target.is_some_and(|target| {
            s.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.observe_vm(form.source.id).ok())
                == Some(target)
        }))
}
fn open(s: &mut State, id: VmId, kind: Kind) {
    if !s.demo && !s.can_mutate() {
        return;
    }
    let Some(inventory) = s.inventory.as_ref() else {
        return;
    };
    let Some(source) = inventory.vms.get(&id).filter(|vm| !vm.is_control_domain) else {
        return;
    };
    let target = s
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.observe_vm(id).ok());
    if !s.demo && target.is_none() {
        return;
    }
    let details = inventory.details.vms.get(&id);
    let form = Editor {
        target,
        pool: inventory.pool_id,
        source: source.clone(),
        kind,
        step: 0,
        name: TextEdit::new(match kind {
            Kind::Create => format!("{} VM", source.name),
            Kind::Clone => format!("{} copy", source.name),
            _ => source.name.clone(),
        }),
        description: TextEdit::new(source.description.clone()),
        vcpus: TextEdit::new(source.vcpus.max(1).to_string()),
        memory: TextEdit::new(
            if source.memory_bytes > 0 && source.memory_bytes % (1 << 20) == 0 {
                format!("{} MiB", source.memory_bytes / (1 << 20))
            } else {
                source.memory_bytes.to_string()
            },
        ),
        disk_size: TextEdit::new(""),
        sr: None,
        network: None,
        iso: None,
        firmware: GuestFirmware::Inherited,
        bios_host: None,
        cloud: false,
        cloud_input: creation_cloud::Draft::default(),
        full_copy: false,
        as_template: source.is_template,
        disks: BTreeSet::new(),
        snapshots: BTreeSet::new(),
        snapshot_disks: BTreeSet::new(),
        initial: inventory.clone(),
        review: None,
        error: None,
    };
    if kind == Kind::Metadata
        && details.is_some_and(|details| details.is_default_template == Some(true))
    {
        return;
    }
    s.vm_lifecycle = Some(Panel::Editor(Box::new(form)));
    s.show_details = false;
    s.error = None;
    if kind == Kind::Convert {
        review(s);
    }
}
fn advertised(inventory: &Inventory, id: VmId, operation: &str) -> bool {
    inventory.details.vms.get(&id).is_some_and(|details| {
        details.current_operations.is_empty()
            && !details.blocked_operations.contains_key(operation)
            && details
                .allowed_operations
                .as_ref()
                .is_some_and(|allowed| allowed.iter().any(|allowed| allowed == operation))
    })
}
pub(super) fn controls(s: &State, id: VmId, k: &Kit) -> El {
    let Some(inventory) = &s.inventory else {
        return el();
    };
    let Some(vm) = inventory.vms.get(&id).filter(|vm| !vm.is_control_domain) else {
        return el();
    };
    let writable = s.demo || s.can_mutate();
    let halted = vm.power_state == PowerState::Halted;
    let custom = inventory
        .details
        .vms
        .get(&id)
        .is_some_and(|details| details.is_default_template == Some(false));
    let mut controls = col().w_full().gap(7.0).child(heading(
        if vm.is_template {
            "Template actions"
        } else {
            "VM lifecycle"
        },
        k,
    ));
    let mut add = |label: &str, key: &str, kind: Kind, enabled: bool| {
        controls = std::mem::replace(&mut controls, el()).child(
            Button::new(label)
                .variant(ButtonVariant::Outline)
                .disabled(!writable || !enabled)
                .on_press(move |s: &mut State| open(s, id, kind))
                .build(k)
                .key(key),
        );
    };
    if vm.is_template {
        add(
            "Create VM…",
            "create-from-template",
            Kind::Create,
            halted && (advertised(inventory, id, "clone") || advertised(inventory, id, "copy")),
        );
    }
    add(
        "Clone or full copy…",
        "clone-vm",
        Kind::Clone,
        halted && (advertised(inventory, id, "clone") || advertised(inventory, id, "copy")),
    );
    if vm.is_template {
        add(
            "Edit template name and description…",
            "edit-template",
            Kind::Metadata,
            halted && custom,
        );
    } else {
        add(
            "Convert to template…",
            "convert-vm-template",
            Kind::Convert,
            halted && advertised(inventory, id, "make_into_template"),
        );
    }
    add(
        if vm.is_template {
            "Delete template…"
        } else {
            "Delete VM…"
        },
        "delete-vm",
        Kind::Delete,
        halted && (!vm.is_template || custom) && advertised(inventory, id, "destroy"),
    );
    if !writable || !halted {
        controls = controls.child(caption("These workflows require a halted VM or template and a current writable connection.",k).wrap());
    }
    controls
}
fn validate_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 255 || name.chars().any(char::is_control) {
        Err("Enter a name of 1–255 characters without control characters.".into())
    } else {
        Ok(trimmed.into())
    }
}
fn validate_description(description: &str) -> Result<String, String> {
    if description.len() > 4096
        || description
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err("Description must be at most 4096 bytes without control characters.".into())
    } else {
        Ok(description.into())
    }
}
fn operation(form: &Editor) -> Result<VmLifecycleOperation, String> {
    Ok(match form.kind {
        Kind::Create => creation::operation(form)?,
        Kind::Clone => VmLifecycleOperation::Clone {
            name: validate_name(&form.name.text)?,
            expected_template: form.source.is_template,
            mode: if form.full_copy {
                VmCopyMode::Full {
                    sr: form.sr.ok_or("Select a destination storage repository.")?,
                }
            } else {
                VmCopyMode::Fast
            },
            as_template: form.as_template,
        },
        Kind::Convert => VmLifecycleOperation::ConvertToTemplate,
        Kind::Metadata => {
            let name = validate_name(&form.name.text)?;
            let description = validate_description(&form.description.text)?;
            if name == form.source.name && description == form.source.description {
                return Err("No template properties have changed.".into());
            }
            VmLifecycleOperation::EditTemplate { name, description }
        }
        Kind::Delete => deletion::operation(form),
    })
}
fn review(s: &mut State) {
    let Some(form) = editor(s) else { return };
    let result = (|| {
        if !current(s, form) {
            return Err(
                "The connection or source changed. Cancel and reopen this workflow.".into(),
            );
        }
        let operation = operation(form)?;
        let inventory = s.inventory.as_ref().ok_or("No current inventory.")?;
        let availability = operation.availability(inventory, form.source.id);
        if !availability.is_available() {
            return Err(availability.explanation());
        }
        let lines = match form.kind {
            Kind::Create => creation::review(form),
            Kind::Delete => deletion::review(form),
            Kind::Clone => vec![format!("Create {} “{}” using {}.",if form.as_template {"template"} else {"VM"},form.name.text.trim(),if form.full_copy {"a full disk copy"} else {"a fast clone on the existing storage"}),
                form.sr.filter(|_| form.full_copy).map(|id| format!("Destination storage: {}",storage_label(&form.initial,id))).unwrap_or_else(|| "Existing storage must support fast cloning; the host verifies this capability.".into()),
                "The result stays halted. A failed later step retains the created object and its UUID in VM changes.".into(),
                "Other changes in this pool are blocked while copying is active or its outcome is unknown. Resolve or explicitly acknowledge an unknown outcome to release this block.".into()],
            Kind::Convert => vec!["Convert this halted VM into a reusable template. It will no longer appear as a runnable VM. Its disks remain attached to the template.".into()],
            Kind::Metadata => vec![format!("Name: {} → {}",form.source.name,form.name.text.trim()),format!("Description: {}",form.description.text)],
        };
        Ok(Review { operation, lines })
    })();
    if let Some(form) = editor_mut(s) {
        match result {
            Ok(review) => {
                form.review = Some(review);
                form.error = None;
            }
            Err(error) => form.error = Some(error),
        }
    }
}
fn apply(s: &mut State) {
    let Some(form) = editor(s) else { return };
    let Some(target) = form.target else { return };
    let Some(review) = &form.review else { return };
    if !s.can_mutate()
        || !current(s, form)
        || !s.inventory.as_ref().is_some_and(|inventory| {
            review
                .operation
                .availability(inventory, form.source.id)
                .is_available()
        })
    {
        return;
    }
    let requirements = review.operation.permission_requirements();
    if session_permissions_view::assess(s, &requirements)
        .decision()
        .is_denied()
    {
        s.error = Some(session_permissions_view::assess(s, &requirements).explanation());
        return;
    }
    let command = xenadmin_core::VmCommand::Lifecycle {
        source: target,
        operation: review.operation.clone(),
    };
    let pending = (form.kind == Kind::Create && form.cloud).then(|| {
        (
            form.pool,
            form.source.id,
            target,
            review.operation.clone(),
            form.sr.unwrap(),
        )
    });
    match s.submit_tracked_command(command) {
        Ok(id) => {
            s.error = None;
            if let Some((pool, source, target, operation, sr)) = pending {
                let draft = std::mem::take(&mut editor_mut(s).unwrap().cloud_input);
                s.vm_lifecycle = Some(Panel::CloudCreation(Box::new(creation_cloud::Pending {
                    id,
                    pool,
                    source,
                    target: Some(target),
                    operation,
                    sr,
                    draft,
                })));
            } else {
                close(s);
            }
        }
        Err(e) => s.error = Some(e.to_string()),
    }
}
fn storage_label(inventory: &Inventory, id: SrId) -> String {
    inventory
        .storage
        .iter()
        .find(|sr| sr.uuid == id.uuid())
        .map_or_else(|| id.to_string(), |sr| format!("{} ({id})", sr.name))
}
fn network_label(inventory: &Inventory, id: NetworkId) -> String {
    inventory
        .networks
        .iter()
        .find(|network| network.uuid == id.uuid())
        .map_or_else(
            || id.to_string(),
            |network| format!("{} ({id})", network.name),
        )
}
pub(super) fn disk_label(inventory: &Inventory, id: VdiId) -> String {
    inventory.details.vdis.get(&id).map_or_else(
        || id.to_string(),
        |disk| {
            format!(
                "{} ({id}) • {}",
                disk.name,
                storage_bytes(disk.virtual_size)
            )
        },
    )
}
fn choices<T: Copy + std::fmt::Display + 'static>(
    label: &str,
    prefix: &str,
    items: Vec<(T, String, bool)>,
    select: fn(&mut Editor, T),
    k: &Kit,
) -> El {
    let mut content = col()
        .w_full()
        .gap(6.0)
        .key(format!("{prefix}-choices"))
        .child(heading(label, k));
    if items.is_empty() {
        return content.child(
            caption(
                "No eligible choices are reported in the current inventory.",
                k,
            )
            .wrap(),
        );
    }
    let mut list = col()
        .w_full()
        .gap(5.0)
        .max_h(150.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    for (id, label, selected) in items {
        list = list.child(
            Button::new(if selected {
                format!("✓ {label}")
            } else {
                label
            })
            .variant(ButtonVariant::Outline)
            .on_press(move |s: &mut State| {
                if let Some(form) = editor_mut(s) {
                    select(form, id);
                    form.error = None;
                }
            })
            .build(k)
            .key(format!("{prefix}-{id}")),
        );
    }
    content = content.child(list);
    content
}
fn storage_choices(form: &Editor, k: &Kit) -> El {
    let mut items: Vec<_> = form
        .initial
        .details
        .storage
        .values()
        .filter(|sr| {
            sr.content_type != "iso"
                && sr
                    .allowed_operations
                    .as_ref()
                    .is_some_and(|ops| ops.iter().any(|op| op == "vdi_create"))
        })
        .map(|sr| {
            (
                sr.id,
                storage_label(&form.initial, sr.id),
                form.sr == Some(sr.id),
            )
        })
        .collect();
    items.sort_by_cached_key(|(_, name, _)| name.to_lowercase());
    choices(
        "Destination storage repository",
        "vm-lifecycle-sr",
        items,
        |form, id| form.sr = Some(id),
        k,
    )
}
fn names(form: &Editor, description: bool, k: &Kit) -> El {
    let mut body = col().w_full().gap(10.0).child(
        TextInput::new(&form.name)
            .label("Name")
            .on_edit(|s: &mut State| &mut editor_mut(s).expect("lifecycle editor open").name)
            .build(k)
            .key("vm-lifecycle-name"),
    );
    if description {
        body = body.child(
            TextInput::new(&form.description)
                .label("Description")
                .on_edit(|s: &mut State| {
                    &mut editor_mut(s).expect("lifecycle editor open").description
                })
                .build(k)
                .key("vm-lifecycle-description"),
        );
    }
    body
}
fn edit(form: &Editor, k: &Kit) -> El {
    match form.kind {
        Kind::Create => creation::editor(form, k),
        Kind::Delete => deletion::editor(form, k),
        Kind::Metadata => names(form, true, k),
        Kind::Convert => el(),
        Kind::Clone => {
            let mut body=names(form,false,k).child(Checkbox::new(form.full_copy).label("Full copy to another storage repository")
                .on_toggle(|s:&mut State| if let Some(form)=editor_mut(s) {form.full_copy = !form.full_copy;}).build(k).key("vm-lifecycle-full-copy"))
                .child(caption("Fast clone uses the source storage. Full copy creates independent disk copies on the selected repository.",k).wrap())
                .child(Checkbox::new(form.as_template).label("Create as a template")
                    .on_toggle(|s:&mut State| if let Some(form)=editor_mut(s) {form.as_template = !form.as_template;}).build(k).key("vm-lifecycle-as-template"));
            if form.full_copy {
                body = body.child(storage_choices(form, k));
            }
            body
        }
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    if matches!(s.vm_lifecycle, Some(Panel::History)) {
        return history(s, k);
    }
    if let Some(Panel::CloudCreation(p)) = &s.vm_lifecycle {
        return creation_cloud::pending_view(s, p, k);
    }
    let Some(form) = editor(s) else { return el() };
    let title = if form.review.is_some() {
        "Review VM change"
    } else {
        match form.kind {
            Kind::Create => "Create VM from template",
            Kind::Clone => "Clone or full copy",
            Kind::Convert => "Convert to template",
            Kind::Metadata => "Edit template",
            Kind::Delete => {
                if form.source.is_template {
                    "Delete template"
                } else {
                    "Delete VM"
                }
            }
        }
    };
    let mut body = col().w_full().gap(12.0).key("vm-lifecycle-body").child(
        caption(
            format!(
                "Source: {}\nUUID: {}\nPool: {}",
                form.source.name, form.source.id, form.pool
            ),
            k,
        )
        .wrap_anywhere(),
    );
    if let Some(review) = &form.review {
        for line in &review.lines {
            body = body.child(caption(line, k).wrap_anywhere());
        }
    } else {
        body = body.child(edit(form, k));
    }
    let current = current(s, form);
    let available = form.review.as_ref().and_then(|review| {
        s.inventory
            .as_ref()
            .map(|inventory| review.operation.availability(inventory, form.source.id))
    });
    if !current {
        body = body.child(
            caption(
                "The connection or source changed. Cancel and reopen this workflow.",
                k,
            )
            .wrap()
            .role("status"),
        );
    }
    if let Some(status) = available.as_ref().filter(|status| !status.is_available()) {
        body = body.child(
            caption(
                format!(
                    "This change is no longer available: {}",
                    status.explanation()
                ),
                k,
            )
            .wrap()
            .role("status"),
        );
    }
    if s.demo {
        body = body.child(
            caption(
                "Synthetic review • Apply is disabled; no host changes can be submitted.",
                k,
            )
            .wrap(),
        );
    }
    if let Some(error) = form.error.as_ref().or(s.error.as_ref()) {
        body = body.child(
            caption(error, k)
                .wrap()
                .color(k.c.danger.icon)
                .role("alert"),
        );
    }
    let permission = form.review.as_ref().map(|review| {
        session_permissions_view::assess(s, &review.operation.permission_requirements())
    });
    let mut buttons = row().w_full().gap(8.0).flex_wrap().justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(close)
            .build(k)
            .key("cancel-vm-lifecycle"),
    );
    if form.review.is_some() {
        if form.kind != Kind::Convert {
            buttons = buttons.child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(form) = editor_mut(s) {
                            form.review = None;
                        }
                    })
                    .build(k)
                    .key("back-vm-lifecycle"),
            );
        }
        buttons = buttons.child(
            Button::new(if form.kind == Kind::Delete {
                "Delete reviewed resources"
            } else {
                "Apply change"
            })
            .variant(if form.kind == Kind::Delete {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Outline
            })
            .disabled(
                !s.can_mutate()
                    || !current
                    || permission
                        .as_ref()
                        .is_some_and(|p| p.decision().is_denied())
                    || !available.is_some_and(|status| status.is_available()),
            )
            .on_press(apply)
            .build(k)
            .key("apply-vm-lifecycle"),
        );
    } else {
        if form.kind == Kind::Create && form.step > 0 {
            buttons = buttons.child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(form) = editor_mut(s) {
                            form.step -= 1;
                            form.error = None;
                        }
                    })
                    .build(k)
                    .key("back-vm-lifecycle"),
            );
        }
        buttons = buttons.child(
            Button::new(
                if form.kind == Kind::Create && form.step < creation_cloud::last_step(form) {
                    "Next"
                } else {
                    "Review change"
                },
            )
            .variant(ButtonVariant::Outline)
            .disabled(!current || (!s.demo && !s.can_mutate()))
            .on_press(|s: &mut State| {
                if editor(s).is_some_and(|form| {
                    form.kind == Kind::Create && form.step < creation_cloud::last_step(form)
                }) {
                    creation::next(s);
                } else {
                    review(s);
                }
            })
            .build(k)
            .key("review-vm-lifecycle"),
        );
    }
    let card = col()
        .w((s.window_width - 40.0).clamp(280.0, 700.0))
        .h((s.window_height - 60.0).clamp(280.0, 800.0))
        .min_h(0.0)
        .padding(22.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(title, k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(session_permissions_view::footer(
            buttons,
            permission.as_ref(),
            (s.window_width - 84.0).clamp(236.0, 656.0),
            k,
        ));
    overlay(card, title)
}
fn overlay(body: El, title: &str) -> El {
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label(title)
        .modal(false, close)
        .key("vm-lifecycle-dialog")
        .child(body)
}
pub(super) fn history_button(s: &State, k: &Kit) -> El {
    if !s.operation_rows.iter().any(|record| {
        matches!(
            record.action,
            xenadmin_core::OperationAction::Lifecycle(_)
                | xenadmin_core::OperationAction::Migration(_)
                | xenadmin_core::OperationAction::Recovery(_)
        )
    }) {
        return el();
    }
    Button::new("VM changes")
        .variant(ButtonVariant::Ghost)
        .on_press(|s: &mut State| s.vm_lifecycle = Some(Panel::History))
        .build(k)
        .key("vm-lifecycle-history")
}
fn history(s: &State, k: &Kit) -> El {
    let mut body=col().w((s.window_width-40.0).clamp(280.0,700.0)).max_h((s.window_height-60.0).clamp(280.0,800.0)).padding(22.0).gap(15.0).radius(10.0).fill(k.c.bg_base).scroll(rune::ScrollSpec{stick_bottom:false})
        .child(heading("VM changes",k)).child(caption("Completed steps and created resources remain recorded after partial failure. Rechecking reads the host without repeating writes or continuing unfinished steps. Other changes in a pool wait while creation or copying is active or its outcome is unknown; resolve or explicitly acknowledge that outcome to release the block.",k).wrap());
    for record in s.operation_rows.iter().rev().filter(|record| {
        matches!(
            record.action,
            xenadmin_core::OperationAction::Lifecycle(_)
                | xenadmin_core::OperationAction::Migration(_)
                | xenadmin_core::OperationAction::Recovery(_)
        )
    }) {
        let can_control = s.can_mutate()
            && s.inventory
                .as_ref()
                .is_some_and(|inventory| inventory.pool_id == record.pool);
        let mut item = col()
            .w_full()
            .gap(6.0)
            .child(heading(
                format!("{} • {:?}", record.action.label(), record.state),
                k,
            ))
            .child(
                caption(
                    format!(
                        "Source VM/template UUID: {}\nPool: {}\n{}",
                        record.vm, record.pool, record.detail
                    ),
                    k,
                )
                .wrap_anywhere(),
            );
        if let Some(id) = record.created_vm {
            item =
                item.child(caption(format!("Created VM/template UUID: {id}"), k).wrap_anywhere());
        }
        for resource in &record.created_devices {
            item = item.child(
                caption(
                    format!("Created {} UUID: {}", resource.label(), resource.uuid()),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        for step in &record.steps {
            item = item.child(
                caption(
                    format!("{} • {:?}\n{}", step.label, step.state, step.detail),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        if record.state == xenadmin_core::OperationState::OutcomeUnknown && record.can_recheck() {
            let id = record.id;
            item = item.child(
                Button::new("Recheck outcome")
                    .variant(ButtonVariant::Outline)
                    .disabled(!can_control)
                    .on_press(move |s: &mut State| {
                        s.control_operation(id, xenadmin_core::OperationControl::Recheck)
                    })
                    .build(k),
            );
            let source = record.vm.to_string();
            item = item.child(
                Button::new("Review unknown outcome acknowledgment…")
                    .variant(ButtonVariant::Ghost)
                    .disabled(!can_control)
                    .on_press(move |s: &mut State| {
                        s.vm_lifecycle = None;
                        s.unknown_review = Some((id, source.clone()));
                    })
                    .build(k),
            );
        }
        if let Some(parent) = record.batch_parent {
            item = item.child(
                caption(
                    format!("Managed by batch {parent}; use pool history for controls."),
                    k,
                )
                .wrap_anywhere(),
            );
        }
        if record.batch_parent.is_none() && record.can_cancel && !record.state.terminal() {
            let id = record.id;
            item = item.child(
                Button::new("Stop remaining steps")
                    .variant(ButtonVariant::Outline)
                    .disabled(!can_control)
                    .on_press(move |s: &mut State| {
                        s.control_operation(
                            id,
                            xenadmin_core::OperationControl::RequestCancellation,
                        )
                    })
                    .build(k),
            );
        }
        body = body.child(item);
    }
    overlay(
        body.child(
            Button::new("Close")
                .variant(ButtonVariant::Outline)
                .on_press(close)
                .build(k)
                .key("close-vm-lifecycle-history"),
        ),
        "VM changes",
    )
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let inventory = Arc::make_mut(s.inventory.as_mut().expect("synthetic inventory"));
    let source = inventory.vms.get_mut(&id).expect("synthetic source");
    source.is_template = true;
    source.name = "Application template".into();
    let details = inventory
        .details
        .vms
        .get_mut(&id)
        .expect("synthetic details");
    details.is_default_template = Some(false);
    details.allowed_operations = Some(vec!["clone".into(), "copy".into(), "destroy".into()]);
    s.page = Page::Templates;
    s.filter_rows();
    s.table.select(id.to_string());
    open(s, id, Kind::Create);
    if let Some(form) = editor_mut(s) {
        form.step = 2;
        form.name = TextEdit::new("New application VM");
        form.sr = form.initial.details.storage.keys().next().copied();
        form.network = form.initial.details.networks.keys().next().copied();
        form.disk_size = TextEdit::new("16 GiB");
    }
}

pub(super) fn review_scene(s: &mut State) {
    scene(s);
    review(s);
}

#[cfg(test)]
#[path = "vm_lifecycle_tests.rs"]
mod tests;

pub(super) fn firmware_scene(s: &mut State) {
    scene(s);
    if let Some(form) = editor_mut(s) {
        form.step = 1;
        let inventory = Arc::make_mut(&mut form.initial);
        let details = inventory.details.vms.get_mut(&form.source.id).unwrap();
        details.firmware = Some("bios".into());
        details.bios_identity_empty = Some(true);
    }
}

pub(super) fn cloud_scene(s: &mut State) {
    creation_cloud::scene(s);
}
pub(super) fn cloud_pending_scene(s: &mut State) {
    creation_cloud::pending_scene(s);
}

pub(super) fn deletion_review(
    inventory: &Inventory,
    source: &Vm,
    disks: &BTreeSet<VdiId>,
    snapshots: &BTreeSet<SnapshotId>,
    snapshot_disks: &BTreeSet<VdiId>,
) -> Vec<String> {
    deletion::describe(inventory, source, disks, snapshots, snapshot_disks)
}
