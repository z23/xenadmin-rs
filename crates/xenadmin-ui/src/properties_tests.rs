//! Tests of the Properties dialog, of the small kernel additions that came
//! with it (`on_nav_change`, the named places, `is_synthetic_pool`, the
//! prelude), and the check that every command id the sources name is
//! registered.
//!
//! The dialog needs pages from more than one area to be tested, and only
//! `host_admin` provides real ones so far. So this file is also a test-only
//! area, `pages` (the last entry of `areas::AREAS`), which provides pages
//! while `m.areas.pages.on` is set and writes down what it is told. It is
//! off by default: the tests of the other areas see only real pages.

use std::path::Path;
use std::sync::Arc;

use xenadmin_core::HostId;

use crate::Harness;
use crate::command;
use crate::prelude::*;
// What the prelude gained with this round; used below.
use crate::prelude::{EXPERIMENTAL_NOTICE, EXPERIMENTAL_NOTICE_KEY, resource_target};
use crate::properties::{self, NO_PAGES};

// -- The test area --------------------------------------------------------------------

#[derive(Default)]
pub struct State {
    /// Whether the area provides its pages.
    pub on: bool,
    /// Whether the kernel collects pages from this area and `host_admin`
    /// only (`Model::property_pages`): the tests of the dialog itself.
    pub isolated: bool,
    /// Every event of `on_property_page` and every `on_nav_change`.
    pub log: Vec<String>,
    /// One draft across the pages of the area, the way an area with
    /// several pages and one review keeps it: created when a page is
    /// entered, kept when it is left, dropped when the dialog closes.
    pub draft: Option<String>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        property_pages: pages,
        property_page: page,
        on_property_page: event,
        on_nav_change: |m| {
            let view = m.nav.view;
            m.areas.pages.log.push(format!("nav {view:?}"));
        },
        ..AreaSpec::new("pages")
    }
}

const NOTES: PropertyPage = PropertyPage::new("notes", "Notes", PropertyPage::UNPLANNED);

fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    if !m.areas.pages.on {
        return Vec::new();
    }
    match PropertiesKind::of(target) {
        // Given out of order: the kernel sorts.
        Some(PropertiesKind::Host) => vec![
            NOTES,
            PropertyPage::planned(PropertiesKind::Host, "alerts").experimental(true),
            PropertyPage::planned(PropertiesKind::Host, "general"),
            // Another area has a page with this id: the first stays.
            PropertyPage::new("multipathing", "Not this one", 5),
        ],
        Some(PropertiesKind::Vm) if target.is_multi() => vec![PropertyPage::BULK],
        Some(PropertiesKind::Vm) => vec![PropertyPage::planned(PropertiesKind::Vm, "general")],
        _ => Vec::new(),
    }
}

fn page(m: &Model, _: &Target, id: &str) -> PageView {
    let draft = m.areas.pages.draft.as_deref().unwrap_or_default();
    let view = PageView::new(blocks![
        text(format!("The page {id}")).key("pages-text"),
        input("pages-draft", "Draft", draft, |m, text| {
            m.areas.pages.draft = Some(text.to_owned());
        }),
    ])
    .footer([button("pages-close", "Close", |m| {
        m.close_properties();
    })]);
    if id == "general" {
        view.footer_note("A note of the page")
    } else {
        view
    }
}

fn event(m: &mut Model, _: &Target, event: PageEvent) {
    let state = &mut m.areas.pages;
    state.log.push(format!("{event:?}"));
    match event {
        PageEvent::Entered(_) => {
            state.draft.get_or_insert_default();
        }
        PageEvent::Left(_) => {}
        PageEvent::Closed => state.draft = None,
    }
}

// -- Fixtures -------------------------------------------------------------------------

/// The server of the `host_admin` scenes, selected, nothing open, and the
/// test area providing its pages.
fn server() -> (Harness, Target) {
    let mut h = Harness::scene("host-administration");
    h.dismiss();
    assert!(h.surfaces().is_empty());
    h.model.areas.pages.on = true;
    h.model.areas.pages.isolated = true;
    h.model.areas.pages.log.clear();
    let target = h.model.scope().clone();
    assert_eq!(target.kind(), ObjectKind::Host);
    (h, target)
}

fn log(h: &Harness) -> Vec<&str> {
    h.model.areas.pages.log.iter().map(String::as_str).collect()
}

fn page_labels(surface: &Surface) -> Vec<String> {
    let SurfaceKind::Properties { pages, .. } = &surface.kind else {
        panic!("{} is not a properties dialog", surface.id);
    };
    pages.iter().map(|page| page.label.clone()).collect()
}

fn current_page(surface: &Surface) -> String {
    let SurfaceKind::Properties { current, .. } = &surface.kind else {
        panic!("{} is not a properties dialog", surface.id);
    };
    current.clone()
}

fn open(h: &Harness) -> &properties::Open {
    h.model.properties.as_ref().expect("a Properties dialog")
}

fn vm_targets(m: &Model, count: usize) -> Vec<ObjectRef> {
    let inventory = m.inventory.clone().expect("inventory");
    inventory
        .vms
        .values()
        .filter(|vm| !vm.is_template && !vm.is_control_domain)
        .take(count)
        .map(|vm| ObjectRef::vm(inventory.pool_id, vm.id))
        .collect()
}

// -- 1. The dialog ----------------------------------------------------------------------

#[test]
fn the_dialog_has_the_pages_of_several_areas_in_the_plans_order() {
    let (mut h, target) = server();
    h.invoke("server.properties");
    let surface = h.surface();
    assert_eq!(surface.id, "host-properties");
    assert_eq!(surface.title, "Properties");
    assert_eq!(surface.size, SurfaceSize::Large);
    // General and Alerts of the test area, the three of `host_admin`, and
    // the page the plan does not list last. The second "multipathing" is
    // dropped.
    assert_eq!(
        page_labels(&surface),
        [
            "General",
            "Alerts",
            "Multipathing",
            "Log Destination",
            "Power On",
            "Notes"
        ]
    );
    // It opens on its first page.
    assert_eq!(current_page(&surface), "general");
    let name = h.model.name_of(target.single().unwrap()).unwrap();
    let uuid = &target.single().unwrap().id;
    assert_eq!(surface.subtitle, Some(format!("{name} ({uuid})")));
    assert_eq!(
        (open(&h).area, open(&h).kind, &open(&h).target),
        ("pages", PropertiesKind::Host, &target)
    );
    // Body, footer and footer note are the page's.
    assert_eq!(
        h.node("pages-text").text.as_deref(),
        Some("The page general")
    );
    assert_eq!(
        surface.footer.iter().map(|b| &b.key).collect::<Vec<_>>(),
        ["pages-close"]
    );
    assert_eq!(
        surface.footer_note.map(|note| note.text),
        Some("A note of the page".to_owned())
    );
    assert_eq!(h.node("host-properties-pages-general").checked, Some(true));
    h.assert_unique_keys();
    assert_eq!(log(&h), ["Entered(\"general\")"]);
}

#[test]
fn turning_the_page_leaves_one_area_and_enters_the_other() {
    let (mut h, _) = server();
    h.invoke("server.properties");
    h.type_text("pages-draft", "kept");
    // To a page of another area: this one hears `Left` and keeps its draft,
    // the other creates its editor.
    h.pick("host-properties-pages", "log-destination");
    assert_eq!(log(&h), ["Entered(\"general\")", "Left(\"general\")"]);
    assert_eq!(h.model.areas.pages.draft.as_deref(), Some("kept"));
    assert_eq!(open(&h).area, "host_admin");
    assert!(h.model.areas.host_admin.editor.is_some());
    assert!(h.find("host-setting-value").is_some());
    assert!(h.find("pages-text").is_none(), "only the current page");
    assert!(h.surface().footer_note.is_none());
    // And back: the editor of the page that was left is gone.
    h.pick("host-properties-pages", "alerts");
    assert!(h.model.areas.host_admin.editor.is_none());
    assert_eq!(h.node("pages-draft").value.as_deref(), Some("kept"));
    assert_eq!(
        log(&h)[2..],
        ["Entered(\"alerts\")"],
        "nothing is told to an area about another area's page"
    );
    // The page that is shown, chosen again, starts afresh.
    h.pick("host-properties-pages", "alerts");
    assert_eq!(log(&h)[3..], ["Left(\"alerts\")", "Entered(\"alerts\")"]);
    // Closing: `Left` for the current page, then `Closed` for every area.
    h.dismiss();
    assert_eq!(log(&h)[5..], ["Left(\"alerts\")", "Closed"]);
    assert!(h.model.areas.pages.draft.is_none());
    assert!(h.model.properties.is_none());
    assert!(h.surfaces().is_empty());
    assert!(!h.model.close_properties(), "nothing was open");
}

#[test]
fn the_experimental_notice_follows_the_current_page() {
    let (mut h, _) = server();
    h.invoke("server.properties");
    assert!(!h.surface().experimental, "General is supported");
    assert!(!h.model.experimental_notice());
    assert!(h.find(EXPERIMENTAL_NOTICE_KEY).is_none());
    h.pick("host-properties-pages", "alerts");
    assert!(h.model.experimental_notice());
    assert_eq!(
        h.node(EXPERIMENTAL_NOTICE_KEY).text.as_deref(),
        Some(EXPERIMENTAL_NOTICE)
    );
    h.pick("host-properties-pages", "power-on");
    assert!(h.model.experimental_notice(), "host_admin's pages are too");
    h.pick("host-properties-pages", "notes");
    assert!(!h.model.experimental_notice());
}

#[test]
fn open_properties_takes_a_page_and_refuses_what_is_not_there() {
    let (mut h, target) = server();
    assert!(h.model.open_properties(&target, Some("power-on")));
    assert_eq!(current_page(&h.surface()), "power-on");
    assert_eq!(open(&h).area, "host_admin");
    // A page the target does not have: nothing changes.
    assert!(!h.model.open_properties(&target, Some("no-such-page")));
    assert_eq!(current_page(&h.surface()), "power-on");
    assert!(!h.model.show_properties_page("no-such-page"));
    assert_eq!(current_page(&h.surface()), "power-on");
    assert!(h.model.show_properties_page("notes"));
    assert_eq!(current_page(&h.surface()), "notes");
    // Opening again closes the open one first, also for the same object.
    h.model.areas.pages.log.clear();
    assert!(h.model.open_properties(&target, None));
    assert_eq!(
        log(&h),
        ["Left(\"notes\")", "Closed", "Entered(\"general\")"]
    );
    h.dismiss();
    // No dialog: no page to show. No such dialog: nothing opens.
    assert!(!h.model.show_properties_page("general"));
    assert!(!h.model.open_properties(&Target::none(), None));
    let folder = Target::one(ObjectRef::new(None, ObjectKind::Folder, "/Production"));
    assert!(!h.model.open_properties(&folder, None));
    // A server no area has a page for.
    let pool = target.pool().expect("pool");
    let stranger = Target::one(ObjectRef::host(pool, HostId::from(uuid::Uuid::new_v4())));
    h.model.areas.pages.on = false;
    assert!(h.model.property_pages(&stranger).is_empty());
    assert!(!h.model.open_properties(&stranger, None));
    assert!(h.model.properties.is_none());
}

#[test]
fn the_dialog_stands_with_the_surfaces_of_the_area_of_its_current_page() {
    let (mut h, _) = server();
    h.invoke("server.properties");
    h.pick("host-properties-pages", "multipathing");
    // The probe is a later area than `host_admin` and an earlier one than
    // the test area.
    h.model.areas.probe.open = true;
    let ids = |h: &Harness| -> Vec<String> { h.surfaces().into_iter().map(|s| s.id).collect() };
    assert_eq!(ids(&h), ["host-properties", "probe"]);
    assert!(h.model.show_properties_page("general"));
    assert_eq!(ids(&h), ["probe", "host-properties"]);
    assert_eq!(
        h.surface().id,
        "host-properties",
        "in front: it takes input"
    );
}

// -- 2. Closing rules -------------------------------------------------------------------

#[test]
fn the_dialog_closes_when_the_scope_or_the_session_changes() {
    let (mut h, target) = server();
    h.invoke("server.properties");
    h.pick("host-properties-pages", "multipathing");
    let pool = target.pool().expect("pool");
    // Another object selected in the tree.
    h.select(ObjectRef::pool(pool));
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.host_admin.editor.is_none());
    assert_eq!(log(&h).last(), Some(&"Closed"));
    // The view of the session is dropped (a disconnect, another pool).
    h.select(target.clone());
    h.invoke("server.properties");
    assert!(h.model.properties.is_some());
    crate::session::clear_view(&mut h.model);
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.pages.draft.is_none());
}

/// The kernel hears of a session change through its own entry at the end
/// of `areas::specs()` (`properties::spec`, id `kernel`): it is not in
/// `areas::AREAS`. Whoever runs the session hooks over the bare list skips
/// it, and the Properties dialog of the previous session stays open over
/// another pool. Each place in `session.rs` that runs the hooks is driven
/// here without a scope change (which would close the dialog by itself),
/// and the source is read so that a new place cannot go unnoticed.
#[test]
fn every_session_change_reaches_the_kernels_entry_after_the_areas() {
    // The kernel's entry is the last spec and is not an area.
    let specs = crate::areas::specs();
    assert_eq!(specs.len(), crate::areas::AREAS.len() + 1);
    assert_eq!(specs.last().map(|spec| spec.id), Some("kernel"));
    assert!(!crate::areas::AREA_IDS.contains(&"kernel"));

    let open = || {
        let (mut h, target) = server();
        h.invoke("server.properties");
        h.pick("host-properties-pages", "multipathing");
        assert!(h.model.properties.is_some());
        h.model.areas.probe.hooks.clear();
        (h, target)
    };
    type Change = fn(&mut Model);
    let changes: [(&str, Change); 2] = [
        ("clear_view", crate::session::clear_view),
        ("disconnect_selected", crate::session::disconnect_selected),
    ];
    for (name, change) in changes {
        let (mut h, target) = open();
        change(&mut h.model);
        // The areas heard of it, the scope did not change, and the
        // kernel's entry closed the dialog and told the page's area.
        assert_eq!(h.model.areas.probe.hooks, ["session"], "{name}");
        assert_eq!(h.model.scope(), &target, "{name}");
        assert!(h.model.properties.is_none(), "{name}: the dialog is open");
        assert_eq!(log(&h).last(), Some(&"Closed"), "{name}");
        assert!(h.model.areas.host_admin.editor.is_none(), "{name}");
    }

    // The source: every run of the session hooks is a loop over
    // `areas::specs()`, and there are exactly the two places driven above.
    let source = include_str!("session.rs");
    let code: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .collect();
    let runs: Vec<usize> = code
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("on_session_change)("))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        runs.len(),
        changes.len(),
        "session.rs runs the session hooks in another number of places: drive each one above"
    );
    for index in runs {
        assert_eq!(
            code[index - 1],
            "for area in crate::areas::specs() {",
            "the session hooks must run over areas::specs(), which ends with the kernel's entry"
        );
    }
    assert!(
        !code.iter().any(|line| line.contains("AREAS")),
        "session.rs must not iterate the bare list of areas"
    );
}

#[test]
fn the_dialog_closes_when_its_object_is_gone_or_its_page_is_withdrawn() {
    let (mut h, target) = server();
    let pool = target.pool().expect("pool");
    let host = target.single().and_then(ObjectRef::host_id).expect("host");
    // Opened on a row of a grid: the tree scope is the pool and stays.
    h.select(ObjectRef::pool(pool));
    h.model.select_in_grid(target.clone());
    let row = h.model.target();
    assert!(h.model.open_properties(&row, Some("multipathing")));
    assert!(!h.tick(0.1), "nothing changed");
    assert!(h.model.properties.is_some());
    let mut inventory = h
        .model
        .inventory
        .take()
        .expect("inventory")
        .as_ref()
        .clone();
    inventory.hosts.remove(&host);
    h.model.inventory = Some(Arc::new(inventory));
    assert!(h.tick(0.1));
    assert!(h.model.properties.is_none());
    assert!(h.model.areas.host_admin.editor.is_none());
    assert_eq!(h.model.scope(), &Target::one(ObjectRef::pool(pool)));

    // The area stops offering the page that is showing.
    let (mut h, _) = server();
    h.invoke("server.properties");
    h.model.areas.pages.on = false;
    // Until the next poll the page is still listed and shown.
    assert_eq!(current_page(&h.surface()), "general");
    assert!(page_labels(&h.surface()).contains(&"General".to_owned()));
    assert!(h.tick(0.1));
    assert!(h.model.properties.is_none());
    // A page of another area that is still offered keeps the dialog.
    h.model.areas.pages.on = true;
    h.invoke("server.properties");
    h.pick("host-properties-pages", "power-on");
    h.model.areas.pages.on = false;
    assert!(!h.tick(0.1));
    assert_eq!(
        page_labels(&h.surface()),
        ["Multipathing", "Log Destination", "Power On"]
    );
}

#[test]
fn a_page_gives_way_to_a_command_only_when_the_command_can_run() {
    let (mut h, target) = server();
    h.invoke("server.properties");
    h.pick("host-properties-pages", "multipathing");
    // Refused: the dialog stays, the reason is in the error line.
    let reboot = CommandId("server.reboot");
    assert!(!h.model.invoke_from_properties(reboot, &target));
    assert_eq!(h.surface().id, "host-properties");
    assert!(h.model.areas.host_admin.editor.is_some());
    assert!(h.model.error.is_some());
    // Possible: the dialog closes first, then the command opens its own.
    let memory = CommandId("server.control-domain-memory");
    assert!(h.model.invoke_from_properties(memory, &target));
    assert!(h.model.properties.is_none());
    assert_eq!(h.surfaces().len(), 1);
    assert_eq!(h.surface().id, "host-dom0-memory");
}

// -- 3. The commands --------------------------------------------------------------------

#[test]
fn the_properties_commands_are_placed_as_the_plan_says() {
    let expected: [(PropertiesKind, &str, &str, Option<MenuPath>, bool); 7] = [
        (
            PropertiesKind::Host,
            "server.properties",
            "host-properties",
            Some(place::SERVER_PROPERTIES),
            false,
        ),
        (
            PropertiesKind::Pool,
            "pool.properties",
            "pool-properties",
            Some(place::POOL_PROPERTIES),
            false,
        ),
        (
            PropertiesKind::Vm,
            "vm.properties",
            "vm-properties",
            Some(place::VM_PROPERTIES),
            true,
        ),
        (
            PropertiesKind::Template,
            "template.properties",
            "template-properties",
            Some(place::TEMPLATES_PROPERTIES),
            true,
        ),
        (
            PropertiesKind::Storage,
            "storage.properties",
            "storage-properties",
            Some(place::STORAGE_PROPERTIES),
            false,
        ),
        (
            PropertiesKind::Network,
            "network.properties",
            "network-properties",
            None,
            false,
        ),
        (
            PropertiesKind::Disk,
            "disk.properties",
            "disk-properties",
            None,
            false,
        ),
    ];
    assert_eq!(expected.map(|row| row.0), PropertiesKind::ALL);
    for (kind, id, surface, menu, multi) in expected {
        assert_eq!(kind.command().0, id);
        assert_eq!(kind.surface_id(), surface);
        let spec = command::find(kind.command()).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(spec.label, "Properties");
        assert_eq!(spec.area, command::KERNEL);
        assert_eq!(spec.menu, menu, "{id}");
        assert_eq!(spec.context, [kind.object_kind()], "{id}");
        assert_eq!(
            spec.local,
            Some(LocalRow::new(TabId::General, properties::LOCAL_ORDER)),
            "{id}"
        );
        assert_eq!(spec.multi, multi, "{id}");
    }
    // No area registers a Properties command of an object of its own.
    for spec in command::registry() {
        let ours = PropertiesKind::ALL
            .iter()
            .any(|kind| kind.command() == spec.id);
        if ours {
            assert_eq!(spec.area, command::KERNEL, "{}", spec.id);
        }
    }
}

#[test]
fn a_properties_command_is_hidden_for_another_kind_and_disabled_without_a_page() {
    let (mut h, host) = server();
    let pool = host.pool().expect("pool");
    let inventory = h.model.inventory.clone().expect("inventory");
    let vms = vm_targets(&h.model, 2);
    let mut targets = vec![
        host.clone(),
        Target::one(ObjectRef::pool(pool)),
        Target::one(vms[0].clone()),
        Target::many(vms.clone()),
    ];
    if let Some(storage) = inventory.storage.first() {
        targets.push(Target::one(ObjectRef::storage(pool, storage.uuid)));
    }
    if let Some(network) = inventory.networks.first() {
        targets.push(Target::one(ObjectRef::network(pool, network.uuid)));
    }
    for on in [true, false] {
        h.model.areas.pages.on = on;
        for target in &targets {
            let kind = PropertiesKind::of(target).expect("a kind with Properties");
            for other in PropertiesKind::ALL {
                let expected = if other != kind {
                    Eligibility::Hidden
                } else {
                    Eligibility::enabled_if(!h.model.property_pages(target).is_empty(), NO_PAGES)
                };
                assert_eq!(
                    h.model.eligibility(other.command(), target),
                    expected,
                    "{} on {:?}",
                    other.command(),
                    target.kind()
                );
            }
        }
    }
    // Targets without a Properties dialog: every command is hidden.
    let two_servers = Target::many(
        inventory
            .hosts
            .keys()
            .take(2)
            .map(|id| ObjectRef::host(pool, *id)),
    );
    let mixed = Target::many([vms[0].clone(), ObjectRef::pool(pool)]);
    for target in [&two_servers, &mixed, &Target::none()] {
        assert_eq!(PropertiesKind::of(target), None);
        for kind in PropertiesKind::ALL {
            assert_eq!(
                h.model.eligibility(kind.command(), target),
                Eligibility::Hidden
            );
        }
    }
    // The right kind, and no area with a page: disabled, with the reason.
    let stranger = Target::one(ObjectRef::host(pool, HostId::from(uuid::Uuid::new_v4())));
    assert_eq!(
        h.model.eligibility(properties::SERVER, &stranger),
        Eligibility::disabled(NO_PAGES)
    );
    assert_eq!(
        h.try_invoke_on("server.properties", &stranger),
        Err(NO_PAGES.into())
    );
    assert!(h.model.properties.is_none());
    // The item is in the context menu, disabled.
    let item = h
        .context_menu(&stranger)
        .into_iter()
        .find(|item| item.id == properties::SERVER)
        .expect("in the context menu");
    assert_eq!(item.disabled.as_deref(), Some(NO_PAGES));
}

#[test]
fn several_vms_get_the_dialog_of_a_vm_with_the_pages_for_the_selection() {
    let (mut h, _) = server();
    let vms = Target::many(vm_targets(&h.model, 2));
    assert_eq!(PropertiesKind::of(&vms), Some(PropertiesKind::Vm));
    // The whole target goes to the areas: this one answers a
    // multi-selection with the bulk page, as `bulk_edit` will.
    assert!(
        h.context_menu(&vms)
            .iter()
            .any(|item| item.id == properties::VM && item.disabled.is_none())
    );
    h.invoke_on("vm.properties", &vms);
    let surface = h.surface();
    assert_eq!(surface.id, "vm-properties");
    assert_eq!(page_labels(&surface), ["Bulk properties"]);
    assert_eq!(surface.subtitle.as_deref(), Some("2 VMs"));
    assert_eq!(open(&h).target, vms);
    h.dismiss();
    let one = Target::one(vms.objects[0].clone());
    h.invoke_on("vm.properties", &one);
    // One VM has the pages for one VM (this area's and those of the real
    // areas), never the page of a multi-selection.
    let labels = page_labels(&h.surface());
    assert!(labels.iter().any(|label| label == "General"), "{labels:?}");
    assert!(
        !labels.iter().any(|label| label == "Bulk properties"),
        "{labels:?}"
    );
}

#[test]
fn the_general_tab_has_the_properties_button_of_the_scope() {
    let (mut h, host) = server();
    h.open_tab(TabId::General);
    let buttons = |h: &Harness| -> Vec<CommandId> {
        h.local_row()
            .into_iter()
            .filter(|item| item.label == "Properties")
            .map(|item| item.id)
            .collect()
    };
    assert_eq!(buttons(&h), [properties::SERVER]);
    h.click("server.properties");
    assert_eq!(h.surface().id, "host-properties");
    assert_eq!(open(&h).target, host);
    h.dismiss();
    let vm = vm_targets(&h.model, 1).remove(0);
    h.select(vm);
    h.open_tab(TabId::General);
    assert_eq!(buttons(&h), [properties::VM]);
}

#[test]
fn the_pages_of_the_areas_are_pages_of_the_plan() {
    // The plan's table, as the kernel holds it.
    for kind in PropertiesKind::ALL {
        let planned = kind.planned();
        assert_eq!(planned[0], ("general", "General"));
        assert_eq!(planned[1], ("custom-fields", "Custom Fields"));
        let mut ids: Vec<_> = planned.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), planned.len(), "{kind:?}");
    }
    let power_on = PropertyPage::planned(PropertiesKind::Host, "power-on");
    assert_eq!((power_on.label, power_on.order), ("Power On", 60));
    assert!(!power_on.experimental);
    let alerts = PropertyPage::planned(PropertiesKind::Vm, "alerts");
    assert_eq!((alerts.label, alerts.order), ("Alerts", 60));
    assert_eq!(
        PropertyPage::planned(PropertiesKind::Host, "alerts").order,
        30
    );
    let stray = PropertyPage::planned(PropertiesKind::Pool, "power-on");
    assert_eq!(
        (stray.label, stray.order),
        ("power-on", PropertyPage::UNPLANNED)
    );
    // What the real areas provide for the objects of the demo follows it.
    let (mut h, host) = server();
    h.model.areas.pages.on = false;
    h.model.areas.pages.isolated = false;
    let pool = host.pool().expect("pool");
    let mut targets = vec![host, Target::one(ObjectRef::pool(pool))];
    targets.extend(vm_targets(&h.model, 1).into_iter().map(Target::one));
    for target in targets {
        let kind = PropertiesKind::of(&target).expect("kind");
        for (area, page) in h.model.property_pages(&target) {
            // A page the plan does not have (the one review of a VM's
            // settings, kept as the last page of its dialog) comes last.
            if !kind.planned().iter().any(|(id, _)| *id == page.id) {
                assert_eq!(page.order, PropertyPage::UNPLANNED, "{area}: {}", page.id);
                continue;
            }
            let planned = PropertyPage::planned(kind, page.id);
            assert_eq!(
                (page.label, page.order),
                (planned.label, planned.order),
                "{area}: {}",
                page.id
            );
        }
    }
}

// -- 4. The small additions -------------------------------------------------------------

#[test]
fn every_area_hears_of_a_change_of_the_navigation_view() {
    let mut h = Harness::with(Model::demo(3, 48));
    let scope = h.model.scope().clone();
    h.open_view(NavView::Objects);
    assert_eq!(log(&h), ["nav Objects"]);
    // The same view again is no change.
    h.model.set_nav_view(NavView::Objects);
    assert_eq!(log(&h), ["nav Objects"]);
    h.model.set_nav_view(NavView::Infrastructure);
    assert_eq!(log(&h), ["nav Objects", "nav Infrastructure"]);
    assert_eq!(h.model.scope(), &scope, "the selection stays");
}

#[test]
fn the_client_only_items_of_host_admin_have_named_places() {
    // Power On is between Reboot and Shut Down, as the plan lists it.
    let between = |place: MenuPath, before: MenuPath, after: MenuPath| {
        (place.menu, place.group) == (before.menu, before.group)
            && (place.menu, place.group) == (after.menu, after.group)
            && before.order < place.order
            && place.order < after.order
    };
    assert!(between(
        place::SERVER_POWER_ON,
        place::SERVER_REBOOT,
        place::SERVER_SHUT_DOWN
    ));
    assert!(between(
        place::SERVER_EVACUATE,
        place::SERVER_MAINTENANCE,
        place::SERVER_CONTROL_DOMAIN_MEMORY
    ));
    let autostart = place::POOL_VM_AUTOSTART;
    let schedules = place::POOL_SNAPSHOT_SCHEDULES;
    assert_eq!(
        (autostart.menu, autostart.group),
        (schedules.menu, schedules.group)
    );
    assert!(autostart.order > schedules.order);
    for (id, place) in [
        ("server.power-on", place::SERVER_POWER_ON),
        ("server.evacuate", place::SERVER_EVACUATE),
        ("pool.vm-autostart", place::POOL_VM_AUTOSTART),
    ] {
        let spec = command::registry()
            .iter()
            .find(|spec| spec.id.0 == id)
            .unwrap_or_else(|| panic!("{id}"));
        assert_eq!(spec.menu, Some(place), "{id}");
    }
}

#[test]
fn a_pool_is_synthetic_by_its_own_session() {
    let mut m = Model::demo(3, 48);
    let first = m.inventory.clone().expect("inventory");
    let mut second = first.as_ref().clone();
    second.pool_id =
        xenadmin_core::PoolId::parse("10000000-0000-0000-0000-000000000077").expect("pool id");
    let other = second.pool_id;
    m.add_synthetic_pool("Second", second);
    assert!(m.is_synthetic_pool(first.pool_id));
    assert!(m.is_synthetic_pool(other));
    // For the scope session `demo` decides, as for `can_mutate`.
    m.demo = false;
    assert!(!m.is_synthetic_pool(first.pool_id));
    assert!(m.is_synthetic_pool(other));
    // A pool no session shows.
    let nowhere =
        xenadmin_core::PoolId::parse("10000000-0000-0000-0000-000000000078").expect("pool id");
    assert!(!m.is_synthetic_pool(nowhere));
    // And the prelude has `resource_target`.
    assert!(resource_target(&ObjectRef::pool(other)).is_some());
}

// -- 5. Command ids in the sources ---------------------------------------------------

/// Commands of areas that are not ported yet, which ported areas already
/// lead to (a button that is absent until the command is registered). An
/// id leaves this list when its area lands; the list must be empty at the
/// end of the migration.
const PENDING: &[&str] = &[
    // host_admin, page Power On: host_services.
    "server.power-on-settings",
    // migration, the second step of the wizard: cross_pool.
    "vm.move-to-pool",
];

/// Ids the tests use for a command that does not exist, on purpose.
const NOT_COMMANDS: &[&str] = &[
    "missing",
    "probe.missing",
    "no.such-command",
    "no.such.command",
];

/// The calls whose first argument, when it is a string literal, is a
/// command id. (Written without their parenthesis, so that this file does
/// not find its own list.)
const CALLS: &[&str] = &[
    "CommandId",
    ".invoke",
    ".try_invoke",
    ".invoke_on",
    ".try_invoke_on",
    ".invoke_context",
    ".invoke_from_properties",
    ".eligibility",
    ".menu_item",
    ".choose",
    ".try_choose",
];

fn sources(dir: &Path, found: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("source directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = std::fs::read_to_string(&path).expect("source file");
            found.push((path.display().to_string(), text));
        }
    }
}

/// The string literals that are the first argument of one of [`CALLS`].
/// Comment lines are skipped; a literal on the line after the parenthesis
/// (as rustfmt breaks a long call) is found.
fn command_literals(source: &str) -> Vec<String> {
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = Vec::new();
    for call in CALLS {
        let needle = format!("{call}(");
        let mut rest = code.as_str();
        while let Some(at) = rest.find(&needle) {
            rest = &rest[at + needle.len()..];
            let Some(literal) = rest.trim_start().strip_prefix('"') else {
                continue;
            };
            if let Some(end) = literal.find('"') {
                found.push(literal[..end].to_owned());
            }
        }
    }
    found
}

#[test]
fn every_command_id_the_sources_name_is_registered() {
    let sample =
        "m.invoke(CommandId(\n    \"a.b\",\n), &t);\n// h.invoke(\"e.f\")\nh.invoke(\"c.d\");";
    assert_eq!(command_literals(sample), ["a.b", "c.d"]);
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 40, "the sources were not found");
    let registered: Vec<&str> = command::registry().iter().map(|spec| spec.id.0).collect();
    let mut named: Vec<(String, String)> = Vec::new();
    for (path, text) in &files {
        for id in command_literals(text) {
            named.push((id, path.clone()));
        }
    }
    assert!(named.len() > 50, "no command ids were found");
    let unknown: Vec<_> = named
        .iter()
        .filter(|(id, _)| {
            !registered.contains(&id.as_str())
                && !PENDING.contains(&id.as_str())
                && !NOT_COMMANDS.contains(&id.as_str())
        })
        .collect();
    assert!(
        unknown.is_empty(),
        "command ids that are not registered (a typo, or the command of an area that is not \
         ported yet, which belongs in PENDING): {unknown:#?}"
    );
    // The lists do not rot: a pending id is still named and still missing.
    for id in PENDING {
        assert!(
            !registered.contains(id),
            "{id} is registered now: take it out of PENDING"
        );
        assert!(
            named.iter().any(|(named, _)| named == id),
            "{id} is not named any more: take it out of PENDING"
        );
    }
    for id in NOT_COMMANDS {
        assert!(!registered.contains(id), "{id}");
    }
}
