//! The command table the menus, toolbar, and context menus will share.
//!
//! The Rune front end still builds its own buttons. This table is the list
//! those buttons are moving toward. Eligibility here is by object kind.

use crate::intent::{DialogId, Intent, Surface, WizardId};
use crate::selection::{ObjectKind, Selection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eligibility {
    Enabled,
    Disabled(&'static str),
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub id: CommandId,
    pub label: &'static str,
    pub menu: &'static str,
    pub kinds: &'static [ObjectKind],
    pub intent: Intent,
}

const ANY: &[ObjectKind] = &[];
const VM: &[ObjectKind] = &[ObjectKind::Vm];
const HOST: &[ObjectKind] = &[ObjectKind::Host];
const POOL: &[ObjectKind] = &[ObjectKind::Pool];
const STORAGE: &[ObjectKind] = &[ObjectKind::StorageRepository];
const TEMPLATE: &[ObjectKind] = &[ObjectKind::Template];

pub const COMMANDS: &[Command] = &[
    command(
        "file.import",
        "Import",
        "File",
        ANY,
        Intent::OpenSurface(Surface::Wizard(WizardId::Import)),
    ),
    command("file.exit", "Exit", "File", ANY, Intent::None),
    command("pool.new", "New Pool", "Pool", ANY, Intent::None),
    command(
        "pool.add-server",
        "Add Server",
        "Pool",
        ANY,
        Intent::OpenSurface(Surface::Dialog(DialogId::AddServer)),
    ),
    command("pool.disconnect", "Disconnect", "Pool", POOL, Intent::None),
    command(
        "pool.properties",
        "Properties",
        "Pool",
        POOL,
        Intent::OpenSurface(Surface::Properties(crate::intent::PageId::General)),
    ),
    command(
        "server.reboot",
        "Reboot",
        "Server",
        HOST,
        Intent::OpenSurface(Surface::Dialog(DialogId::Maintenance)),
    ),
    command(
        "server.maintenance",
        "Enter or Exit Maintenance Mode",
        "Server",
        HOST,
        Intent::OpenSurface(Surface::Dialog(DialogId::Maintenance)),
    ),
    command(
        "vm.new",
        "New VM",
        "VM",
        ANY,
        Intent::OpenSurface(Surface::Wizard(WizardId::NewVm)),
    ),
    command(
        "vm.start",
        "Start",
        "VM",
        VM,
        Intent::OpenSurface(Surface::Dialog(DialogId::Power)),
    ),
    command(
        "vm.shutdown",
        "Shut Down",
        "VM",
        VM,
        Intent::OpenSurface(Surface::Dialog(DialogId::Power)),
    ),
    command(
        "vm.migrate",
        "Migrate to Server",
        "VM",
        VM,
        Intent::OpenSurface(Surface::Wizard(WizardId::Migrate)),
    ),
    command(
        "vm.snapshot",
        "Take a Snapshot",
        "VM",
        VM,
        Intent::OpenSurface(Surface::Dialog(DialogId::Snapshot)),
    ),
    command(
        "vm.delete",
        "Delete VM",
        "VM",
        VM,
        Intent::OpenSurface(Surface::Dialog(DialogId::DeleteVm)),
    ),
    command(
        "storage.new",
        "New Storage",
        "Storage",
        ANY,
        Intent::OpenSurface(Surface::Wizard(WizardId::NewStorage)),
    ),
    command("storage.repair", "Repair", "Storage", STORAGE, Intent::None),
    command(
        "templates.new",
        "New VM wizard",
        "Templates",
        TEMPLATE,
        Intent::OpenSurface(Surface::Wizard(WizardId::NewVm)),
    ),
    command(
        "tools.options",
        "Options",
        "Tools",
        ANY,
        Intent::OpenSurface(Surface::Dialog(DialogId::Options)),
    ),
    command(
        "help.about",
        "About",
        "Help",
        ANY,
        Intent::OpenSurface(Surface::Dialog(DialogId::Help)),
    ),
];

const fn command(
    id: &'static str,
    label: &'static str,
    menu: &'static str,
    kinds: &'static [ObjectKind],
    intent: Intent,
) -> Command {
    Command {
        id: CommandId(id),
        label,
        menu,
        kinds,
        intent,
    }
}

pub fn eligibility(id: CommandId, selection: Selection) -> Eligibility {
    let Some(command) = COMMANDS.iter().find(|command| command.id == id) else {
        return Eligibility::Hidden;
    };
    if command.kinds.is_empty() || command.kinds.contains(&selection.kind) {
        Eligibility::Enabled
    } else {
        Eligibility::Disabled("Not available for this selection")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_unique_and_power_follows_the_selection() {
        let mut ids: Vec<_> = COMMANDS.iter().map(|command| command.id.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len());
        let vm = Selection {
            kind: ObjectKind::Vm,
            host_live: false,
        };
        let pool = Selection {
            kind: ObjectKind::Pool,
            host_live: false,
        };
        assert_eq!(eligibility(CommandId("vm.start"), vm), Eligibility::Enabled);
        assert_eq!(
            eligibility(CommandId("vm.start"), pool),
            Eligibility::Disabled("Not available for this selection")
        );
        assert_eq!(
            eligibility(CommandId("file.exit"), pool),
            Eligibility::Enabled
        );
        assert_eq!(eligibility(CommandId("missing"), vm), Eligibility::Hidden);
    }
}
