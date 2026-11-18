//! The commands of the area: what the buttons of the Rune details-pane
//! section "VM lifecycle" / "Template actions" and "Cloud-init drive…"
//! became. Labels and menu places are the plan's (Menus table).
//!
//! Every command is for one object and `Hidden` for several, and `Hidden`
//! for an object its change does not apply to (Rune showed no button): a
//! VM command for a template and the reverse, a control domain, an object
//! its pool's inventory does not have. It is `Disabled` where Rune
//! disabled the button, with the reason [`edit::capture`] gives.
//!
//! New VM is the exception: the client has no template picker, so `vm.new`
//! (VM menu, toolbar) and `templates.new` stay visible and are `Disabled`
//! with [`edit::NO_TEMPLATE`] until one template is the target.

use super::{drive, edit};
use crate::prelude::*;

const AREA: &str = "vm_lifecycle";

pub const NEW: CommandId = CommandId("vm.new");
pub const TEMPLATES_NEW: CommandId = CommandId("templates.new");
pub const CREATE_FROM_SELECTION: CommandId = CommandId("templates.create-vm");
pub const COPY: CommandId = CommandId("vm.copy");
pub const TEMPLATES_COPY: CommandId = CommandId("templates.copy");
pub const CONVERT: CommandId = CommandId("vm.convert-to-template");
pub const CLOUD_INIT: CommandId = CommandId("vm.cloud-init-drive");
pub const DELETE: CommandId = CommandId("vm.delete");
pub const TEMPLATES_DELETE: CommandId = CommandId("templates.delete");

/// In menu order.
pub fn commands() -> Vec<CommandSpec> {
    const VM: &[ObjectKind] = &[ObjectKind::Vm];
    const TEMPLATE: &[ObjectKind] = &[ObjectKind::Template];
    vec![
        CommandSpec::new(NEW.0, "New VM", AREA)
            .menu(place::VM_NEW)
            .toolbar(ToolbarSlot::NEW_VM)
            .eligibility(new_vm)
            .invoke(create),
        CommandSpec::new(COPY.0, "Copy VM", AREA)
            .menu(place::VM_COPY)
            .context(VM)
            .eligibility(|m, target| change(m, target, edit::Kind::Clone, false))
            .invoke(copy),
        CommandSpec::new(CONVERT.0, "Convert to Template", AREA)
            .menu(place::VM_CONVERT_TO_TEMPLATE)
            .context(VM)
            .eligibility(|m, target| change(m, target, edit::Kind::Convert, false))
            .invoke(|m, target| open(m, target, edit::Kind::Convert)),
        CommandSpec::new(CLOUD_INIT.0, "Cloud-init Drive", AREA)
            .menu(MenuPath::new(Menu::Vm, 6, 30))
            .context(VM)
            .local(TabId::Storage, 60)
            .eligibility(cloud_init)
            .dynamic_label(cloud_init_label)
            .invoke(|m, target| {
                if let Some(object) = target.single() {
                    drive::open(m, object);
                }
            }),
        CommandSpec::new(DELETE.0, "Delete VM", AREA)
            .menu(place::VM_DELETE)
            .context(VM)
            .eligibility(|m, target| change(m, target, edit::Kind::Delete, false))
            .invoke(delete),
        CommandSpec::new(TEMPLATES_NEW.0, "New VM wizard", AREA)
            .menu(place::TEMPLATES_NEW_VM)
            .eligibility(new_vm)
            .invoke(create),
        CommandSpec::new(CREATE_FROM_SELECTION.0, "Create VM From Selection", AREA)
            .menu(place::TEMPLATES_CREATE_FROM_SELECTION)
            .context(TEMPLATE)
            .eligibility(|m, target| change(m, target, edit::Kind::Create, true))
            .invoke(create),
        CommandSpec::new(TEMPLATES_COPY.0, "Copy", AREA)
            .menu(place::TEMPLATES_COPY)
            .context(TEMPLATE)
            .eligibility(|m, target| change(m, target, edit::Kind::Clone, true))
            .invoke(copy),
        CommandSpec::new(TEMPLATES_DELETE.0, "Delete Template", AREA)
            .menu(place::TEMPLATES_DELETE)
            .context(TEMPLATE)
            .eligibility(|m, target| change(m, target, edit::Kind::Delete, true))
            .invoke(delete),
    ]
}

/// The one object of a target when the inventory of its pool has it as a
/// template (`template`) or as a VM that is none.
fn source<'a>(m: &Model, target: &'a Target, template: bool) -> Option<&'a ObjectRef> {
    let object = target.single()?;
    let inventory = m.inventory_for(object.pool?)?;
    let vm = inventory.vms.get(&object.vm_id()?)?;
    (vm.is_template == template).then_some(object)
}

/// A change of one VM (`template` false) or one template.
fn change(m: &Model, target: &Target, kind: edit::Kind, template: bool) -> Eligibility {
    match source(m, target, template).map(|object| edit::capture(m, object, kind)) {
        Some(Ok(_)) => Eligibility::Enabled,
        Some(Err(eligibility)) => eligibility,
        None => Eligibility::Hidden,
    }
}

/// New VM: visible whatever the target, enabled for one template a VM can
/// be created from.
fn new_vm(m: &Model, target: &Target) -> Eligibility {
    match change(m, target, edit::Kind::Create, true) {
        Eligibility::Hidden => Eligibility::disabled(edit::NO_TEMPLATE),
        other => other,
    }
}

fn open(m: &mut Model, target: &Target, kind: edit::Kind) {
    if let Some(object) = target.single() {
        edit::open(m, object, kind);
    }
}

fn create(m: &mut Model, target: &Target) {
    open(m, target, edit::Kind::Create);
}

fn copy(m: &mut Model, target: &Target) {
    open(m, target, edit::Kind::Clone);
}

fn delete(m: &mut Model, target: &Target) {
    open(m, target, edit::Kind::Delete);
}

/// The cloud-init drive: hidden for anything but one ordinary VM, also on
/// the Storage tab of a host, a pool, a template or a repository.
fn cloud_init(m: &Model, target: &Target) -> Eligibility {
    match target.single().map(|object| drive::capture(m, object)) {
        Some(Ok(_)) => Eligibility::Enabled,
        Some(Err(eligibility)) => eligibility,
        None => Eligibility::Hidden,
    }
}

/// The label Rune gave its button for a VM that is not halted.
fn cloud_init_label(m: &Model, target: &Target) -> Option<String> {
    matches!(cloud_init(m, target), Eligibility::Disabled(_))
        .then(|| "Cloud-init Drive (shut down first)".to_owned())
}
