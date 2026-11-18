//! The Properties dialog of an object: one dialog the kernel owns, with
//! pages the areas provide.
//!
//! The plan gives the Properties dialog of one object pages from several
//! areas (a server: General, Custom Fields, Alerts, Multipathing, Log
//! Destination, Power On), while a [`Surface`] belongs to one area and a
//! command id is registered once. So the dialog, its state and its commands
//! are here, and an area hands in its pages through three hooks of its
//! [`AreaSpec`]:
//!
//! - `property_pages(&Model, &Target) -> Vec<PropertyPage>`: the pages it
//!   has for this target, each with its id, label, place in the page list
//!   and whether it is outside the supported core;
//! - `property_page(&Model, &Target, page_id) -> PageView`: the body, the
//!   footer buttons and the footer note of ONE page. Only the current page
//!   is asked for, once per description;
//! - `on_property_page(&mut Model, &Target, PageEvent)`: the page was
//!   entered or left, or the dialog closed. The area creates its draft on
//!   `Entered` and drops it on `Left` (or on `Closed`, when one draft spans
//!   several of its pages). This is what opening and closing the Rune dialog
//!   of the module did.
//!
//! The kernel owns the rest: which target and which page ([`Open`], in
//! `m.properties`), [`Model::open_properties`], [`Model::close_properties`],
//! [`Model::show_properties_page`], the surface (kind `Properties`, id
//! `<kind>-properties`, the experimental notice of the current page), and
//! the seven commands `server.properties`, `pool.properties`,
//! `vm.properties`, `template.properties`, `storage.properties`,
//! `network.properties` and `disk.properties`.
//!
//! The dialog closes when the tree scope changes, when the scope session
//! changes, when its object leaves the inventory and when the area stops
//! offering the current page. In the stack of surfaces it stands where the
//! surfaces of the area of its current page stand ([`Model::surfaces`]), so
//! what an area's page opens is in front of it exactly as it was in front of
//! that area's own dialog.

use crate::areas::{self, AreaSpec};
use crate::command::{CommandId, CommandSpec, Eligibility, KERNEL, place};
use crate::model::Model;
use crate::object::{ObjectKind, Target};
use crate::scene::SceneSpec;
use crate::surface::{Block, Button, Surface, Text};
use crate::tabs::TabId;

/// Which Properties dialog: the kind of object it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertiesKind {
    Host,
    Pool,
    Vm,
    Template,
    Storage,
    Network,
    Disk,
}

impl PropertiesKind {
    pub const ALL: [Self; 7] = [
        Self::Host,
        Self::Pool,
        Self::Vm,
        Self::Template,
        Self::Storage,
        Self::Network,
        Self::Disk,
    ];

    /// The dialog a target gets: that of its one object, or, for several
    /// VMs or several templates, the VM's or the template's (the plan's
    /// "Several VMs": the areas are given the whole target and `bulk_edit`
    /// answers for it). `None` for anything else.
    pub fn of(target: &Target) -> Option<Self> {
        let kind = target.common_kind()?;
        let found = Self::ALL
            .into_iter()
            .find(|candidate| candidate.object_kind() == kind)?;
        (!target.is_multi() || matches!(found, Self::Vm | Self::Template)).then_some(found)
    }

    pub const fn object_kind(self) -> ObjectKind {
        match self {
            Self::Host => ObjectKind::Host,
            Self::Pool => ObjectKind::Pool,
            Self::Vm => ObjectKind::Vm,
            Self::Template => ObjectKind::Template,
            Self::Storage => ObjectKind::StorageRepository,
            Self::Network => ObjectKind::Network,
            Self::Disk => ObjectKind::VirtualDisk,
        }
    }

    /// The id of the surface: `<kind>-properties`.
    pub const fn surface_id(self) -> &'static str {
        match self {
            Self::Host => "host-properties",
            Self::Pool => "pool-properties",
            Self::Vm => "vm-properties",
            Self::Template => "template-properties",
            Self::Storage => "storage-properties",
            Self::Network => "network-properties",
            Self::Disk => "disk-properties",
        }
    }

    /// The command that opens the dialog.
    pub const fn command(self) -> CommandId {
        match self {
            Self::Host => SERVER,
            Self::Pool => POOL,
            Self::Vm => VM,
            Self::Template => TEMPLATE,
            Self::Storage => STORAGE,
            Self::Network => NETWORK,
            Self::Disk => DISK,
        }
    }

    /// The pages the plan gives the dialog (docs/GPUI-MIGRATION.md,
    /// "Properties pages"), as `(id, label)` in the plan's order. The plan
    /// has no row for a template: it has the pages of a VM, as in the
    /// reference client.
    pub const fn planned(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Vm | Self::Template => &[
                ("general", "General"),
                ("custom-fields", "Custom Fields"),
                ("cpu-memory", "CPU and Memory"),
                ("boot-options", "Boot Options"),
                ("start-options", "Start Options"),
                ("alerts", "Alerts"),
                ("home-server", "Home Server"),
                ("advanced", "Advanced"),
            ],
            Self::Host => &[
                ("general", "General"),
                ("custom-fields", "Custom Fields"),
                ("alerts", "Alerts"),
                ("multipathing", "Multipathing"),
                ("log-destination", "Log Destination"),
                ("power-on", "Power On"),
            ],
            Self::Pool => &[
                ("general", "General"),
                ("custom-fields", "Custom Fields"),
                ("email-options", "Email Options"),
                ("network-options", "Network Options"),
                ("security", "Security"),
            ],
            Self::Storage => &[
                ("general", "General"),
                ("custom-fields", "Custom Fields"),
                ("alerts", "Alerts"),
                ("read-caching", "Read Caching"),
            ],
            Self::Network => &[
                ("general", "General"),
                ("custom-fields", "Custom Fields"),
                ("network-settings", "Network Settings"),
            ],
            Self::Disk => &[
                ("general", "General"),
                ("custom-fields", "Custom Fields"),
                ("size-location", "Size and Location"),
            ],
        }
    }
}

/// A page an area provides for a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyPage {
    /// Unique within the dialog of the target, across areas. The page list
    /// has the key `<surface>-pages` and the page `<surface>-pages-<id>`.
    pub id: &'static str,
    pub label: &'static str,
    /// The place in the page list: lowest first; equal orders keep the
    /// order of the areas.
    pub order: u16,
    /// The page is outside the supported core: while it is the current
    /// page the dialog carries the experimental notice.
    pub experimental: bool,
}

impl PropertyPage {
    /// The order of a page the plan does not list: after the planned ones.
    pub const UNPLANNED: u16 = 500;

    /// The one page of the dialog of several VMs or several templates (the
    /// plan's "Several VMs: Bulk properties").
    pub const BULK: Self = Self::new("bulk", "Bulk properties", 10);

    pub const fn new(id: &'static str, label: &'static str, order: u16) -> Self {
        Self {
            id,
            label,
            order,
            experimental: false,
        }
    }

    /// A page of the plan's table, by its id: the plan's label, and an
    /// order that puts it at the plan's position (10, 20, ..). An id the
    /// plan does not have gets itself as its label and comes last
    /// ([`Self::UNPLANNED`]); prefer [`Self::new`] for such a page.
    pub fn planned(kind: PropertiesKind, id: &'static str) -> Self {
        let position = kind
            .planned()
            .iter()
            .position(|(planned, _)| *planned == id);
        match position {
            Some(index) => Self::new(id, kind.planned()[index].1, (index as u16 + 1) * 10),
            None => Self::new(id, id, Self::UNPLANNED),
        }
    }

    pub const fn experimental(mut self, experimental: bool) -> Self {
        self.experimental = experimental;
        self
    }
}

/// What an area shows on its page that is the current one.
#[derive(Default)]
pub struct PageView {
    pub body: Vec<Block>,
    /// In order; the last is the default action. The area gives its own
    /// Close button when it wants one: Escape and the close button of the
    /// dialog close it in any case.
    pub footer: Vec<Button>,
    /// For example the permission assessment of a review.
    pub footer_note: Option<Text>,
}

impl PageView {
    pub fn new(body: Vec<Block>) -> Self {
        Self {
            body,
            ..Self::default()
        }
    }

    pub fn footer(mut self, buttons: impl IntoIterator<Item = Button>) -> Self {
        self.footer = buttons.into_iter().collect();
        self
    }

    pub fn footer_note(mut self, note: impl Into<Text>) -> Self {
        self.footer_note = Some(note.into());
        self
    }
}

/// What `on_property_page` tells an area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageEvent {
    /// The page with this id, one of this area's, became the current page:
    /// the dialog opened on it or the user turned to it. The area creates
    /// the draft of the page. Choosing the page that is already shown
    /// leaves and enters it again: the page starts afresh.
    Entered(&'static str),
    /// The page with this id stopped being the current page: another page
    /// was chosen (of this area or another), or the dialog is closing. An
    /// area whose draft belongs to the one page drops it here.
    Left(&'static str),
    /// The dialog closed. Told to EVERY area, after `Left` went to the area
    /// of the page that was showing. An area that keeps one draft across
    /// several of its pages drops it here.
    Closed,
}

/// The open Properties dialog. `m.properties`; read it freely, change it
/// only through [`Model::open_properties`], [`Model::show_properties_page`]
/// and [`Model::close_properties`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    /// What the dialog is about, as it was given to `open_properties`.
    pub target: Target,
    pub kind: PropertiesKind,
    /// The area of the current page.
    pub area: &'static str,
    /// The current page.
    pub page: PropertyPage,
}

/// The reason of a Properties command for an object no area has a page
/// for: its kind has none, or its pool has no inventory.
pub const NO_PAGES: &str = "This object has no properties that can be edited now.";

pub const SERVER: CommandId = CommandId("server.properties");
pub const POOL: CommandId = CommandId("pool.properties");
pub const VM: CommandId = CommandId("vm.properties");
pub const TEMPLATE: CommandId = CommandId("template.properties");
pub const STORAGE: CommandId = CommandId("storage.properties");
pub const NETWORK: CommandId = CommandId("network.properties");
pub const DISK: CommandId = CommandId("disk.properties");

/// The place of the Properties button in the local action row of the
/// General tab (the plan: "General: sections for the object, Properties
/// button"): last.
pub const LOCAL_ORDER: u8 = 200;

impl Model {
    /// The pages the areas provide for the Properties dialog of `target`,
    /// each with the id of its area, in the order of the page list. Of two
    /// pages with one id the first stays.
    pub fn property_pages(&self, target: &Target) -> Vec<(&'static str, PropertyPage)> {
        let mut pages: Vec<(&'static str, PropertyPage)> = Vec::new();
        for area in areas::specs() {
            // The kernel's own tests see the pages of their test area and
            // of one real area only, so that they do not depend on which
            // areas provide pages for a server.
            #[cfg(test)]
            if self.areas.pages.isolated && !matches!(area.id, "pages" | "host_admin") {
                continue;
            }
            for page in (area.property_pages)(self, target) {
                if !pages.iter().any(|(_, other)| other.id == page.id) {
                    pages.push((area.id, page));
                }
            }
        }
        // Stable: equal orders keep the order of the areas.
        pages.sort_by_key(|(_, page)| page.order);
        pages
    }

    /// Opens the Properties dialog of `target` on the page with this id, or
    /// on its first page. False, and nothing changes, when the target has
    /// no Properties dialog, no area provides a page for it, or the page
    /// asked for is not among them. A Properties dialog that is open is
    /// closed first, also when it is the one of the same target.
    pub fn open_properties(&mut self, target: &Target, page: Option<&str>) -> bool {
        let Some(kind) = PropertiesKind::of(target) else {
            return false;
        };
        let pages = self.property_pages(target);
        let chosen = match page {
            Some(id) => pages.iter().find(|(_, page)| page.id == id),
            None => pages.first(),
        };
        let Some(&(area, page)) = chosen else {
            return false;
        };
        self.close_properties();
        self.properties = Some(Open {
            target: target.clone(),
            kind,
            area,
            page,
        });
        tell(self, area, target, PageEvent::Entered(page.id));
        true
    }

    /// Turns the open Properties dialog to the page with this id: the
    /// current page is left, the new one entered. The page that is already
    /// shown is left and entered again. False when no dialog is open or its
    /// target has no such page.
    pub fn show_properties_page(&mut self, id: &str) -> bool {
        let Some(open) = &self.properties else {
            return false;
        };
        let pages = self.property_pages(&open.target);
        let Some(&(area, page)) = pages.iter().find(|(_, page)| page.id == id) else {
            return false;
        };
        // Taken while the area hears `Left`: a handler that closes the
        // dialog finds none.
        let Some(open) = self.properties.take() else {
            return false;
        };
        tell(self, open.area, &open.target, PageEvent::Left(open.page.id));
        let target = open.target.clone();
        self.properties = Some(Open { area, page, ..open });
        tell(self, area, &target, PageEvent::Entered(page.id));
        true
    }

    /// Closes the Properties dialog: the area of the current page hears
    /// `Left`, then every area hears `Closed`. Returns whether one was open.
    pub fn close_properties(&mut self) -> bool {
        let Some(open) = self.properties.take() else {
            return false;
        };
        tell(self, open.area, &open.target, PageEvent::Left(open.page.id));
        for area in areas::specs() {
            (area.on_property_page)(self, &open.target, PageEvent::Closed);
        }
        true
    }

    /// Runs a command from a page of the Properties dialog that gives way
    /// to what the command opens (a page's button into a workflow of
    /// another area). When the command can run, the dialog closes first and
    /// then it runs; when it is refused, the dialog stays as it is and the
    /// reason is in [`Model::error`]. Returns whether the command ran.
    pub fn invoke_from_properties(&mut self, id: CommandId, target: &Target) -> bool {
        if self.eligibility(id, target).is_enabled() {
            self.close_properties();
        }
        self.invoke(id, target)
    }
}

fn tell(m: &mut Model, area: &str, target: &Target, event: PageEvent) {
    if let Some(spec) = areas::area(area) {
        (spec.on_property_page)(m, target, event);
    }
}

/// The open dialog, described, with the id of the area whose surfaces it
/// stands with in the stack.
pub(crate) fn surface(m: &Model) -> Option<(&'static str, Surface)> {
    let open = m.properties.as_ref()?;
    let mut pages = m.property_pages(&open.target);
    // A page that was just withdrawn is shown until the next poll closes
    // the dialog: a description changes nothing.
    if !pages.iter().any(|(_, page)| page.id == open.page.id) {
        pages.push((open.area, open.page));
        pages.sort_by_key(|(_, page)| page.order);
    }
    let view = areas::area(open.area)
        .map(|spec| (spec.property_page)(m, &open.target, open.page.id))
        .unwrap_or_default();
    let mut surface = Surface::properties(
        open.kind.surface_id(),
        "Properties",
        pages.iter().map(|(_, page)| (page.id, page.label)),
        open.page.id,
        |m, id| {
            m.show_properties_page(id);
        },
    )
    .experimental(open.page.experimental)
    .body(view.body)
    .footer(view.footer)
    .on_close(|m| {
        m.close_properties();
    });
    if let Some(subtitle) = subtitle(m, &open.target) {
        surface = surface.subtitle(subtitle);
    }
    if let Some(note) = view.footer_note {
        surface = surface.footer_note(note);
    }
    Some((open.area, surface))
}

/// The name and UUID of the object, or how many objects.
fn subtitle(m: &Model, target: &Target) -> Option<String> {
    match target.single() {
        Some(object) => {
            let name = m
                .name_of(object)
                .unwrap_or_else(|| format!("Unknown {}", object.kind.label().to_lowercase()));
            Some(format!("{name} ({})", object.id))
        }
        None => {
            let noun = if target.all_of(ObjectKind::Template) {
                "templates"
            } else {
                "VMs"
            };
            Some(format!("{} {noun}", target.len()))
        }
    }
}

/// After the inventories changed: the dialog closes when an object of its
/// target is gone (or became another kind of object), and when the area no
/// longer offers the current page. Returns whether it closed.
pub(crate) fn poll(m: &mut Model) -> bool {
    let Some(open) = &m.properties else {
        return false;
    };
    let present = open
        .target
        .objects
        .iter()
        .all(|object| m.resolve(object).as_ref() == Some(object));
    let offered = present
        && m.property_pages(&open.target)
            .iter()
            .any(|(area, page)| *area == open.area && page.id == open.page.id);
    !offered && m.close_properties()
}

fn close(m: &mut Model) {
    m.close_properties();
}

/// The kernel's entry in [`areas::specs`], after every area: it is how the
/// kernel hears of a scope change and a session change, which both close
/// the Properties dialog. It has no state in `m.areas` and is not in
/// [`areas::AREA_IDS`].
pub(crate) fn spec() -> AreaSpec {
    AreaSpec {
        scenes,
        on_scope_change: close,
        on_session_change: close,
        ..AreaSpec::new(KERNEL)
    }
}

/// The scene of the dialog itself, which no Rune scene shows (Rune had no
/// Properties dialog of a server): the server of the `host_admin` scenes
/// with its Properties dialog open on the first page.
fn scenes() -> Vec<SceneSpec> {
    vec![SceneSpec::new("host-properties", KERNEL, |m| {
        if let Some(target) = areas::host_admin::scenes::prepare(m) {
            m.invoke(SERVER, &target);
        }
    })]
}

fn eligibility(m: &Model, target: &Target, kind: PropertiesKind) -> Eligibility {
    if PropertiesKind::of(target) != Some(kind) {
        return Eligibility::Hidden;
    }
    Eligibility::enabled_if(!m.property_pages(target).is_empty(), NO_PAGES)
}

fn open(m: &mut Model, target: &Target, kind: PropertiesKind) {
    if PropertiesKind::of(target) == Some(kind) {
        m.open_properties(target, None);
    }
}

/// A Properties command: hidden for another kind of target, disabled with
/// [`NO_PAGES`] when no area provides a page, and in the local action row
/// of the General tab.
macro_rules! properties_command {
    ($kind:expr) => {
        CommandSpec::new($kind.command().0, "Properties", KERNEL)
            .local(TabId::General, LOCAL_ORDER)
            .local_scopes(const { &[$kind.object_kind()] })
            .eligibility(|m, target| eligibility(m, target, $kind))
            .invoke(|m, target| open(m, target, $kind))
    };
}

/// The seven Properties commands, in menu order. The Server, Pool, VM,
/// Storage and Templates menus have the item (the plan's Menus table); a
/// network and a virtual disk have no menu, so theirs are in the context
/// menu and on the General tab only. `vm.properties` and
/// `template.properties` accept several objects.
pub(crate) fn commands() -> Vec<CommandSpec> {
    use PropertiesKind as Kind;
    vec![
        properties_command!(Kind::Pool)
            .menu(place::POOL_PROPERTIES)
            .context(&[ObjectKind::Pool]),
        properties_command!(Kind::Host)
            .menu(place::SERVER_PROPERTIES)
            .context(&[ObjectKind::Host]),
        properties_command!(Kind::Vm)
            .menu(place::VM_PROPERTIES)
            .context(&[ObjectKind::Vm])
            .multi(),
        properties_command!(Kind::Storage)
            .menu(place::STORAGE_PROPERTIES)
            .context(&[ObjectKind::StorageRepository]),
        properties_command!(Kind::Template)
            .menu(place::TEMPLATES_PROPERTIES)
            .context(&[ObjectKind::Template])
            .multi(),
        properties_command!(Kind::Network).context(&[ObjectKind::Network]),
        properties_command!(Kind::Disk).context(&[ObjectKind::VirtualDisk]),
    ]
}

#[cfg(test)]
#[path = "properties_tests.rs"]
pub(crate) mod tests;
