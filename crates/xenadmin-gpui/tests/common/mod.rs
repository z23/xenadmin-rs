//! What the tests of the wanted list share: a scene opened through the scene
//! tool's own setup, and the steps a wish is made of, run with real events
//! on the Kit's headless test platform.
//!
//! "Inside the window" is strict here: all of the element is within the
//! window AND within every scrolling body it is drawn in, so a control that
//! is cut by the edge of a dialog's body does not count. "Reachable" means a
//! turn of the wheel over that body brings it there.
#![allow(dead_code)]

use xenadmin_gpui::kit::testing::{
    ElementSnapshot, ScrollDelta, TestAppContext, TestEventExt as _, TestQueryExt as _,
    TestWindowExt as _,
};
use xenadmin_gpui::kit::*;
use xenadmin_gpui::scene_view::LOCAL_ROW_KEY;
use xenadmin_gpui::scenes::{self, Options, Setup, ShellWindow};
use xenadmin_ui::Model;
use xenadmin_ui::surface::Node;

pub const WIDE: (f32, f32) = (1440., 900.);
/// The smallest size the main window can have. The Rune client's compact
/// size, 513 by 652, does not exist any more.
pub const SMALL: (f32, f32) = (800., 600.);
pub const BOTH: &[(f32, f32)] = &[WIDE, SMALL];
pub const ONLY_SMALL: &[(f32, f32)] = &[SMALL];
pub const ONLY_WIDE: &[(f32, f32)] = &[WIDE];

/// The reason a control that would change a host has in the demo.
pub const DEMO: &str = "Demo mode cannot change infrastructure.";

pub fn open(cx: &mut TestAppContext, setup: Setup, (width, height): (f32, f32)) -> ShellWindow {
    let options = Options {
        width,
        height,
        ..Options::default()
    };
    let scene = cx
        .update(|cx| scenes::open_shell(cx, setup, &options))
        .expect("shell window");
    cx.run_until_parked();
    settle(cx, &scene);
    scene
}

pub fn open_scene(cx: &mut TestAppContext, name: &str, size: (f32, f32)) -> ShellWindow {
    let setup = scenes::setup(name).unwrap_or_else(|| panic!("no scene is named {name}"));
    open(cx, setup, size)
}

/// A registered scene with its model changed before the window opens.
pub fn open_scene_with(
    cx: &mut TestAppContext,
    name: &str,
    size: (f32, f32),
    change: impl FnOnce(&mut Model),
) -> ShellWindow {
    let mut setup = scenes::setup(name).unwrap_or_else(|| panic!("no scene is named {name}"));
    change(&mut setup.model);
    open(cx, setup, size)
}

/// Closes the window of a scene, so that a test can open the next one.
pub fn close(cx: &mut TestAppContext, scene: ShellWindow) {
    let _ = cx.update_window(scene.window, |_, window, _| window.remove_window());
    drop(scene);
    cx.run_until_parked();
}

/// Draws until what the last event caused is on screen.
pub fn settle(cx: &mut TestAppContext, scene: &ShellWindow) {
    for _ in 0..3 {
        cx.update_window(scene.window, |_, window, cx| window.render_frame(cx))
            .expect("window");
        cx.run_until_parked();
    }
}

pub fn on<R>(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    act: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    let result = cx
        .update_window(scene.window, |_, window, cx| act(window, cx))
        .expect("window");
    cx.run_until_parked();
    settle(cx, scene);
    result
}

pub fn model<R>(cx: &mut TestAppContext, scene: &ShellWindow, read: impl FnOnce(&Model) -> R) -> R {
    cx.update(|cx| read(scene.store.read(cx).model()))
}

/// Changes the model as an area's handler would, and draws the result.
pub fn run(cx: &mut TestAppContext, scene: &ShellWindow, change: impl FnOnce(&mut Model)) {
    cx.update(|cx| scene.store.update(cx, |store, cx| store.run(change, cx)));
    cx.run_until_parked();
    settle(cx, scene);
}

/// The ids of the surfaces the store describes, back to front.
pub fn surfaces(cx: &mut TestAppContext, scene: &ShellWindow) -> Vec<String> {
    cx.update(|cx| {
        let surfaces = scene.store.read(cx).surfaces();
        surfaces.iter().map(|surface| surface.id.clone()).collect()
    })
}

/// The id of the surface in front.
pub fn front(cx: &mut TestAppContext, scene: &ShellWindow) -> Option<String> {
    surfaces(cx, scene).pop()
}

/// What the surface in front describes.
pub fn described(cx: &mut TestAppContext, scene: &ShellWindow) -> Vec<Node> {
    cx.update(|cx| {
        let surfaces = scene.store.read(cx).surfaces();
        surfaces
            .last()
            .map(xenadmin_ui::surface::flatten)
            .unwrap_or_default()
    })
}

/// What the surface in front describes under a key.
pub fn node(cx: &mut TestAppContext, scene: &ShellWindow, key: &str) -> Option<Node> {
    described(cx, scene)
        .into_iter()
        .find(|node| node.key.as_deref() == Some(key))
}

/// Whether any element of the window says the text, in its name or value.
pub fn says(cx: &mut TestAppContext, scene: &ShellWindow, text: &str) -> bool {
    on(cx, scene, |window, _| with_text(window, None, text).is_ok())
}

/// The operations the model has handed to a connection.
pub fn submitted(cx: &mut TestAppContext, scene: &ShellWindow) -> usize {
    model(cx, scene, |m| {
        m.operations.len() + m.resource_operations.len()
    })
}

fn name(id: &ElementId) -> Option<&str> {
    match id {
        ElementId::Name(name) => Some(name.as_ref()),
        _ => None,
    }
}

/// The key of an element: the last id of its path.
pub fn key_of(element: &ElementSnapshot) -> Option<&str> {
    element.path().last().and_then(name)
}

pub fn under(element: &ElementSnapshot, scope: &str) -> bool {
    let path = element.path();
    path[..path.len().saturating_sub(1)]
        .iter()
        .any(|id| name(id) == Some(scope))
}

/// The element with a key. `scope` is the surface in front, when there is
/// one: a key is looked for in it alone, as a user can only reach what is in
/// front. `local:<command>` is the command's button in the local action row
/// (a command can be in the toolbar as well), and a key that ends in `*` is
/// the first element, from the top, whose key starts so.
pub fn locate(window: &Window, scope: Option<&str>, key: &str) -> Result<ElementSnapshot, String> {
    let (scope, key) = match key.strip_prefix("local:") {
        Some(command) => (Some(LOCAL_ROW_KEY), command),
        None => (scope, key),
    };
    let prefix = key.strip_suffix('*');
    let mut found: Vec<ElementSnapshot> = window
        .elements()
        .into_iter()
        .filter(|element| {
            let hit = match (key_of(element), prefix) {
                (Some(name), Some(prefix)) => name.starts_with(prefix),
                (Some(name), None) => name == key,
                (None, _) => false,
            };
            hit && scope.is_none_or(|scope| under(element, scope))
        })
        .collect();
    found.sort_by(|left, right| {
        let (left, right) = (left.bounds().origin, right.bounds().origin);
        (left.y, left.x)
            .partial_cmp(&(right.y, right.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if found.len() > 1 && prefix.is_none() {
        found.retain(ElementSnapshot::visible);
        if found.len() != 1 {
            return Err(format!(
                "`{key}` is {} elements in {}",
                found.len(),
                scope.unwrap_or("the window")
            ));
        }
    }
    if found.is_empty() {
        return Err(format!(
            "`{key}` is not in {}",
            scope.unwrap_or("the window")
        ));
    }
    // Of several that start so, the first that can be used.
    let usable = found
        .iter()
        .position(|element| element.disabled() != Some(true))
        .unwrap_or(0);
    Ok(found.remove(usable))
}

/// What an element is seen through, innermost last: the bodies that scroll
/// (of a dialog or a wizard, of a properties page, of a tab).
fn clips(window: &Window, element: &ElementSnapshot) -> Vec<ElementSnapshot> {
    let path = element.path();
    let all = window.elements();
    let mut clips = Vec::new();
    for (index, id) in path.iter().enumerate().take(path.len().saturating_sub(1)) {
        let Some(name) = name(id) else { continue };
        if !scenes::scroll::is_scroll_body(name) {
            continue;
        }
        if let Some(clip) = all.iter().find(|other| other.path() == &path[..=index]) {
            clips.push(clip.clone());
        }
    }
    clips
}

fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    let slack = px(0.5);
    inner.left() >= outer.left() - slack
        && inner.top() >= outer.top() - slack
        && inner.right() <= outer.right() + slack
        && inner.bottom() <= outer.bottom() + slack
}

/// All of the element is drawn, in the window and in every body it is seen
/// through: a user sees it and can press it without scrolling anything.
pub fn inside(window: &Window, element: &ElementSnapshot) -> Result<(), String> {
    let key = key_of(element).unwrap_or("?").to_owned();
    let bounds = element.bounds();
    if !element.visible() {
        return Err(format!("`{key}` is not visible: {bounds:?}"));
    }
    if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
        return Err(format!("`{key}` has no size: {bounds:?}"));
    }
    let viewport = Bounds::new(Default::default(), window.viewport_size());
    if !contains(viewport, bounds) {
        return Err(format!(
            "`{key}` is outside the {:?} window: {bounds:?}",
            viewport.size
        ));
    }
    for clip in clips(window, element) {
        if !contains(clip.bounds(), bounds) {
            return Err(format!(
                "`{key}` is cut by `{}`: {bounds:?} in {:?}",
                key_of(&clip).unwrap_or("?"),
                clip.bounds()
            ));
        }
    }
    Ok(())
}

/// What [`reach`] asks: [`inside`], except that an element taller than a
/// body it is seen through (a long review, a summary, a whole section)
/// cannot be inside it. Such an element is reached when its start is at
/// the top of that body and no other body cuts the start.
fn reached(window: &Window, element: &ElementSnapshot) -> Result<(), String> {
    let bounds = element.bounds();
    let bodies = clips(window, element);
    let start = bodies
        .iter()
        .map(|body| body.bounds().top())
        .fold(px(0.), |low, top| low.max(top));
    // The lowest body is the one the element is seen in: a body that grows
    // with what it holds is no window on it.
    let tight = bodies.into_iter().min_by(|left, right| {
        let height = |body: &ElementSnapshot| f32::from(body.bounds().size.height);
        height(left).total_cmp(&height(right))
    });
    match tight {
        Some(clip) if bounds.size.height > clip.bounds().size.height - px(8.) => {
            let seen = clip.bounds();
            let viewport = Bounds::new(Default::default(), window.viewport_size());
            if element.visible()
                && contains(viewport, seen)
                && bounds.top() >= start - px(0.5)
                && bounds.top() < seen.top() + seen.size.height / 2.
                && bounds.left() >= seen.left() - px(0.5)
                && bounds.right() <= seen.right() + px(0.5)
            {
                Ok(())
            } else {
                Err(format!(
                    "the start of `{}` is not in `{}`: {bounds:?} in {seen:?}",
                    key_of(element).unwrap_or("?"),
                    key_of(&clip).unwrap_or("?")
                ))
            }
        }
        _ => inside(window, element),
    }
}

/// Brings an element into view by turning the wheel over the body it is in,
/// and says whether all of it is inside the window then.
pub fn reach(
    window: &mut Window,
    cx: &mut App,
    scope: Option<&str>,
    key: &str,
) -> Result<ElementSnapshot, String> {
    // The innermost body first, then the bodies that one is seen through.
    let depth = clips(window, &locate(window, scope, key)?).len();
    for level in (0..depth).rev() {
        for _ in 0..8 {
            let element = locate(window, scope, key)?;
            let bodies = clips(window, &element);
            let Some(clip) = bodies.get(level).cloned() else {
                break;
            };
            let (bounds, seen) = (element.bounds(), clip.bounds());
            let down = bounds.bottom() - seen.bottom();
            let up = seen.top() - bounds.top();
            // The start is cut by the lowest top edge of the bodies the
            // element is seen through, which need not be this one's.
            let start = bodies
                .iter()
                .map(|body| body.bounds().top())
                .fold(seen.top(), |low, top| low.max(top));
            let room = bounds.top() - start;
            let delta = if down > px(0.5) && room > px(0.) {
                // Not further than its top: a tall element shows its start.
                -(down + px(4.)).min(room)
            } else if up > px(0.5) {
                up + px(4.)
            } else {
                break;
            };
            let Some(id) = clip.path().last().cloned() else {
                break;
            };
            window.scroll(id, ScrollDelta::Pixels(point(px(0.), delta)), cx);
            window.render_frame(cx);
        }
    }
    let element = locate(window, scope, key)?;
    reached(window, &element)?;
    Ok(element)
}

/// A press and a release of the primary button at the centre of an element
/// that was brought into view.
pub fn press(
    window: &mut Window,
    cx: &mut App,
    scope: Option<&str>,
    key: &str,
) -> Result<ElementSnapshot, String> {
    let element = reach(window, cx, scope, key)?;
    click_at(window, cx, element.bounds().center(), MouseButton::Left);
    Ok(element)
}

pub fn click_at(window: &mut Window, cx: &mut App, at: Point<Pixels>, button: MouseButton) {
    window.pointer_move(at, None, cx);
    window.render_frame(cx);
    window.pointer_down(at, button, cx);
    window.pointer_up(at, button, cx);
    window.render_frame(cx);
}

/// The first element, from the top, whose accessible name or value has the
/// text: a line of a review, a caption, a notice.
pub fn with_text(
    window: &Window,
    scope: Option<&str>,
    text: &str,
) -> Result<ElementSnapshot, String> {
    let mut found: Vec<ElementSnapshot> = window
        .elements()
        .into_iter()
        .filter(|element| {
            (element.label().is_some_and(|label| label.contains(text))
                || element.value().is_some_and(|value| value.contains(text)))
                && scope.is_none_or(|scope| under(element, scope))
        })
        .collect();
    // The innermost element that says it: a group is named after its
    // heading, the line is what must be seen.
    found.sort_by(|left, right| {
        let (a, b) = (left.bounds(), right.bounds());
        (a.size.height, a.origin.y)
            .partial_cmp(&(b.size.height, b.origin.y))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
        .into_iter()
        .next()
        .ok_or_else(|| format!("no element says \"{text}\""))
}

/// One thing a wish asks of a scene. A row of the table is a list of these.
#[derive(Clone, Copy, Debug)]
pub enum Step {
    /// These are inside the window as it is: nothing is scrolled.
    Inside(&'static [&'static str]),
    /// Each of these can be brought wholly into view by scrolling the body
    /// it is in.
    Reach(&'static [&'static str]),
    /// Brought into view and pressed with the pointer: how a wish gets from
    /// one state of a surface to the next.
    Press(&'static str),
    /// Pressed, and the field takes the text from the keyboard.
    Type(&'static str, &'static str),
    /// The surface in front has this id.
    Front(&'static str),
    /// No surface is open.
    NoSurface,
    /// The control is disabled; a reason that is not empty must be its
    /// description.
    Disabled(&'static str, &'static str),
    /// The control is there and can be used.
    Enabled(&'static str),
    /// None of these is an element of the surface in front (or of the
    /// window, when there is none).
    Absent(&'static [&'static str]),
    /// An element that says this can be scrolled wholly into view.
    Says(&'static str),
    /// The accessible name of the element contains the text.
    Label(&'static str, &'static str),
    /// What the model describes under the key (its label, value or text)
    /// contains the text: for a control whose element the Kit gives no
    /// accessible name.
    Described(&'static str, &'static str),
    /// The checkbox is, or is not, checked.
    Checked(&'static str, bool),
    /// The first is drawn above the second.
    Above(&'static str, &'static str),
    /// Pressed with the pointer, it submits nothing, opens and closes
    /// nothing and leaves what the surface describes as it was.
    Inert(&'static str),
    /// The model is changed as a handler would change it: for a state no
    /// registered scene starts in.
    Set(fn(&mut Model)),
    /// A keystroke ("escape", "shift-down") to whatever has the keyboard.
    Key(&'static str),
}

pub fn run_step(cx: &mut TestAppContext, scene: &ShellWindow, step: Step) -> Result<(), String> {
    let front_id = front(cx, scene);
    let scope = front_id.as_deref();
    match step {
        Step::Inside(keys) => on(cx, scene, |window, _| {
            keys.iter().try_for_each(|key| {
                let element = locate(window, scope, key)?;
                inside(window, &element)
            })
        }),
        Step::Reach(keys) => on(cx, scene, |window, cx| {
            keys.iter()
                .try_for_each(|key| reach(window, cx, scope, key).map(|_| ()))
        }),
        Step::Press(key) => on(cx, scene, |window, cx| {
            let element = press(window, cx, scope, key)?;
            match element.disabled() {
                Some(true) => Err(format!(
                    "`{key}` is disabled: {}",
                    element.description().unwrap_or("no reason is given")
                )),
                _ => Ok(()),
            }
        }),
        Step::Type(key, text) => on(cx, scene, |window, cx| {
            press(window, cx, scope, key)?;
            window.input(text, cx);
            Ok(())
        }),
        Step::Front(id) => match scope {
            Some(front) if front == id => Ok(()),
            other => Err(format!("the surface in front is {other:?}, not `{id}`")),
        },
        Step::NoSurface => match scope {
            None => Ok(()),
            Some(front) => Err(format!("`{front}` is open")),
        },
        Step::Disabled(key, reason) => on(cx, scene, |window, cx| {
            let element = reach(window, cx, scope, key)?;
            if element.disabled() != Some(true) {
                return Err(format!("`{key}` is not disabled"));
            }
            if !reason.is_empty() && element.description() != Some(reason) {
                return Err(format!(
                    "`{key}` is disabled with {:?}, not \"{reason}\"",
                    element.description()
                ));
            }
            Ok(())
        }),
        Step::Enabled(key) => on(cx, scene, |window, cx| {
            let element = reach(window, cx, scope, key)?;
            match element.disabled() {
                Some(true) => Err(format!(
                    "`{key}` is disabled: {:?}",
                    element.description().unwrap_or_default()
                )),
                _ => Ok(()),
            }
        }),
        Step::Absent(keys) => on(cx, scene, |window, _| {
            keys.iter()
                .try_for_each(|key| match locate(window, scope, key) {
                    Ok(element) => Err(format!(
                        "`{key}` is in {}: {:?}",
                        scope.unwrap_or("the window"),
                        element.bounds()
                    )),
                    Err(_) => Ok(()),
                })
        }),
        Step::Says(text) => on(cx, scene, |window, cx| {
            let element = with_text(window, scope, text)?;
            let key = key_of(&element)
                .ok_or_else(|| format!("what says \"{text}\" has no key"))?
                .to_owned();
            let owner = element.path().iter().rev().skip(1).find_map(name);
            // The line is looked for again by its own key, under the
            // element it is in: a path of a description is not unique in a
            // window, a key under its parent is.
            reach(window, cx, owner.or(scope), &key).map(|_| ())
        }),
        Step::Label(key, text) => on(cx, scene, |window, _| {
            let element = locate(window, scope, key)?;
            match element.label() {
                Some(label) if label.contains(text) => Ok(()),
                other => Err(format!("`{key}` is named {other:?}, without \"{text}\"")),
            }
        }),
        Step::Described(key, text) => {
            let nodes = described(cx, scene);
            let node = nodes
                .iter()
                .find(|node| node.key.as_deref() == Some(key))
                .ok_or_else(|| format!("the surface describes no `{key}`"))?;
            let said = [&node.label, &node.value, &node.text];
            if said.iter().any(|part| {
                part.as_ref()
                    .is_some_and(|part| part.to_lowercase().contains(&text.to_lowercase()))
            }) {
                Ok(())
            } else {
                Err(format!("`{key}` is described without \"{text}\": {node:?}"))
            }
        }
        Step::Checked(key, checked) => on(cx, scene, |window, _| {
            let element = locate(window, scope, key)?;
            if element.checked() == Some(checked) {
                Ok(())
            } else {
                Err(format!(
                    "`{key}` is checked: {:?}, not {checked}",
                    element.checked()
                ))
            }
        }),
        Step::Above(upper, lower) => on(cx, scene, |window, _| {
            let above = locate(window, scope, upper)?;
            // What is seen of the upper one: a body that scrolls ends
            // above the footer, whatever the length of what is in it.
            let upper_at = clips(window, &above)
                .iter()
                .fold(above.bounds(), |seen, clip| seen.intersect(&clip.bounds()));
            let lower_at = locate(window, scope, lower)?.bounds();
            if above.visible() && upper_at.bottom() <= lower_at.top() + px(0.5) {
                Ok(())
            } else {
                Err(format!(
                    "`{upper}` is not above `{lower}`: {upper_at:?} {lower_at:?}"
                ))
            }
        }),
        Step::Inert(key) => {
            let before = (
                submitted(cx, scene),
                surfaces(cx, scene),
                described(cx, scene),
            );
            on(cx, scene, |window, cx| {
                press(window, cx, scope, key).map(|_| ())
            })?;
            let after = (
                submitted(cx, scene),
                surfaces(cx, scene),
                described(cx, scene),
            );
            if after.0 != before.0 {
                return Err(format!("a press on `{key}` submitted an operation"));
            }
            if after.1 != before.1 {
                return Err(format!(
                    "a press on `{key}` changed the surfaces: {:?} to {:?}",
                    before.1, after.1
                ));
            }
            if after.2 != before.2 {
                let changed: Vec<&Node> = after
                    .2
                    .iter()
                    .filter(|node| !before.2.contains(node))
                    .take(3)
                    .collect();
                return Err(format!(
                    "a press on `{key}` changed what the surface describes: {changed:?}"
                ));
            }
            Ok(())
        }
        Step::Set(change) => {
            run(cx, scene, change);
            Ok(())
        }
        Step::Key(key) => {
            on(cx, scene, |window, cx| window.press(key, cx));
            Ok(())
        }
    }
}

/// Runs the steps of a wish on an open scene. The error names the step.
pub fn run_steps(
    cx: &mut TestAppContext,
    scene: &ShellWindow,
    steps: &[Step],
) -> Result<(), String> {
    for (index, step) in steps.iter().enumerate() {
        run_step(cx, scene, *step).map_err(|error| format!("step {index} {step:?}: {error}"))?;
    }
    Ok(())
}

/// One wish of the list: a scene, the sizes it is asked at, and what must
/// be on screen, reachable or happen there.
#[derive(Clone, Copy, Debug)]
pub struct Wish {
    /// The area that asked, as the list names it.
    pub area: &'static str,
    /// The Rune test whose window half this is, or what it is about.
    pub about: &'static str,
    pub scene: &'static str,
    pub sizes: &'static [(f32, f32)],
    pub steps: &'static [Step],
}

/// Runs a wish at each of its sizes, in a window of its own each time.
/// What failed, one line per size.
pub fn run_wish(cx: &mut TestAppContext, wish: &Wish) -> Vec<String> {
    let mut failures = Vec::new();
    for size in wish.sizes {
        let scene = open_scene(cx, wish.scene, *size);
        if let Err(error) = run_steps(cx, &scene, wish.steps) {
            failures.push(format!(
                "{} / {} ({}) at {}x{}: {error}",
                wish.area, wish.scene, wish.about, size.0, size.1
            ));
        }
        close(cx, scene);
    }
    failures
}

/// A wish that must hold: the failures are the panic.
#[track_caller]
pub fn check(cx: &mut TestAppContext, wish: &Wish) {
    let failures = run_wish(cx, wish);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
