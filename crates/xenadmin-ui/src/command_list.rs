//! The command registry as text: what `xenadmin-scenes --commands` prints
//! and `scripts/check.sh` writes to `artifacts/ui/commands.txt`. The
//! Commands table of the migration ledger is made from that file
//! (`scripts/gpui_commands_table.py`).
//!
//! One line per registered command, in registry order (the kernel's, then
//! each area's in `AREAS` order), eight fields separated by a tab:
//!
//! | # | Field | Form | None |
//! | --- | --- | --- | --- |
//! | 1 | id | `server.maintenance` | |
//! | 2 | label | the static label; a dynamic label is not resolved | |
//! | 3 | menu path | `Server [3.10]`, `View > Toolbars [2.10]`: menu, submenu, group.order | `-` |
//! | 4 | toolbar slot | `Power.20`: group.order | `-` |
//! | 5 | context kinds | `Server, Pool`: the kinds whose context menu shows it | `-` |
//! | 6 | local row | `Home.10`: tab.order | `-` |
//! | 7 | shortcut | `ctrl-shift-m` (GPUI syntax) | `-` |
//! | 8 | area | `host_admin`, `kernel` | |

use crate::command::{self, CommandSpec};

/// What stands for a field a command does not have.
pub const NONE: &str = "-";

/// The eight fields of one command.
pub fn fields(spec: &CommandSpec) -> [String; 8] {
    let none = || NONE.to_owned();
    let menu = spec.menu.map_or_else(none, |path| {
        let place = match path.submenu {
            Some(submenu) => format!("{} > {submenu}", path.menu.title()),
            None => path.menu.title().to_owned(),
        };
        format!("{place} [{}.{}]", path.group, path.order)
    });
    let toolbar = spec
        .toolbar
        .map_or_else(none, |slot| format!("{:?}.{}", slot.group, slot.order));
    let context = if spec.context.is_empty() {
        none()
    } else {
        let kinds: Vec<&str> = spec.context.iter().map(|kind| kind.label()).collect();
        kinds.join(", ")
    };
    let rows: Vec<String> = spec
        .local_rows()
        .map(|row| format!("{}.{}", row.tab.title(), row.order))
        .collect();
    let local = if rows.is_empty() {
        none()
    } else {
        rows.join(", ")
    };
    [
        spec.id.0.to_owned(),
        spec.label.to_owned(),
        menu,
        toolbar,
        context,
        local,
        spec.shortcut.map_or_else(none, str::to_owned),
        spec.area.to_owned(),
    ]
}

/// One line per registered command, each ended by a newline.
pub fn listing() -> String {
    command::registry()
        .iter()
        .map(|spec| fields(spec).join("\t") + "\n")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_has_one_line_of_eight_fields_for_every_registered_command() {
        let listing = listing();
        let lines: Vec<Vec<&str>> = listing
            .lines()
            .map(|line| line.split('\t').collect())
            .collect();
        assert_eq!(lines.len(), command::registry().len());
        for (line, spec) in lines.iter().zip(command::registry()) {
            assert_eq!(line.len(), 8, "{line:?}");
            assert_eq!(line[0], spec.id.0);
            assert_eq!(line[1], spec.label);
            assert_eq!(line[7], spec.area);
            assert!(line.iter().all(|field| !field.is_empty()), "{line:?}");
        }
        let line = |id: &str| {
            lines
                .iter()
                .find(|line| line[0] == id)
                .unwrap_or_else(|| panic!("no line for {id}"))
                .clone()
        };
        // The kernel's Back: a toolbar button with a shortcut, in no menu,
        // no context menu and no local row.
        assert_eq!(
            line("nav.back"),
            [
                "nav.back",
                "Back",
                NONE,
                "Navigation.10",
                NONE,
                NONE,
                "alt-left",
                "kernel"
            ]
        );
        // An area's command with a context menu.
        let maintenance = line("server.maintenance");
        assert!(maintenance[2].starts_with("Server ["), "{maintenance:?}");
        assert_eq!(maintenance[4], "Server");
        assert_eq!(maintenance[7], "host_admin");
        // The harness's probe: a local row.
        assert_eq!(line("probe.open")[5], "Home.10");
    }
}
