//! What is selected: the tree scope, the grid selection inside a tab, and
//! the active tab.
//!
//! K2 extends this with the Back and Forward history of `(scope, tab)`
//! pairs, the last active tab per object kind, and the re-selection of the
//! parent when the selected object disappears.

use crate::object::Target;
use crate::tabs::TabId;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// The tree selection. It sets the scope: which object the tab strip is
    /// about.
    pub scope: Target,
    /// The grid selection inside the active tab. It sets the target of the
    /// tab's commands and does not change the scope. Empty when no row is
    /// selected.
    pub grid: Target,
    /// The tab the user chose. `None`, or a tab the scope does not have,
    /// means the first tab of the scope.
    pub tab: Option<TabId>,
}

impl Selection {
    /// What a command acts on: the grid selection when there is one,
    /// otherwise the scope.
    pub fn target(&self) -> &Target {
        if self.grid.is_empty() {
            &self.scope
        } else {
            &self.grid
        }
    }
}
