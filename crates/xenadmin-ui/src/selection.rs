//! What is selected: the tree scope, the grid selection inside a tab, the
//! active tab, the Back and Forward history, the last tab per object kind,
//! and the context menu that is open.
//!
//! The rules (plan, "Selection and scope"):
//!
//! - The tree selection sets the scope. A grid selection inside a tab sets
//!   the action target and does not change the scope.
//! - Selection is by [`ObjectRef`] (pool, kind, id) and survives inventory
//!   updates. When the object disappears its parent is selected and a status
//!   notice says so ([`crate::Model::reconcile_selection`]).
//! - Back and Forward step through `(scope, tab)` pairs.
//! - The last active tab is remembered per object kind.
//! - A multi-selection shows only the Search tab.
//!
//! The methods that change the selection are on [`crate::Model`]: `select`,
//! `select_in_grid`, `set_active_tab`, `back`, `forward`,
//! `open_context_menu`, `invoke_context`.

use std::collections::BTreeMap;

use xenadmin_core::{ObservedResource, PoolId};

use crate::object::{ObjectKind, ObjectRef, Target};
use crate::tabs::TabId;

#[derive(Clone, Debug, Default)]
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
    /// The context menu that is open, with what it was opened for.
    pub menu: Option<MenuContext>,
    /// What is remembered of the scope object while it exists, so that its
    /// parent can be selected and it can be named when it disappears.
    pub(crate) anchor: Option<Anchor>,
    pub(crate) history: History,
    pub(crate) last_tab: BTreeMap<ObjectKind, TabId>,
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

    /// The tab last chosen while an object of this kind was selected.
    pub fn last_tab(&self, kind: ObjectKind) -> Option<TabId> {
        self.last_tab.get(&kind).copied()
    }
}

/// The name and the parent of the scope object as last seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub(crate) object: ObjectRef,
    pub(crate) name: String,
    pub(crate) parent: Option<ObjectRef>,
}

/// One place Back and Forward return to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub scope: Target,
    pub tab: TabId,
}

/// The Back and Forward stacks. The current place is not in either.
#[derive(Clone, Debug, Default)]
pub struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
}

impl History {
    /// How many places each stack keeps.
    pub const LIMIT: usize = 100;

    /// The user went somewhere new from `from`: it becomes the place Back
    /// returns to, and there is nothing to go forward to any more.
    pub(crate) fn visit(&mut self, from: Place) {
        if self.back.last() != Some(&from) {
            self.back.push(from);
            if self.back.len() > Self::LIMIT {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Steps back from `from`. Places for which `usable` is false (their
    /// object is gone) are dropped on the way.
    pub(crate) fn back(&mut self, from: Place, usable: impl Fn(&Place) -> bool) -> Option<Place> {
        Self::step(&mut self.back, &mut self.forward, from, usable)
    }

    pub(crate) fn forward(
        &mut self,
        from: Place,
        usable: impl Fn(&Place) -> bool,
    ) -> Option<Place> {
        Self::step(&mut self.forward, &mut self.back, from, usable)
    }

    fn step(
        take: &mut Vec<Place>,
        give: &mut Vec<Place>,
        from: Place,
        usable: impl Fn(&Place) -> bool,
    ) -> Option<Place> {
        while let Some(place) = take.pop() {
            if place == from || !usable(&place) {
                continue;
            }
            give.push(from);
            if give.len() > Self::LIMIT {
                give.remove(0);
            }
            return Some(place);
        }
        None
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// Where a context menu was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOrigin {
    /// On a row of the navigation tree.
    Tree,
    /// On a row of a grid inside the active tab.
    Grid,
}

/// The scope a context menu was opened in (the Rune
/// `inventory_menu::Scope`): the pool of the row, the tree scope, the tab,
/// and an observation of the pool, which holds the connection generation.
/// A reconnect, a pool switch, a scope change or a tab change makes a
/// captured scope differ from a fresh one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuScope {
    pub(crate) pool: Option<PoolId>,
    pub(crate) scope: Target,
    pub(crate) tab: TabId,
    pub(crate) observation: Option<ObservedResource>,
}

/// An open context menu (the Rune `inventory_menu::Context`): the scope and
/// the selection captured when it opened. Every item checks both again when
/// it is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuContext {
    pub(crate) scope: MenuScope,
    pub origin: MenuOrigin,
    /// The row the menu was opened on.
    pub object: ObjectRef,
    /// What the menu's commands act on: the selection as it was when the
    /// menu opened.
    pub target: Target,
}

/// Why a stale context menu refuses its items.
pub const STALE_MENU: &str =
    "The inventory or the selection changed after this menu opened. Open the menu again.";

#[cfg(test)]
mod tests {
    use super::*;

    fn place(n: u8, tab: TabId) -> Place {
        Place {
            scope: Target::one(ObjectRef::new(None, ObjectKind::Folder, format!("/{n}"))),
            tab,
        }
    }

    #[test]
    fn back_and_forward_step_through_places_and_a_new_visit_drops_the_forward_stack() {
        let mut history = History::default();
        let (a, b, c) = (
            place(1, TabId::Search),
            place(2, TabId::Search),
            place(3, TabId::Search),
        );
        assert!(!history.can_go_back() && !history.can_go_forward());
        history.visit(a.clone()); // went from a to b
        history.visit(b.clone()); // went from b to c
        assert_eq!(history.back(c.clone(), |_| true), Some(b.clone()));
        assert_eq!(history.back(b.clone(), |_| true), Some(a.clone()));
        assert_eq!(history.back(a.clone(), |_| true), None);
        assert!(history.can_go_forward());
        assert_eq!(history.forward(a.clone(), |_| true), Some(b.clone()));
        // Going somewhere new from b: c can no longer be reached by Forward.
        history.visit(b.clone());
        assert!(!history.can_go_forward());
        assert_eq!(history.back(place(4, TabId::Search), |_| true), Some(b));
    }

    #[test]
    fn a_place_whose_object_is_gone_is_skipped() {
        let mut history = History::default();
        let (a, b, c) = (
            place(1, TabId::Search),
            place(2, TabId::Search),
            place(3, TabId::Search),
        );
        history.visit(a.clone());
        history.visit(b.clone());
        let gone = b.clone();
        assert_eq!(history.back(c, |place| *place != gone), Some(a));
        assert!(!history.can_go_back());
    }

    #[test]
    fn the_history_is_bounded() {
        let mut history = History::default();
        for n in 0..=255u8 {
            history.visit(place(n, TabId::Search));
        }
        let mut steps = 0;
        let mut from = place(0, TabId::General);
        while let Some(next) = history.back(from.clone(), |_| true) {
            from = next;
            steps += 1;
        }
        assert_eq!(steps, History::LIMIT);
    }
}
