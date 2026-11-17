//! The scene tool: any scene as a PNG and as an element tree, without a
//! display.
//!
//! A scene is a model set up by a function. For a registered scene name
//! (`xenadmin_ui::scene::registry()`) the tool builds `Model::fixture()`,
//! applies the scene and renders what the model then describes. The
//! `gallery-*` names are the renderer's own fixtures ([`gallery`]).
//!
//! A scene is rendered as the whole window: the shell (title and menu bar,
//! toolbar, navigation pane, status bar) around the content of the scene.
//! [`open`] opens the content alone, for the renderer tests.
//!
//! The window is a test window: the Kit's deterministic platform with the
//! machine's real text system and, for a PNG, an offscreen renderer. What a
//! PNG shows depends on the fonts and the graphics adapter of the machine
//! that made it, so PNGs are artifacts and not goldens; assertions belong on
//! the tree.

pub mod chrome;
pub mod gallery;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use xenadmin_ui::surface::{self, Node};
use xenadmin_ui::{Model, scene};

use crate::assets::AppAssets;
use crate::intents::Hooks;
use crate::kit::testing::{
    ElementSnapshot, HeadlessAppContext, PlatformTextSystem, TestQueryExt, TestWindowExt,
    current_headless_renderer, current_platform,
};
use crate::kit::*;
use crate::scene_view::{SceneView, TAB_BASE};
use crate::shell::ShellView;
use crate::store::{AppStore, Describe};
use gallery::{Lab, LabRef};

/// How a scene is rendered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// The size of the window in logical pixels.
    pub width: f32,
    pub height: f32,
    pub mode: ThemeMode,
    /// 1 or 2 device pixels per logical pixel.
    pub scale: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 1440.,
            height: 900.,
            mode: ThemeMode::Dark,
            scale: 1.,
        }
    }
}

/// A model set up for a scene, and for a gallery scene the lab its
/// descriptions come from.
pub struct Setup {
    pub model: Model,
    pub lab: Option<LabRef>,
    /// Descriptions in place of the model's and the lab's.
    pub describe: Option<Describe>,
}

impl Setup {
    /// A model as it is: what a registered scene set up.
    pub fn model(model: Model) -> Self {
        Self {
            model,
            lab: None,
            describe: None,
        }
    }

    /// A demo model with a lab: the fixture of the renderer tests.
    pub fn lab() -> Self {
        let mut model = Model::fixture();
        model.load_demo(3, 48);
        Self {
            model,
            lab: Some(Lab::new()),
            describe: None,
        }
    }

    fn describe(&self) -> Option<Describe> {
        self.describe
            .clone()
            .or_else(|| self.lab.as_ref().map(Lab::describe))
    }
}

/// Every scene name the tool renders: the registered scenes in registry
/// order, then the renderer's gallery and the shell's fixtures.
pub fn names() -> Vec<&'static str> {
    scene::registry()
        .iter()
        .map(|spec| spec.name)
        .chain(gallery::SCENES)
        .chain(chrome::SCENES)
        .collect()
}

/// The model of a scene. `None` when no scene has that name.
pub fn setup(name: &str) -> Option<Setup> {
    if gallery::SCENES.contains(&name) {
        let mut setup = Setup::lab();
        let lab = setup.lab.clone()?;
        if !gallery::apply(name, &lab) {
            return None;
        }
        // The table scenes show a selection: the third guest, and for the
        // multi selection the second to the fourth.
        let mut guests: Vec<(String, String)> = setup
            .model
            .inventory
            .iter()
            .flat_map(|inventory| inventory.guests())
            .map(|vm| (vm.name.clone(), vm.id.to_string()))
            .collect();
        guests.sort_by(|left, right| xenadmin_ui::natural_cmp(&left.0, &right.0));
        let ids: Vec<String> = guests.into_iter().map(|(_, id)| id).take(4).collect();
        if ids.len() == 4 {
            let selected = match name {
                "gallery-tab" | "gallery-picker" => vec![ids[2].clone()],
                "gallery-multi" => ids[1..4].to_vec(),
                _ => Vec::new(),
            };
            if !selected.is_empty() {
                let target = gallery::grid_target(&setup.model, &selected);
                setup.model.select_in_grid(target);
                let mut lab = lab.borrow_mut();
                lab.selected = selected.first().cloned();
                lab.marked = selected;
            }
        }
        return Some(setup);
    }
    match name {
        "gallery-shell" => {
            return Some(Setup {
                describe: Some(chrome::describe()),
                ..Setup::model(chrome::shell_model())
            });
        }
        // Nothing connected and nothing saved: the window a first start
        // shows.
        "gallery-empty" => return Some(Setup::model(Model::fixture())),
        _ => {}
    }
    let mut model = Model::fixture();
    scene::apply(name, &mut model).then(|| Setup::model(model))
}

/// What a scene window is made of.
pub struct SceneWindow {
    pub window: AnyWindowHandle,
    pub view: Entity<SceneView>,
    pub store: Entity<AppStore>,
    pub lab: Option<LabRef>,
}

/// What the window of a whole scene is made of.
pub struct ShellWindow {
    pub window: AnyWindowHandle,
    pub view: Entity<ShellView>,
    pub store: Entity<AppStore>,
    pub lab: Option<LabRef>,
}

/// Initialises the Kit, the key bindings and the theme, and makes the store
/// of a scene.
fn prepare(cx: &mut App, setup: Setup, options: &Options) -> (Entity<AppStore>, Option<LabRef>) {
    gpui_kit::init(cx);
    crate::init(cx);
    // A capture must show the final state, not a dialog on its way in.
    cx.set_reduce_motion(true);
    crate::theme::apply(options.mode, setup.model.preferences.options.zoom(), cx);
    let describe = setup.describe();
    let lab = setup.lab;
    let store = cx.new(|_| AppStore::new(setup.model));
    if let Some(describe) = describe {
        store.update(cx, |store, cx| store.set_describe(describe, cx));
    }
    (store, lab)
}

fn window_options(options: &Options) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(options.width), px(options.height)),
        })),
        focus: false,
        show: false,
        ..Default::default()
    }
}

/// Opens the content of a scene alone: the object header, the tab strip,
/// the tab body and the surfaces, without the shell. Call it inside
/// `cx.update` of a test or headless application context.
pub fn open(cx: &mut App, setup: Setup, options: &Options) -> Result<SceneWindow, String> {
    let (store, lab) = prepare(cx, setup, options);
    let (window, view) = gpui_kit::open_window(window_options(options), cx, {
        let store = store.clone();
        move |window, cx| cx.new(|cx| SceneView::new(store, window, cx))
    })
    .map_err(|error| format!("the scene window did not open: {error}"))?;
    Ok(SceneWindow {
        window,
        view,
        store,
        lab,
    })
}

/// Opens the whole window of a scene: the shell around its content. The
/// model's intents are recorded, not performed. Call it inside `cx.update`
/// of a test or headless application context.
pub fn open_shell(cx: &mut App, setup: Setup, options: &Options) -> Result<ShellWindow, String> {
    open_shell_with(cx, setup, options, Hooks::default())
}

/// [`open_shell`] with the application's hooks given: what the close of
/// the window and its deactivation call.
pub fn open_shell_with(
    cx: &mut App,
    setup: Setup,
    options: &Options,
    hooks: Hooks,
) -> Result<ShellWindow, String> {
    let (store, lab) = prepare(cx, setup, options);
    let (window, view) = gpui_kit::open_window(window_options(options), cx, {
        let store = store.clone();
        move |window, cx| cx.new(|cx| ShellView::new(store, hooks, window, cx))
    })
    .map_err(|error| format!("the scene window did not open: {error}"))?;
    Ok(ShellWindow {
        window,
        view,
        store,
        lab,
    })
}

/// A rendered scene.
pub struct Output {
    /// The frame, when a capture was asked for.
    pub image: Option<image::RgbaImage>,
    pub tree: Value,
}

/// One platform text system per thread: building a platform starts worker
/// threads that are never joined.
fn text_system() -> Arc<dyn PlatformTextSystem> {
    thread_local! {
        static TEXT_SYSTEM: Arc<dyn PlatformTextSystem> = current_platform(true).text_system();
    }
    TEXT_SYSTEM.with(Arc::clone)
}

/// Renders a scene. With `capture` the frame is drawn by the offscreen
/// renderer, which needs a graphics adapter (a software one will do);
/// without it only the tree is made and no adapter is needed.
pub fn render(name: &str, options: &Options, capture: bool) -> Result<Output, String> {
    let setup = setup(name).ok_or_else(|| format!("no scene is named `{name}`"))?;
    render_setup(name, setup, options, capture)
}

/// [`render`] for a model set up by the caller.
pub fn render_setup(
    name: &str,
    setup: Setup,
    options: &Options,
    capture: bool,
) -> Result<Output, String> {
    if options.scale != 1. && options.scale != 2. {
        return Err(format!("the scale is 1 or 2, not {}", options.scale));
    }
    let mut cx = if capture {
        HeadlessAppContext::with_platform(
            text_system(),
            Arc::new(AppAssets),
            current_headless_renderer,
        )
    } else {
        HeadlessAppContext::with_platform(text_system(), Arc::new(AppAssets), || Ok(None))
    };
    let scene = cx.update(|cx| open_shell(cx, setup, options))?;
    cx.run_until_parked();
    // The dialog host presents the surfaces after the first update, and
    // widgets created in a frame are corrected in the next: draw until the
    // frame is settled.
    for _ in 0..3 {
        cx.update_window(scene.window, |_, window, cx| {
            if options.scale == 1. {
                window.set_scale_factor(1.);
            }
            window.render_frame(cx);
        })
        .map_err(|error| format!("the scene did not draw: {error}"))?;
        cx.run_until_parked();
    }
    let tree = cx
        .update_window(scene.window, |_, window, cx| {
            let mut tree = tree_json(window, cx, &scene.store);
            tree["scene"] = json!(name);
            tree["theme"] = json!(if options.mode.is_dark() {
                "dark"
            } else {
                "light"
            });
            tree["scale"] = json!(options.scale);
            tree
        })
        .map_err(|error| format!("the scene did not draw: {error}"))?;
    let image = if capture {
        let full = cx
            .capture_screenshot(scene.window)
            .map_err(|error| format!("the frame was not captured: {error}"))?;
        Some(if options.scale == 1. {
            // The offscreen target has the size of the 2x test window; the
            // 1x frame is its top left quarter.
            image::imageops::crop_imm(&full, 0, 0, options.width as u32, options.height as u32)
                .to_image()
        } else {
            full
        })
    } else {
        None
    };
    Ok(Output { image, tree })
}

/// Renders a scene and writes what was asked for: the frame as a PNG, the
/// element tree as JSON. An error also when the frame lacks an element its
/// description has; the files are written first, so they can be looked at.
pub fn write(
    name: &str,
    options: &Options,
    snap: Option<&std::path::Path>,
    tree: Option<&std::path::Path>,
) -> Result<(), String> {
    let output = render(name, options, snap.is_some())?;
    if let (Some(path), Some(image)) = (snap, &output.image) {
        image
            .save(path)
            .map_err(|error| format!("{} was not written: {error}", path.display()))?;
    }
    if let Some(path) = tree {
        let text = serde_json::to_string_pretty(&output.tree)
            .map_err(|error| format!("the tree was not serialised: {error}"))?;
        std::fs::write(path, text + "\n")
            .map_err(|error| format!("{} was not written: {error}", path.display()))?;
    }
    let missing = missing_keys(&output.tree);
    if !missing.is_empty() {
        return Err(format!(
            "the frame of `{name}` lacks elements its description has: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

fn id_string(id: &ElementId) -> String {
    match id {
        ElementId::Name(name) => name.to_string(),
        ElementId::NamedInteger(name, index) => format!("{name}:{index}"),
        ElementId::Integer(index) => index.to_string(),
        other => format!("{other:?}"),
    }
}

/// The ids on the way to an element, without the view segments: those are
/// allocation-order ids and shift when a scene creates entities in another
/// order.
pub fn id_path(element: &ElementSnapshot) -> Vec<String> {
    element
        .path()
        .iter()
        .filter(|id| !matches!(id, ElementId::View(_)))
        .map(id_string)
        .collect()
}

/// The nodes of everything the model describes now, by scope: the tab body
/// under [`TAB_BASE`], each surface under its id.
pub fn described(store: &AppStore) -> BTreeMap<String, Vec<Node>> {
    let model = store.model();
    let mut scopes = BTreeMap::new();
    if let Some(body) = store.tab_body(model.active_tab()) {
        scopes.insert(
            TAB_BASE.to_owned(),
            surface::flatten_blocks(TAB_BASE, &body.blocks()),
        );
    }
    for surface in store.surfaces().iter() {
        scopes.insert(surface.id.clone(), surface::flatten(surface));
    }
    scopes
}

/// The keys of a description that must be elements of the frame.
///
/// All of them, except what the frame cannot hold: the options of a
/// drop-down that is closed, what is inside a section that starts folded,
/// and the rows of a table after its first (a table draws the rows that are
/// in view only).
pub fn required_keys(nodes: &[Node]) -> Vec<String> {
    let folded: Vec<String> = nodes
        .iter()
        .filter(|node| node.role == "group" && node.value.as_deref() == Some("collapsed"))
        .map(|node| format!("{}/", node.path))
        .collect();
    let mut tables_seen = BTreeSet::new();
    nodes
        .iter()
        .filter(|node| !folded.iter().any(|prefix| node.path.starts_with(prefix)))
        .filter(|node| node.role != "option")
        .filter(|node| {
            if node.role != "row" {
                return true;
            }
            let table = node.path.rsplit_once('/').map_or("", |(table, _)| table);
            tables_seen.insert(table.to_owned())
        })
        .filter_map(|node| node.key.clone())
        .collect()
}

fn bounds_json(bounds: Bounds<Pixels>) -> Value {
    json!([
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    ])
}

/// The element tree of the last frame, merged with the model's flattened
/// description.
///
/// `elements` lists every element with an id, in reading order, with its id
/// path, what the Kit exposes of it (role, label, value, description,
/// bounds, focus, checked, selected, disabled) and, where the id is the key
/// or the path of a node of the description, that node: its text, its
/// disabled reason and its tone are there even where the Kit exposes none.
/// `missing` lists the nodes that must be in the frame and are not.
/// `model` is the model's own account of the scene.
pub fn tree_json(window: &Window, cx: &App, store: &Entity<AppStore>) -> Value {
    let store = store.read(cx);
    let scopes = described(store);
    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    let elements: Vec<Value> = window
        .elements()
        .iter()
        .map(|element| {
            let path = id_path(element);
            let id = path.last().cloned().unwrap_or_default();
            // The scope is the surface the element is in, else the tab.
            let scope = path
                .iter()
                .rev()
                .find(|segment| segment.as_str() != TAB_BASE && scopes.contains_key(*segment))
                .cloned()
                .unwrap_or_else(|| TAB_BASE.to_owned());
            let node = scopes.get(&scope).and_then(|nodes| {
                nodes
                    .iter()
                    .find(|node| node.key.as_deref() == Some(id.as_str()) || node.path == id)
            });
            if let Some(node) = node {
                found.insert((scope.clone(), node.path.clone()));
            }
            json!({
                "id": id,
                "path": path,
                "role": element.role().map(|role| format!("{role:?}")),
                "label": element.label(),
                "value": element.value(),
                "description": element.description(),
                "placeholder": element.placeholder(),
                "bounds": bounds_json(element.bounds()),
                "visible": element.visible(),
                "focused": element.focused(),
                "checked": element.checked(),
                "selected": element.selected(),
                "expanded": element.expanded(),
                "disabled": element.disabled(),
                "node": node,
            })
        })
        .collect();
    let mut missing = Vec::new();
    for (scope, nodes) in &scopes {
        let required: BTreeSet<String> = required_keys(nodes).into_iter().collect();
        for node in nodes {
            let needed = node.key.as_ref().is_some_and(|key| required.contains(key));
            if needed && !found.contains(&(scope.clone(), node.path.clone())) {
                missing.push(json!({ "scope": scope, "node": node }));
            }
        }
    }
    let viewport = window.viewport_size();
    json!({
        "viewport": [f32::from(viewport.width), f32::from(viewport.height)],
        "elements": elements,
        "missing": missing,
        "model": scene::tree(store.model()),
    })
}

/// The keys the description requires and the frame does not have, as
/// `<scope>: <key>`.
pub fn missing_keys(tree: &Value) -> Vec<String> {
    tree["missing"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            format!(
                "{}: {}",
                entry["scope"].as_str().unwrap_or_default(),
                entry["node"]["key"].as_str().unwrap_or_default()
            )
        })
        .collect()
}
