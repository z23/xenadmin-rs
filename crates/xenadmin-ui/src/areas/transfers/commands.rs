//! The commands of the area: what the Rune buttons "Archives" and "Convert
//! local disk…" of the top control row, "Export…" of the details pane and
//! "Export disk image" of a disk became.
//!
//! - `file.import`: `Disabled` while no pool is shown (the Rune dialog
//!   opened and could do nothing); the pool is the target's, else the scope
//!   session's.
//! - `vm.export`, `templates.export`: one VM or one template. `Hidden` for
//!   a selection of several (the `bulk_export` area's `vm.export-selection`
//!   and `templates.export-selection`), for another kind and
//!   for a control domain (Rune showed no button). `Disabled` with what the
//!   core says where Rune disabled the button; a read-only connection may
//!   export.
//! - `disk.export-image`: one virtual disk. `Hidden` when it has no
//!   repository or size, as the Rune button.
//! - `tools.convert-disk-image`: always; it needs no connection.

use xenadmin_core::VdiId;

use super::{conversion, export_disk, export_disk_eligibility, export_eligibility, panel};
use crate::prelude::*;

const AREA: &str = "transfers";

pub const IMPORT: CommandId = CommandId("file.import");
pub const EXPORT: CommandId = CommandId("vm.export");
pub const EXPORT_TEMPLATE: CommandId = CommandId("templates.export");
pub const EXPORT_DISK: CommandId = CommandId("disk.export-image");
pub const CONVERT: CommandId = CommandId("tools.convert-disk-image");

/// In menu order. Labels are the plan's (Menus table); "Export Disk
/// Image" has no menu item there and is a context and local-row command.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(IMPORT.0, "Import", AREA)
            .menu(place::FILE_IMPORT)
            .eligibility(|m, target| {
                Eligibility::enabled_if(import_pool(m, target).is_some(), panel::NO_POOL)
            })
            .invoke(|m, target| {
                let pool = import_pool(m, target);
                panel::begin(m, pool, None);
            }),
        CommandSpec::new(EXPORT.0, "Export", AREA)
            .menu(place::VM_EXPORT)
            .context(&[ObjectKind::Vm])
            .eligibility(|m, target| export(m, target, ObjectKind::Vm))
            .invoke(open_export),
        CommandSpec::new(EXPORT_TEMPLATE.0, "Export to File", AREA)
            .menu(place::TEMPLATES_EXPORT)
            .context(&[ObjectKind::Template])
            .eligibility(|m, target| export(m, target, ObjectKind::Template))
            .invoke(open_export),
        CommandSpec::new(EXPORT_DISK.0, "Export Disk Image", AREA)
            .context(&[ObjectKind::VirtualDisk])
            .local(TabId::Storage, 80)
            .eligibility(|m, target| match disk(target) {
                Some((pool, vdi)) => export_disk_eligibility(m, pool, vdi),
                None => Eligibility::Hidden,
            })
            .invoke(|m, target| {
                if let Some((pool, vdi)) = disk(target) {
                    export_disk(m, pool, vdi);
                }
            }),
        CommandSpec::new(CONVERT.0, "Convert Disk Image", AREA)
            .menu(place::TOOLS_CONVERT_DISK_IMAGE)
            .invoke(|m, _| conversion::open(m)),
    ]
}

/// The pool an import goes to: the target's, else the scope session's.
fn import_pool(m: &Model, target: &Target) -> Option<xenadmin_core::PoolId> {
    target
        .pool()
        .filter(|pool| m.inventory_for(*pool).is_some())
        .or_else(|| m.inventory.as_ref().map(|i| i.pool_id))
}

fn export(m: &Model, target: &Target, kind: ObjectKind) -> Eligibility {
    match target
        .single_of(kind)
        .and_then(|object| super::vm_source(m, object))
    {
        Some((pool, source)) => export_eligibility(m, pool, &source),
        None => Eligibility::Hidden,
    }
}

fn open_export(m: &mut Model, target: &Target) {
    if let Some((pool, source)) = target
        .single()
        .and_then(|object| super::vm_source(m, object))
    {
        super::export(m, pool, source);
    }
}

fn disk(target: &Target) -> Option<(xenadmin_core::PoolId, VdiId)> {
    let object = target.single_of(ObjectKind::VirtualDisk)?;
    Some((object.pool?, VdiId::from(object.uuid()?)))
}
