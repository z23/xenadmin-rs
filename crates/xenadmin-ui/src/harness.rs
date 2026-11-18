//! `Harness`: drives a [`Model`] through its descriptions, without a window.
//!
//! A Rune test that built a `rune::Runtime`, found elements by key and
//! pressed them becomes a harness test that asserts the same thing:
//!
//! ```ignore
//! let mut h = Harness::scene("host-administration");
//! h.type_text("host-name", "xcp-lab-09");
//! h.click("host-review");
//! assert_eq!(h.node("host-confirm").disabled, None);
//! ```
//!
//! What the harness can reach is what a user can reach. While a surface is
//! open only the surface in front takes input, exactly as a modal dialog
//! does. With no surface open it reaches the toolbar, the local action row
//! of the active tab and the body of the active tab. Menu commands are run
//! with [`Harness::invoke`].

use std::path::PathBuf;

use crate::command::{self, CommandId, Eligibility, MenuItem, MenuView};
use crate::intent::Intent;
use crate::model::Model;
use crate::nav::{NavRow, NavView};
use crate::object::{ObjectRef, Target};
use crate::scene;
use crate::selection::MenuOrigin;
use crate::surface::{self, Element, Handle, Node, Surface};
use crate::tabs::{TabBody, TabId};

pub struct Harness {
    /// The model under test, for assertions on state and for set-up.
    pub model: Model,
    clock: f64,
}

impl Default for Harness {
    fn default() -> Self {
        Self::new()
    }
}

impl Harness {
    /// A harness over [`Model::fixture`].
    pub fn new() -> Self {
        Self::with(Model::fixture())
    }

    /// A harness over a model the test prepared.
    pub fn with(model: Model) -> Self {
        Self { model, clock: 0.0 }
    }

    /// A harness over a fixture model with the named scene applied. Panics
    /// when no such scene is registered.
    #[track_caller]
    pub fn scene(name: &str) -> Self {
        let mut model = Model::fixture();
        assert!(
            scene::apply(name, &mut model),
            "the scene `{name}` is not registered"
        );
        Self::with(model)
    }

    // -- What is shown ------------------------------------------------------

    /// Every open surface, back to front.
    pub fn surfaces(&self) -> Vec<Surface> {
        self.model.surfaces()
    }

    /// The surface in front. Panics when none is open.
    #[track_caller]
    pub fn surface(&self) -> Surface {
        self.model
            .front_surface()
            .unwrap_or_else(|| panic!("no surface is open"))
    }

    /// The id of the surface in front, when one is open.
    pub fn surface_id(&self) -> Option<String> {
        self.model.front_surface().map(|surface| surface.id)
    }

    /// The body of the active tab. Panics when no area provides it.
    #[track_caller]
    pub fn tab(&self) -> TabBody {
        let tab = self.model.active_tab();
        self.model.tab_body(tab).unwrap_or_else(|| {
            panic!(
                "no area provides the {} tab for {:?}",
                tab.title(),
                self.model.scope().kind()
            )
        })
    }

    /// Selects in the tree.
    pub fn select(&mut self, scope: impl Into<Target>) {
        self.model.select(scope.into());
    }

    /// Chooses a tab of the scope. Panics when the scope does not have it.
    #[track_caller]
    pub fn open_tab(&mut self, tab: TabId) {
        assert!(
            self.model.tabs().contains(&tab),
            "the selection has no {} tab; it has {:?}",
            tab.title(),
            self.model.tabs()
        );
        self.model.set_active_tab(tab);
    }

    fn reachable(&self) -> Vec<Element> {
        if let Some(surface) = self.model.front_surface() {
            return surface::elements(&surface);
        }
        let tab = self.model.active_tab();
        let mut elements = Vec::new();
        let toolbar = command::toolbar(&self.model);
        let local = command::local_row(&self.model, tab);
        // A command that is in the toolbar and in the local row is two
        // buttons: each is keyed by its place, so no key occurs twice.
        let twice = |id: CommandId| {
            toolbar.iter().flatten().any(|item| item.id == id)
                && local.iter().any(|item| item.id == id)
        };
        for item in toolbar.iter().flatten() {
            elements.push(command_element("toolbar", item, twice(item.id)));
        }
        for item in &local {
            elements.push(command_element("local", item, twice(item.id)));
        }
        if let Some(body) = self.model.tab_body(tab) {
            elements.extend(surface::block_elements("tab", &body.blocks()));
        }
        elements
    }

    fn place(&self) -> String {
        match self.model.front_surface() {
            Some(surface) => format!("the surface `{}`", surface.id),
            None => format!("the {} tab", self.model.active_tab().title()),
        }
    }

    /// The nodes a user can reach now: those of the surface in front, or
    /// with no surface open the toolbar, the local action row and the body
    /// of the active tab.
    pub fn nodes(&self) -> Vec<Node> {
        self.reachable().into_iter().map(|e| e.node).collect()
    }

    /// The node with this key among [`Harness::nodes`]. A command that is
    /// in the toolbar and in the local row is found by its id too: that is
    /// its button of the local row (`toolbar:<id>` is the other one).
    pub fn find(&self, key: &str) -> Option<Node> {
        self.element(key).ok().map(|element| element.node)
    }

    /// The node with this key. Panics when it is not there.
    #[track_caller]
    pub fn node(&self, key: &str) -> Node {
        match self.element(key) {
            Ok(element) => element.node,
            Err(missing) => panic!("{missing}"),
        }
    }

    /// The keys of [`Harness::nodes`], in reading order.
    pub fn keys(&self) -> Vec<String> {
        self.nodes()
            .into_iter()
            .filter_map(|node| node.key)
            .collect()
    }

    /// Every string a user can read in [`Harness::nodes`], in reading order.
    pub fn texts(&self) -> Vec<String> {
        self.nodes()
            .iter()
            .flat_map(Node::strings)
            .map(str::to_owned)
            .collect()
    }

    /// Whether some visible string contains `needle`.
    pub fn has_text(&self, needle: &str) -> bool {
        self.nodes()
            .iter()
            .flat_map(Node::strings)
            .any(|text| text.contains(needle))
    }

    /// Panics when a key occurs twice among [`Harness::nodes`].
    #[track_caller]
    pub fn assert_unique_keys(&self) {
        let duplicates = surface::duplicate_keys(&self.nodes());
        assert!(
            duplicates.is_empty(),
            "duplicate keys in {}: {duplicates:?}",
            self.place()
        );
    }

    /// One line per reachable node, in reading order, to look at a surface
    /// or a tab while writing a test (`println!("{}", h.dump())`): the role,
    /// then the key (the path where the node has none), and whichever of
    /// label, value, text, checked and the reason it is disabled the node
    /// has. The first line says what is reachable.
    pub fn dump(&self) -> String {
        let mut out = format!("{}:\n", self.place());
        for node in self.nodes() {
            out.push_str(node.role);
            match &node.key {
                Some(key) => out.push_str(&format!(" key={key}")),
                None => out.push_str(&format!(" path={}", node.path)),
            }
            for (name, value) in [
                ("label", &node.label),
                ("value", &node.value),
                ("text", &node.text),
            ] {
                if let Some(value) = value {
                    out.push_str(&format!(" {name}={value:?}"));
                }
            }
            if let Some(checked) = node.checked {
                out.push_str(&format!(" checked={checked}"));
            }
            if let Some(reason) = &node.disabled {
                out.push_str(&format!(" disabled={reason:?}"));
            }
            out.push('\n');
        }
        out
    }

    fn element(&self, key: &str) -> Result<Element, String> {
        let elements = self.reachable();
        // A command shown twice is keyed by place (`local:<id>`,
        // `toolbar:<id>`); its bare id finds the one of the local row.
        let by_place = [format!("local:{key}"), format!("toolbar:{key}")];
        let found = elements
            .iter()
            .find(|element| element.node.key.as_deref() == Some(key))
            .or_else(|| {
                by_place.iter().find_map(|placed| {
                    elements
                        .iter()
                        .find(|element| element.node.key.as_deref() == Some(placed))
                })
            });
        match found {
            Some(element) => Ok(element.clone()),
            None => {
                let keys: Vec<_> = elements
                    .iter()
                    .filter_map(|element| element.node.key.as_deref())
                    .collect();
                Err(format!(
                    "no element with key `{key}` in {}; keys there: {keys:?}",
                    self.place()
                ))
            }
        }
    }

    // -- Interaction --------------------------------------------------------

    /// Presses the element: a button, a checkbox, an option (which picks
    /// it), a row (which selects it), the header of a sortable column, a
    /// toolbar or local-row command.
    /// `Err` carries the reason when the element is disabled, or says that
    /// it is missing or cannot be pressed.
    pub fn try_click(&mut self, key: &str) -> Result<(), String> {
        let element = self.element(key)?;
        if let Some(reason) = element.node.disabled {
            return Err(reason);
        }
        match element.handle {
            Some(Handle::Press(act)) | Some(Handle::Toggle(act)) => act(&mut self.model),
            Some(Handle::Pick { id, on_pick, .. }) => on_pick(&mut self.model, &id),
            Some(Handle::Row {
                id,
                multi,
                on_select,
                on_select_many,
                ..
            }) => match (multi, on_select, on_select_many) {
                (false, Some(on_select), _) => on_select(&mut self.model, &id),
                (true, _, Some(on_select_many)) => on_select_many(&mut self.model, &[id]),
                _ => return Err(format!("the row `{key}` cannot be selected")),
            },
            Some(Handle::Sort {
                column,
                descending,
                on_sort,
                ..
            }) => on_sort(&mut self.model, &column, descending),
            Some(Handle::Command(id)) => {
                let target = self.model.target();
                self.model
                    .try_invoke(id, &target)
                    .map_err(|refused| refused.to_string())?;
            }
            Some(Handle::Edit { .. } | Handle::Table { .. }) | None => {
                return Err(format!(
                    "the element `{key}` ({}) cannot be pressed",
                    element.node.role
                ));
            }
        }
        Ok(())
    }

    /// Presses the element. Panics with the reason when it is missing,
    /// disabled or not pressable.
    #[track_caller]
    pub fn click(&mut self, key: &str) {
        if let Err(reason) = self.try_click(key) {
            panic!("cannot click `{key}`: {reason}");
        }
    }

    /// Replaces the text of a text field: the field's `on_edit` receives
    /// `text`. Panics when the field is missing, disabled or not a text
    /// field.
    #[track_caller]
    pub fn type_text(&mut self, key: &str, text: &str) {
        match self.enabled(key) {
            Element {
                handle: Some(Handle::Edit { on_edit, .. }),
                ..
            } => on_edit(&mut self.model, text),
            element => panic!("`{key}` ({}) is not a text field", element.node.role),
        }
    }

    /// Presses Enter in a text field. Panics when the field has no
    /// `on_submit`.
    #[track_caller]
    pub fn submit(&mut self, key: &str) {
        match self.enabled(key) {
            Element {
                handle:
                    Some(Handle::Edit {
                        on_submit: Some(on_submit),
                        ..
                    }),
                ..
            } => on_submit(&mut self.model),
            element => panic!("`{key}` ({}) does not act on Enter", element.node.role),
        }
    }

    /// Toggles a checkbox. Panics when it is missing, disabled or not a
    /// checkbox.
    #[track_caller]
    pub fn toggle(&mut self, key: &str) {
        match self.enabled(key) {
            Element {
                handle: Some(Handle::Toggle(on_toggle)),
                ..
            } => on_toggle(&mut self.model),
            element => panic!("`{key}` ({}) is not a checkbox", element.node.role),
        }
    }

    /// Picks the option with id `option` in the choice, the select field or
    /// the page list with key `key`. Panics when the owner or the option is
    /// missing or disabled.
    #[track_caller]
    pub fn pick(&mut self, key: &str, option: &str) {
        let owner = self.enabled(key);
        let elements = self.reachable();
        let options: Vec<_> = elements
            .iter()
            .filter_map(|element| match &element.handle {
                Some(Handle::Pick { owner, id, on_pick }) if owner == key => {
                    Some((id.clone(), on_pick.clone(), element.node.disabled.clone()))
                }
                _ => None,
            })
            .collect();
        assert!(
            !options.is_empty(),
            "`{key}` ({}) has no options to pick",
            owner.node.role
        );
        match options.iter().find(|(id, _, _)| id == option) {
            Some((id, on_pick, None)) => on_pick(&mut self.model, id),
            Some((_, _, Some(reason))) => {
                panic!("cannot pick `{option}` in `{key}`: {reason}")
            }
            None => {
                let ids: Vec<_> = options.iter().map(|(id, _, _)| id.as_str()).collect();
                panic!("`{key}` has no option `{option}`; it has {ids:?}")
            }
        }
    }

    fn row(&self, key: &str, row: &str) -> Handle {
        self.enabled(key);
        let rows: Vec<_> = self
            .reachable()
            .into_iter()
            .filter_map(|element| match &element.handle {
                Some(Handle::Row { table, id, .. }) if table == key => {
                    Some((id.clone(), element.handle.clone()))
                }
                _ => None,
            })
            .collect();
        match rows.iter().find(|(id, _)| id == row) {
            Some((_, Some(handle))) => handle.clone(),
            _ => {
                let ids: Vec<_> = rows.iter().map(|(id, _)| id.as_str()).collect();
                panic!("the table `{key}` has no row `{row}`; it has {ids:?}")
            }
        }
    }

    /// Selects one row of a table by its id. In a multi-selection table the
    /// row becomes the whole selection. Panics when the table or the row is
    /// missing or the table has no selection.
    #[track_caller]
    pub fn select_row(&mut self, key: &str, row: &str) {
        match self.row(key, row) {
            Handle::Row {
                multi: false,
                on_select: Some(on_select),
                ..
            } => on_select(&mut self.model, row),
            Handle::Row {
                multi: true,
                on_select_many: Some(on_select_many),
                ..
            } => on_select_many(&mut self.model, &[row.to_owned()]),
            _ => panic!("the rows of `{key}` cannot be selected"),
        }
    }

    /// Selects several rows of a multi-selection table by their ids.
    #[track_caller]
    pub fn select_rows(&mut self, key: &str, rows: &[&str]) {
        for row in rows {
            self.row(key, row);
        }
        match self.enabled(key).handle {
            Some(Handle::Table {
                multi: true,
                on_select_many: Some(on_select_many),
                ..
            }) => {
                let rows: Vec<String> = rows.iter().map(|row| (*row).to_owned()).collect();
                on_select_many(&mut self.model, &rows);
            }
            _ => panic!("`{key}` is not a multi-selection table"),
        }
    }

    /// Presses the header of a sortable column of the table `key`: the
    /// table's `on_sort` receives the column and the new direction (a press
    /// on the column the table is sorted by reverses the order). Panics when
    /// the table or the column is missing or the column is not sortable.
    #[track_caller]
    pub fn sort(&mut self, key: &str, column: &str) {
        self.enabled(key);
        let header = surface::Table::column_key(key, column);
        if let Err(reason) = self.try_click(&header) {
            panic!("cannot sort `{key}` by `{column}`: {reason}");
        }
    }

    /// Activates a row (double click, Enter). Panics when the table has no
    /// `on_activate`.
    #[track_caller]
    pub fn activate_row(&mut self, key: &str, row: &str) {
        match self.row(key, row) {
            Handle::Row {
                on_activate: Some(on_activate),
                ..
            } => on_activate(&mut self.model, row),
            _ => panic!("the rows of `{key}` cannot be activated"),
        }
    }

    /// Dismisses the surface in front: Escape, the overlay, the close
    /// button. Panics when no surface is open.
    #[track_caller]
    pub fn dismiss(&mut self) {
        let surface = self.surface();
        (surface.on_close)(&mut self.model);
    }

    #[track_caller]
    fn enabled(&self, key: &str) -> Element {
        match self.element(key) {
            Ok(element) => {
                if let Some(reason) = &element.node.disabled {
                    panic!("`{key}` is disabled: {reason}");
                }
                element
            }
            Err(missing) => panic!("{missing}"),
        }
    }

    // -- Commands -----------------------------------------------------------

    fn command_id(command: &str) -> Result<CommandId, String> {
        command::registry()
            .iter()
            .find(|spec| spec.id.0 == command)
            .map(|spec| spec.id)
            .ok_or_else(|| format!("there is no command `{command}`"))
    }

    /// The eligibility of a command on the model's target. `Hidden` for an
    /// unknown id.
    pub fn eligibility(&self, command: &str) -> Eligibility {
        match Self::command_id(command) {
            Ok(id) => self.model.eligibility(id, &self.model.target()),
            Err(_) => Eligibility::Hidden,
        }
    }

    /// Runs a command on the model's target, as a menu item does. `Err`
    /// carries the reason when it is disabled, hidden or unknown.
    pub fn try_invoke(&mut self, command: &str) -> Result<(), String> {
        let target = self.model.target();
        self.try_invoke_on(command, &target)
    }

    /// Runs a command on a given target, as a context menu item does.
    pub fn try_invoke_on(&mut self, command: &str, target: &Target) -> Result<(), String> {
        let id = Self::command_id(command)?;
        self.model
            .try_invoke(id, target)
            .map_err(|refused| refused.to_string())
    }

    /// Runs a command on the model's target. Panics with the reason when it
    /// is refused.
    #[track_caller]
    pub fn invoke(&mut self, command: &str) {
        if let Err(reason) = self.try_invoke(command) {
            panic!("cannot invoke `{command}`: {reason}");
        }
    }

    /// Runs a command on a given target. Panics with the reason when it is
    /// refused.
    #[track_caller]
    pub fn invoke_on(&mut self, command: &str, target: &Target) {
        if let Err(reason) = self.try_invoke_on(command, target) {
            panic!("cannot invoke `{command}`: {reason}");
        }
    }

    /// The menu bar for the model's target.
    pub fn menus(&self) -> Vec<MenuView> {
        command::menu_bar(&self.model)
    }

    /// The item of a command in the menu bar, when it is shown.
    pub fn menu_item(&self, command: &str) -> Option<MenuItem> {
        self.menus()
            .iter()
            .flat_map(MenuView::items)
            .find(|item| item.id.0 == command)
            .cloned()
    }

    /// The toolbar for the model's target, flattened.
    pub fn toolbar(&self) -> Vec<MenuItem> {
        command::toolbar(&self.model)
            .into_iter()
            .flatten()
            .collect()
    }

    /// The local action row of the active tab.
    pub fn local_row(&self) -> Vec<MenuItem> {
        command::local_row(&self.model, self.model.active_tab())
    }

    /// The row context menu of the table with key `key`: the table's
    /// `row_menu` commands resolved against the model's target, which is the
    /// row the test selected before (a right click selects the row first).
    /// Hidden commands are omitted. Panics when the table is missing.
    #[track_caller]
    pub fn row_menu(&self, key: &str) -> Vec<MenuItem> {
        match self.element(key) {
            Ok(Element {
                handle: Some(Handle::Table { row_menu, .. }),
                ..
            }) => command::resolve(&self.model, &row_menu, &self.model.target()),
            Ok(element) => panic!("`{key}` ({}) is not a table", element.node.role),
            Err(missing) => panic!("{missing}"),
        }
    }

    /// The context menu of a target, flattened.
    pub fn context_menu(&self, target: &Target) -> Vec<MenuItem> {
        command::context_menu(&self.model, target)
            .into_iter()
            .flatten()
            .collect()
    }

    // -- Navigation and context menus --------------------------------------

    /// The rows of the active navigation view, narrowed by the filter: what
    /// the tree shows.
    pub fn nav_rows(&self) -> Vec<NavRow> {
        crate::nav::rows(&self.model)
    }

    /// The first row of the navigation tree with this label. Panics, listing
    /// the labels there, when there is none.
    #[track_caller]
    pub fn nav_row(&self, label: &str) -> NavRow {
        let rows = self.nav_rows();
        match rows.iter().find(|row| row.label == label) {
            Some(row) => row.clone(),
            None => panic!(
                "no row `{label}` in the {} view; it has {:?}",
                self.model.nav.view.title(),
                rows.iter()
                    .map(|row| row.label.as_str())
                    .collect::<Vec<_>>()
            ),
        }
    }

    /// Presses the first row of the navigation tree with this label, as a
    /// click on it does: an object row selects its object.
    #[track_caller]
    pub fn press_nav(&mut self, label: &str) {
        let row = self.nav_row(label);
        self.model.select_nav_row(&row.id);
    }

    /// Switches the navigation view.
    pub fn open_view(&mut self, view: NavView) {
        self.model.set_nav_view(view);
    }

    /// A secondary press on the row of an object, in the tree or in a grid
    /// of the active tab. Returns the items of the menu that opens,
    /// flattened; none when no menu opens.
    pub fn open_menu(&mut self, object: ObjectRef, origin: MenuOrigin) -> Vec<MenuItem> {
        if !self.model.open_context_menu(object, origin) {
            return Vec::new();
        }
        self.model.context_menu().into_iter().flatten().collect()
    }

    /// Chooses an item of the open context menu. `Err` carries the reason
    /// when the menu is stale or closed, or the command is refused. The menu
    /// closes either way.
    pub fn try_choose(&mut self, command: &str) -> Result<(), String> {
        let id = Self::command_id(command)?;
        self.model
            .invoke_context(id)
            .map_err(|refused| refused.to_string())
    }

    /// Chooses an item of the open context menu. Panics with the reason
    /// when it is refused.
    #[track_caller]
    pub fn choose(&mut self, command: &str) {
        if let Err(reason) = self.try_choose(command) {
            panic!("cannot choose `{command}`: {reason}");
        }
    }

    // -- Time and intents ---------------------------------------------------

    /// Advances the clock by `seconds` and polls the model, as a wake or a
    /// timed tick does. Returns what the poll returned.
    pub fn tick(&mut self, seconds: f64) -> bool {
        self.clock += seconds;
        self.model.poll(self.clock)
    }

    /// Takes the intents the model queued, as the front end does after
    /// every update.
    pub fn take_intents(&mut self) -> Vec<Intent> {
        self.model.take_intents()
    }

    /// Answers the oldest queued file prompt with a path, or with `None` as
    /// a cancelled prompt does. Other queued intents stay queued. Panics
    /// when no file prompt is queued.
    #[track_caller]
    pub fn answer_file(&mut self, path: Option<PathBuf>) {
        let index = self
            .model
            .intents
            .iter()
            .position(|intent| matches!(intent, Intent::ChooseFile(_)))
            .unwrap_or_else(|| panic!("no file prompt is queued: {:?}", self.model.intents));
        if let Intent::ChooseFile(request) = self.model.intents.remove(index) {
            (request.reply)(&mut self.model, path);
        }
    }

    /// Answers the oldest queued clipboard read. Panics when none is queued.
    #[track_caller]
    pub fn answer_clipboard(&mut self, text: Option<&str>) {
        let index = self
            .model
            .intents
            .iter()
            .position(|intent| matches!(intent, Intent::ReadClipboard(_)))
            .unwrap_or_else(|| panic!("no clipboard read is queued: {:?}", self.model.intents));
        if let Intent::ReadClipboard(reply) = self.model.intents.remove(index) {
            reply(&mut self.model, text.map(str::to_owned));
        }
    }

    /// The element tree of the model as JSON ([`scene::tree`]).
    pub fn tree(&self) -> serde_json::Value {
        scene::tree(&self.model)
    }
}

/// The button of a command in the toolbar (`base` "toolbar") or the local
/// row ("local"). Its key is the command id, or `<base>:<id>` when the
/// command is shown in both places.
fn command_element(base: &str, item: &MenuItem, by_place: bool) -> Element {
    let key = match by_place {
        true => format!("{base}:{}", item.id),
        false => item.id.0.to_owned(),
    };
    let node = Node {
        path: format!("{base}/{}", item.id),
        key: Some(key),
        role: "button",
        label: Some(item.label.clone()),
        disabled: item.disabled.clone(),
        checked: item.checked,
        ..Node::default()
    };
    Element {
        node,
        handle: Some(Handle::Command(item.id)),
    }
}

/// A test-only area with one of every control, for the tests below.
#[cfg(test)]
pub(crate) mod probe {
    use crate::prelude::*;

    #[derive(Default)]
    pub struct State {
        /// The probe's commands and its Home tab body exist only when set,
        /// so no other test sees them.
        pub active: bool,
        pub open: bool,
        pub child: bool,
        pub name: String,
        pub password: SecretText,
        pub force: bool,
        pub host: Option<String>,
        pub mode: String,
        pub page: String,
        pub disk: Option<String>,
        pub disks: Vec<String>,
        pub activated: Option<String>,
        pub sort: Option<(String, bool)>,
        pub presses: u32,
        pub submits: u32,
        pub polls: u32,
        pub tool_runs: u32,
        /// Shows `probe.both`, a command in the toolbar and in the local row.
        pub both: bool,
        pub both_runs: u32,
        /// The scope and session hooks the kernel called, in order.
        pub hooks: Vec<&'static str>,
    }

    pub fn spec() -> AreaSpec {
        AreaSpec {
            commands,
            surfaces,
            poll,
            on_scope_change: |m| m.areas.probe.hooks.push("scope"),
            on_session_change: |m| m.areas.probe.hooks.push("session"),
            ..AreaSpec::new("probe")
        }
    }

    fn when_active(m: &Model, _: &Target) -> Eligibility {
        if !m.areas.probe.active {
            Eligibility::Hidden
        } else if m.areas.probe.name == "busy" {
            Eligibility::disabled("The probe is busy.")
        } else {
            Eligibility::Enabled
        }
    }

    fn commands() -> Vec<CommandSpec> {
        vec![
            CommandSpec::new("probe.open", "Open Probe", "probe")
                .menu(MenuPath::new(Menu::Tools, 200, 10))
                .local(TabId::Home, 10)
                .eligibility(when_active)
                .invoke(|m, _| m.areas.probe.open = true),
            CommandSpec::new("probe.tool", "Probe Tool", "probe")
                .toolbar(ToolbarSlot::new(ToolbarGroup::New, 200))
                .eligibility(when_active)
                .invoke(|m, _| m.areas.probe.tool_runs += 1),
            CommandSpec::new("probe.both", "Probe Both", "probe")
                .toolbar(ToolbarSlot::new(ToolbarGroup::New, 210))
                .local(TabId::Home, 20)
                .eligibility(|m, _| match m.areas.probe.both {
                    true => Eligibility::Enabled,
                    false => Eligibility::Hidden,
                })
                .invoke(|m, _| m.areas.probe.both_runs += 1),
        ]
    }

    fn poll(m: &mut Model, _clock: f64) -> bool {
        if !m.areas.probe.active {
            return false;
        }
        m.areas.probe.polls += 1;
        m.request_tick(1.0);
        true
    }

    /// The Home tab body, asked first by `Model::tab_body` in test builds.
    pub fn tab(m: &Model, tab: TabId, _scope: &Target) -> Option<TabBody> {
        (m.areas.probe.active && tab == TabId::Home).then(|| {
            TabBody::Blocks(blocks![
                caption("Probe tab").key("probe-tab-text"),
                button("probe-tab-button", "Open", |m| m.areas.probe.open = true),
            ])
        })
    }

    fn surfaces(m: &Model) -> Vec<Surface> {
        let s = &m.areas.probe;
        let mut surfaces = Vec::new();
        if s.open {
            let disk_rows = || {
                [
                    row("d1", ["Disk 1", "8.0 GiB"]),
                    row("d2", ["Disk 2", "1.0 GiB"]),
                ]
            };
            surfaces.push(
                Surface::dialog("probe", "Probe")
                    .body(blocks![
                        input("probe-name", "Name", &s.name, |m, text| {
                            m.areas.probe.name = text.to_owned()
                        })
                        .on_submit(|m| m.areas.probe.submits += 1),
                        secret_input("probe-password", "Password", &s.password, |m, text| {
                            m.areas.probe.password.set(text)
                        }),
                        textarea("probe-notes", "Notes", "", |_, _| {})
                            .disabled_if(true, "Notes are read-only here."),
                        checkbox("probe-force", "Force", s.force, |m| {
                            m.areas.probe.force = !m.areas.probe.force
                        }),
                        checkbox("probe-locked", "Locked", false, |m| {
                            m.areas.probe.force = true
                        })
                        .disabled_if(true, "Locked by policy."),
                        select(
                            "probe-host",
                            "Host",
                            vec![
                                option("a", "Host A"),
                                option("b", "Host B").disabled_if(true, "Host B is full."),
                                option("c", "Host C"),
                            ],
                            s.host.as_deref(),
                            |m, id| m.areas.probe.host = Some(id.to_owned())
                        ),
                        choice(
                            "probe-mode",
                            vec![
                                option("live", "Live"),
                                option("halted", "Halted").key("probe-halted-legacy"),
                            ],
                            Some(s.mode.as_str()),
                            |m, id| m.areas.probe.mode = id.to_owned()
                        ),
                        table(
                            "probe-disks",
                            [column("name", "Name").sortable(), column("size", "Size")],
                            disk_rows()
                        )
                        .sorted(
                            s.sort.as_ref().map_or("name", |sort| sort.0.as_str()),
                            s.sort.as_ref().is_some_and(|sort| sort.1),
                            |m, column, descending| {
                                m.areas.probe.sort = Some((column.to_owned(), descending))
                            }
                        )
                        .single(s.disk.as_deref(), |m, id| {
                            m.areas.probe.disk = Some(id.to_owned())
                        })
                        .on_activate(|m, id| m.areas.probe.activated = Some(id.to_owned())),
                        table(
                            "probe-many",
                            [("name", "Name"), ("size", "Size")],
                            disk_rows()
                        )
                        .multi(s.disks.clone(), |m, ids| m
                            .areas
                            .probe
                            .disks =
                            ids.to_vec()),
                        table("probe-plain", [("name", "Name")], [row("p1", ["Plain"])])
                            .row_menu([CommandId("probe.open"), CommandId("probe.missing")]),
                        button("probe-press", "Press", |m| m.areas.probe.presses += 1),
                    ])
                    .footer([
                        button("probe-cancel", "Cancel", |m| m.areas.probe.open = false),
                        button("probe-child", "More", |m| m.areas.probe.child = true),
                        button("probe-confirm", "Confirm", |m| m.areas.probe.presses += 100)
                            .primary()
                            .disabled_if(s.name.is_empty(), "Enter a name."),
                    ])
                    .on_close(|m| m.areas.probe.open = false),
            );
        }
        if s.child {
            surfaces.push(
                Surface::properties(
                    "probe-child",
                    "Probe properties",
                    [("general", "General"), ("advanced", "Advanced")],
                    if s.page.is_empty() {
                        "general"
                    } else {
                        s.page.as_str()
                    },
                    |m, id| m.areas.probe.page = id.to_owned(),
                )
                .body(blocks![caption("Child").key("probe-child-text")])
                .on_close(|m| m.areas.probe.child = false),
            );
        }
        surfaces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_probe() -> Harness {
        let mut h = Harness::new();
        h.model.areas.probe.active = true;
        h.model.areas.probe.open = true;
        h
    }

    #[test]
    fn click_runs_the_handler() {
        let mut h = open_probe();
        h.click("probe-press");
        h.click("probe-press");
        assert_eq!(h.model.areas.probe.presses, 2);
        assert_eq!(h.try_click("probe-press"), Ok(()));
        assert_eq!(h.model.areas.probe.presses, 3);
    }

    #[test]
    fn a_disabled_button_refuses_with_its_reason() {
        let mut h = open_probe();
        assert_eq!(h.try_click("probe-confirm"), Err("Enter a name.".into()));
        assert_eq!(h.model.areas.probe.presses, 0);
        assert_eq!(
            h.node("probe-confirm").disabled.as_deref(),
            Some("Enter a name.")
        );
        h.type_text("probe-name", "vm-1");
        h.click("probe-confirm");
        assert_eq!(h.model.areas.probe.presses, 100);
    }

    #[test]
    #[should_panic(expected = "cannot click `probe-confirm`: Enter a name.")]
    fn click_panics_with_the_reason_of_a_disabled_button() {
        open_probe().click("probe-confirm");
    }

    #[test]
    fn a_missing_key_is_reported_with_the_keys_that_exist() {
        let mut h = open_probe();
        let error = h.try_click("no-such-key").expect_err("missing");
        assert!(error.starts_with("no element with key `no-such-key` in the surface `probe`"));
        assert!(error.contains("probe-press"));
        assert_eq!(
            h.try_click("probe-name"),
            Err("the element `probe-name` (textbox) cannot be pressed".into())
        );
    }

    #[test]
    fn type_text_reaches_on_edit_and_enter_reaches_on_submit() {
        let mut h = open_probe();
        h.type_text("probe-name", "web-01");
        assert_eq!(h.model.areas.probe.name, "web-01");
        assert_eq!(h.node("probe-name").value.as_deref(), Some("web-01"));
        h.submit("probe-name");
        assert_eq!(h.model.areas.probe.submits, 1);
        h.type_text("probe-password", "hunter2");
        assert_eq!(h.model.areas.probe.password.expose(), "hunter2");
        // The description shows a mask, never the secret.
        assert_eq!(h.node("probe-password").value.as_deref(), Some("•••••••"));
        assert!(!h.has_text("hunter2"));
    }

    #[test]
    #[should_panic(expected = "`probe-notes` is disabled: Notes are read-only here.")]
    fn type_text_refuses_a_disabled_field() {
        open_probe().type_text("probe-notes", "x");
    }

    #[test]
    #[should_panic(expected = "`probe-force` (checkbox) is not a text field")]
    fn type_text_refuses_what_is_not_a_text_field() {
        open_probe().type_text("probe-force", "x");
    }

    #[test]
    fn toggle_flips_a_checkbox_and_refuses_a_disabled_one() {
        let mut h = open_probe();
        assert_eq!(h.node("probe-force").checked, Some(false));
        h.toggle("probe-force");
        assert_eq!(h.node("probe-force").checked, Some(true));
        h.click("probe-force");
        assert!(!h.model.areas.probe.force);
        assert_eq!(h.try_click("probe-locked"), Err("Locked by policy.".into()));
        assert!(!h.model.areas.probe.force);
    }

    #[test]
    #[should_panic(expected = "`probe-locked` is disabled: Locked by policy.")]
    fn toggle_panics_on_a_disabled_checkbox() {
        open_probe().toggle("probe-locked");
    }

    #[test]
    fn pick_chooses_an_option_of_a_choice_a_select_and_a_page_list() {
        let mut h = open_probe();
        h.pick("probe-mode", "halted");
        assert_eq!(h.model.areas.probe.mode, "halted");
        assert_eq!(h.node("probe-mode").value.as_deref(), Some("Halted"));
        // An option is an element of its own: clicking it picks it, and it
        // keeps the key of the Rune button it replaces when one is given.
        h.click("probe-mode-live");
        assert_eq!(h.model.areas.probe.mode, "live");
        h.click("probe-halted-legacy");
        assert_eq!(h.model.areas.probe.mode, "halted");
        assert_eq!(h.node("probe-halted-legacy").checked, Some(true));

        h.pick("probe-host", "c");
        assert_eq!(h.model.areas.probe.host.as_deref(), Some("c"));
        assert_eq!(h.node("probe-host").value.as_deref(), Some("Host C"));
        assert_eq!(h.try_click("probe-host-b"), Err("Host B is full.".into()));

        h.click("probe-child");
        h.pick("probe-child-pages", "advanced");
        assert_eq!(h.model.areas.probe.page, "advanced");
        assert_eq!(h.node("probe-child-pages-advanced").checked, Some(true));
    }

    #[test]
    #[should_panic(expected = "cannot pick `b` in `probe-host`: Host B is full.")]
    fn pick_refuses_a_disabled_option() {
        open_probe().pick("probe-host", "b");
    }

    #[test]
    #[should_panic(expected = "`probe-mode` has no option `paused`; it has [\"live\", \"halted\"]")]
    fn pick_reports_the_options_when_the_option_is_missing() {
        open_probe().pick("probe-mode", "paused");
    }

    #[test]
    fn select_row_selects_by_id_and_activation_is_separate() {
        let mut h = open_probe();
        h.select_row("probe-disks", "d2");
        assert_eq!(h.model.areas.probe.disk.as_deref(), Some("d2"));
        assert_eq!(h.node("probe-disks:d2").checked, Some(true));
        assert_eq!(h.node("probe-disks:d1").checked, Some(false));
        h.click("probe-disks:d1");
        assert_eq!(h.model.areas.probe.disk.as_deref(), Some("d1"));
        assert!(h.model.areas.probe.activated.is_none());
        h.activate_row("probe-disks", "d2");
        assert_eq!(h.model.areas.probe.activated.as_deref(), Some("d2"));

        h.select_row("probe-many", "d1");
        assert_eq!(h.model.areas.probe.disks, ["d1"]);
        h.select_rows("probe-many", &["d1", "d2"]);
        assert_eq!(h.model.areas.probe.disks, ["d1", "d2"]);
        assert_eq!(h.node("probe-many:d2").checked, Some(true));
        // A table without selection has rows that are not selectable.
        assert_eq!(h.node("probe-plain:p1").checked, None);
        assert_eq!(
            h.try_click("probe-plain:p1"),
            Err("the row `probe-plain:p1` cannot be selected".into())
        );
    }

    #[test]
    fn sort_presses_a_sortable_header_and_a_second_press_reverses_the_order() {
        let mut h = open_probe();
        assert_eq!(
            h.node("probe-disks#name").value.as_deref(),
            Some("ascending")
        );
        h.sort("probe-disks", "name");
        assert_eq!(h.model.areas.probe.sort, Some(("name".into(), true)));
        assert_eq!(
            h.node("probe-disks#name").value.as_deref(),
            Some("descending")
        );
        h.click("probe-disks#name");
        assert_eq!(h.model.areas.probe.sort, Some(("name".into(), false)));
        // Size is not sortable: its header has no key.
        assert!(h.find("probe-disks#size").is_none());
    }

    #[test]
    #[should_panic(expected = "cannot sort `probe-disks` by `size`")]
    fn sort_refuses_a_column_that_is_not_sortable() {
        open_probe().sort("probe-disks", "size");
    }

    #[test]
    fn a_row_menu_resolves_the_tables_commands_on_the_target() {
        let mut h = open_probe();
        let menu = h.row_menu("probe-plain");
        assert_eq!(menu.len(), 1, "an unknown command is omitted");
        assert_eq!(menu[0].id, CommandId("probe.open"));
        assert_eq!(menu[0].disabled, None);
        assert!(h.row_menu("probe-disks").is_empty());
        h.model.areas.probe.name = "busy".into();
        assert_eq!(
            h.row_menu("probe-plain")[0].disabled.as_deref(),
            Some("The probe is busy.")
        );
        // Hidden for the target: the menu omits it.
        h.model.areas.probe.active = false;
        assert!(h.row_menu("probe-plain").is_empty());
    }

    #[test]
    #[should_panic(expected = "the table `probe-disks` has no row `d9`; it has [\"d1\", \"d2\"]")]
    fn select_row_reports_the_rows_when_the_row_is_missing() {
        open_probe().select_row("probe-disks", "d9");
    }

    #[test]
    fn find_by_key_keys_and_texts_read_the_front_surface() {
        let h = open_probe();
        assert!(h.find("probe-name").is_some());
        assert!(h.find("missing").is_none());
        assert_eq!(h.node("probe").label.as_deref(), Some("Probe"));
        let keys = h.keys();
        assert_eq!(keys.first().map(String::as_str), Some("probe"));
        assert!(keys.iter().any(|key| key == "probe-disks:d1"));
        assert_eq!(keys.last().map(String::as_str), Some("probe-confirm"));
        h.assert_unique_keys();
        assert!(h.texts().iter().any(|text| text == "8.0 GiB"));
        assert!(h.has_text("Host B"));
        assert_eq!(h.surface_id().as_deref(), Some("probe"));
    }

    #[test]
    fn only_the_surface_in_front_takes_input() {
        let mut h = open_probe();
        h.click("probe-child");
        assert_eq!(h.surfaces().len(), 2);
        assert_eq!(h.surface().id, "probe-child");
        assert!(h.find("probe-child-text").is_some());
        let error = h.try_click("probe-press").expect_err("behind the child");
        assert!(error.contains("in the surface `probe-child`"));
        h.dismiss();
        assert_eq!(h.surface().id, "probe");
        h.click("probe-press");
        h.dismiss();
        assert!(h.surfaces().is_empty());
        assert_eq!(h.surface_id(), None);
    }

    #[test]
    fn without_a_surface_the_toolbar_the_local_row_and_the_tab_body_are_reachable() {
        let mut h = Harness::new();
        h.model.areas.probe.active = true;
        // The kernel's Back and Forward lead the toolbar. The commands of
        // the ported areas are there too (Add Server and the action row of
        // the Home tab); this test follows the kernel's and the probe's.
        let keys: Vec<String> = h
            .keys()
            .into_iter()
            .filter(|key| key.starts_with("nav.") || key.starts_with("probe"))
            .collect();
        assert_eq!(
            keys,
            [
                "nav.back",
                "nav.forward",
                "probe.tool",
                "probe.open",
                "probe-tab-text",
                "probe-tab-button"
            ]
        );
        assert_eq!(
            h.try_click("nav.back"),
            Err("There is no earlier selection to go back to.".into())
        );
        assert!(matches!(h.tab(), TabBody::Blocks(blocks) if blocks.len() == 2));
        assert_eq!(h.node("probe.tool").path, "toolbar/probe.tool");
        h.click("probe.tool");
        assert_eq!(h.model.areas.probe.tool_runs, 1);
        let probes = |items: Vec<MenuItem>| {
            items
                .iter()
                .filter(|item| item.id.0.starts_with("probe."))
                .count()
        };
        assert_eq!(probes(h.local_row()), 1);
        assert_eq!(probes(h.toolbar()), 1);
        // A local-row command is a button like any other.
        h.click("probe.open");
        assert_eq!(h.surface().id, "probe");
        // The tab is behind the dialog now.
        assert!(h.find("probe-tab-button").is_none());
        h.dismiss();
        h.click("probe-tab-button");
        assert!(h.model.areas.probe.open);
    }

    #[test]
    fn a_command_in_the_toolbar_and_the_local_row_is_keyed_by_place() {
        let mut h = Harness::new();
        h.model.areas.probe.active = true;
        h.model.areas.probe.both = true;
        h.assert_unique_keys();
        let keys = h.keys();
        assert!(keys.contains(&"toolbar:probe.both".to_owned()), "{keys:?}");
        assert!(keys.contains(&"local:probe.both".to_owned()), "{keys:?}");
        assert!(!keys.contains(&"probe.both".to_owned()));
        // A command shown once keeps its id as the key.
        assert!(keys.contains(&"probe.tool".to_owned()));
        // The id alone is the button of the local row.
        assert_eq!(h.node("probe.both").path, "local/probe.both");
        assert_eq!(h.node("toolbar:probe.both").path, "toolbar/probe.both");
        h.click("probe.both");
        h.click("toolbar:probe.both");
        h.click("local:probe.both");
        assert_eq!(h.model.areas.probe.both_runs, 3);
    }

    #[test]
    fn dump_has_one_line_for_every_reachable_node() {
        let mut h = open_probe();
        h.type_text("probe-name", "busy");
        let dump = h.dump();
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines[0], "the surface `probe`:");
        assert_eq!(lines.len(), h.nodes().len() + 1);
        let line = |key: &str| {
            let key = format!(" key={key} ");
            *lines
                .iter()
                .find(|line| line.contains(&key))
                .unwrap_or_else(|| panic!("no line for{key}in\n{dump}"))
        };
        assert!(line("probe-name").starts_with("textbox key=probe-name label=\"Name\" value=\"busy\""));
        let disabled = h
            .nodes()
            .into_iter()
            .find(|node| node.key.is_some() && node.disabled.is_some())
            .expect("the probe has a disabled control");
        let key = disabled.key.clone().unwrap();
        assert!(
            line(&key).ends_with(&format!(" disabled={:?}", disabled.disabled.unwrap())),
            "{}",
            line(&key)
        );
        // A node without a key is named by its path.
        assert!(lines.iter().any(|line| line.contains(" path=probe/")));
        // With no surface open the dump is of the tab.
        let h = Harness::new();
        assert!(h.dump().starts_with("the Home tab:\n"));
    }

    #[test]
    fn a_command_is_invoked_on_the_target_and_refused_with_its_reason() {
        let mut h = Harness::new();
        assert_eq!(h.eligibility("probe.open"), Eligibility::Hidden);
        assert!(h.menu_item("probe.open").is_none());
        assert!(
            h.try_invoke("probe.open")
                .expect_err("hidden")
                .contains("not available")
        );
        assert_eq!(
            h.try_invoke("no.such.command"),
            Err("there is no command `no.such.command`".into())
        );
        h.model.areas.probe.active = true;
        assert_eq!(
            h.menu_item("probe.open").map(|item| item.label),
            Some("Open Probe".into())
        );
        h.invoke("probe.open");
        assert!(h.model.areas.probe.open);
        h.dismiss();
        h.model.areas.probe.name = "busy".into();
        assert_eq!(h.try_invoke("probe.open"), Err("The probe is busy.".into()));
        assert_eq!(
            h.node("probe.open").disabled.as_deref(),
            Some("The probe is busy.")
        );
        assert_eq!(h.try_click("probe.open"), Err("The probe is busy.".into()));
        assert_eq!(
            h.try_invoke_on("probe.open", &Target::none()),
            Err("The probe is busy.".into())
        );
        assert!(!h.model.areas.probe.open);
    }

    #[test]
    #[should_panic(expected = "cannot invoke `probe.open`")]
    fn invoke_panics_when_the_command_is_refused() {
        Harness::new().invoke("probe.open");
    }

    #[test]
    fn tick_advances_the_clock_and_polls() {
        let mut h = Harness::new();
        assert!(!h.tick(1.0));
        h.model.areas.probe.active = true;
        assert!(h.tick(0.5));
        assert!(h.tick(0.25));
        assert_eq!(h.model.areas.probe.polls, 2);
        assert_eq!(h.model.clock(), 1.75);
        assert_eq!(h.model.take_tick_request(), Some(1.0));
    }

    #[test]
    fn file_and_clipboard_prompts_are_answered_in_place() {
        let mut h = Harness::new();
        h.model.copy_text("kept");
        h.model
            .choose_file(crate::FileRequest::open("Import", |m, path| {
                m.error = path.map(|path| path.display().to_string());
            }));
        h.model
            .read_clipboard(|m, text| m.areas.probe.name = text.unwrap_or_default());
        h.answer_file(Some(PathBuf::from("/tmp/vm.xva")));
        assert_eq!(h.model.error.as_deref(), Some("/tmp/vm.xva"));
        h.answer_clipboard(Some("pasted"));
        assert_eq!(h.model.areas.probe.name, "pasted");
        let rest = h.take_intents();
        assert!(matches!(rest.as_slice(), [Intent::CopyText(text)] if text == "kept"));
    }

    #[test]
    fn a_scene_starts_the_harness_and_gives_a_json_tree() {
        let h = Harness::scene("demo");
        assert!(h.model.demo);
        assert!(h.surfaces().is_empty());
        let tree = h.tree();
        assert_eq!(tree["demo"], serde_json::json!(true));
        // The demo scene shows the synthetic pool, selected.
        assert_eq!(tree["scope"][0]["kind"], serde_json::json!("pool"));
        assert_eq!(
            tree["tabs"],
            serde_json::json!([
                "general",
                "memory",
                "storage",
                "networking",
                "ha",
                "users",
                "search"
            ])
        );
    }

    #[test]
    fn the_tree_and_a_context_menu_are_driven_like_a_user_does() {
        let mut h = Harness::scene("demo");
        assert_eq!(h.nav_rows()[0].label, "Demonstration pool");
        h.press_nav("Database");
        let vm = h.model.selected().cloned().expect("the VM is selected");
        assert_eq!(vm.kind, crate::ObjectKind::Vm);
        assert_eq!(h.model.header().name, "Database");
        // A heading selects nothing.
        h.open_view(NavView::Objects);
        h.press_nav("Servers");
        assert_eq!(h.model.selected(), Some(&vm));
        h.open_view(NavView::Infrastructure);
        // The context menu of the VM offers Copy UUID and runs it.
        let items = h.open_menu(vm.clone(), MenuOrigin::Tree);
        assert!(items.iter().any(|item| item.id.0 == "object.copy-uuid"));
        h.choose("object.copy-uuid");
        assert!(matches!(h.take_intents().as_slice(), [Intent::CopyText(text)] if *text == vm.id));
        // The menu is closed now.
        assert_eq!(
            h.try_choose("object.copy-uuid"),
            Err("The command object.copy-uuid is not available for this selection.".into())
        );
        // No row, no menu.
        let gone = ObjectRef::new(vm.pool, crate::ObjectKind::Vm, uuid::Uuid::from_u128(7));
        assert!(h.open_menu(gone, MenuOrigin::Grid).is_empty());
    }

    #[test]
    #[should_panic(expected = "no row `No such row` in the Infrastructure view")]
    fn a_missing_tree_row_panics_with_the_labels_there() {
        Harness::scene("demo").press_nav("No such row");
    }

    #[test]
    #[should_panic(expected = "the scene `no-such-scene` is not registered")]
    fn an_unknown_scene_panics() {
        Harness::scene("no-such-scene");
    }

    // Whichever tab of the demo pool no area provides yet. Once every tab
    // has a provider the panic cannot be reached, and there is nothing left
    // to test.
    #[test]
    fn a_tab_without_a_provider_panics() {
        let mut h = Harness::scene("demo");
        let Some(tab) = h
            .model
            .tabs()
            .iter()
            .copied()
            .find(|tab| h.model.tab_body(*tab).is_none())
        else {
            return;
        };
        h.open_tab(tab);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.tab()))
            .map(|_| ())
            .expect_err("a tab without a provider must panic");
        let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
        assert_eq!(
            message,
            format!("no area provides the {} tab for Pool", tab.title())
        );
    }
}
