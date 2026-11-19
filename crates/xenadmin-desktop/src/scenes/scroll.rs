//! Scrolling a scene before its frame is captured: what is below the fold
//! of a tab or of a dialog can be looked at in a frame of the real size.
//!
//! A step turns the wheel, as a user would. `By` turns it over an element
//! by a number of pixels; `Reveal` turns it over the bodies an element is
//! seen through until the element is inside them.

use crate::kit::testing::{
    ElementSnapshot, TestEventExt as _, TestQueryExt as _, TestWindowExt as _,
};
use crate::kit::*;
use crate::scene_view::{TAB_BODY_KEY, TAB_COLUMN_KEY, TAB_SCROLL_KEY};

/// One step of scrolling.
#[derive(Clone, Debug, PartialEq)]
pub enum Scroll {
    /// The wheel is turned over the element with `key`: `pixels` down, or
    /// up when negative. Whatever scrolls under the pointer there moves:
    /// the body of a tab or a dialog, a grid, the tree.
    By { key: String, pixels: f32 },
    /// The element with this key is scrolled into view. It has to be an
    /// element: a row of a grid or of the tree that is far out of view is
    /// not one, and is reached with [`Scroll::By`].
    Reveal(String),
}

impl Scroll {
    /// `KEY=PIXELS` of the command line.
    pub fn by(text: &str) -> Result<Self, String> {
        let wrong = || format!("--scroll takes KEY=PIXELS, not `{text}`");
        let (key, pixels) = text.rsplit_once('=').ok_or_else(wrong)?;
        let pixels: f32 = pixels.parse().map_err(|_| wrong())?;
        if key.is_empty() || !pixels.is_finite() {
            return Err(wrong());
        }
        Ok(Self::By {
            key: key.to_owned(),
            pixels,
        })
    }
}

fn name(id: &ElementId) -> Option<&str> {
    match id {
        ElementId::Name(name) => Some(name.as_ref()),
        _ => None,
    }
}

/// Whether the element with this key is a body that scrolls: of a tab
/// (`tab-scroll`; of a tab that ends in a table also `tab-results-body`,
/// the table's part, and `tab-column`, which holds both parts and scrolls
/// when sections follow a short table), of a dialog or a wizard
/// (`<surface>-body`), of a properties page (`<surface>-pages-page`). The
/// renderer marks exactly these for the test support.
pub fn is_scroll_body(key: &str) -> bool {
    key == TAB_SCROLL_KEY
        || key == TAB_COLUMN_KEY
        || key.ends_with("-page")
        || (key.ends_with("-body") && key != TAB_BODY_KEY)
}

/// The element with a key. Of several (a command in the toolbar and in the
/// local row, a key in the tab and in a dialog over it), the one under
/// `front`, the surface in front, and then the first from the top.
fn locate(window: &Window, front: Option<&str>, key: &str) -> Result<ElementSnapshot, String> {
    let mut found: Vec<ElementSnapshot> = window
        .elements()
        .into_iter()
        .filter(|element| element.path().last().and_then(name) == Some(key))
        .collect();
    if let Some(front) = front {
        let under =
            |element: &ElementSnapshot| element.path().iter().any(|id| name(id) == Some(front));
        if found.iter().any(under) {
            found.retain(under);
        }
    }
    found.sort_by(|left, right| {
        let (left, right) = (left.bounds().origin, right.bounds().origin);
        (left.y, left.x)
            .partial_cmp(&(right.y, right.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    found
        .into_iter()
        .next()
        .ok_or_else(|| format!("no element has the key `{key}`"))
}

/// The paths of the bodies an element is seen through, innermost first.
fn bodies(element: &ElementSnapshot) -> Vec<Vec<ElementId>> {
    let path = element.path();
    let mut bodies: Vec<Vec<ElementId>> = (0..path.len().saturating_sub(1))
        .filter(|index| name(&path[*index]).is_some_and(is_scroll_body))
        .map(|index| path[..=index].to_vec())
        .collect();
    bodies.reverse();
    bodies
}

/// A turn of the wheel at a point: `down` pixels down.
fn wheel(window: &mut Window, cx: &mut App, at: Point<Pixels>, down: Pixels) {
    window.pointer_move(at, None, cx);
    let event = ScrollEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), -down)),
        ..Default::default()
    };
    window.dispatch_event(gpui_kit::InputEvent::to_platform_input(event), cx);
    window.render_frame(cx);
}

/// The part of an element that is in the window.
fn seen(window: &Window, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
    bounds.intersect(&Bounds::new(Default::default(), window.viewport_size()))
}

/// Runs one step. `front` is the surface in front, if any.
pub fn apply(
    window: &mut Window,
    cx: &mut App,
    front: Option<&str>,
    step: &Scroll,
) -> Result<(), String> {
    window.render_frame(cx);
    match step {
        Scroll::By { key, pixels } => {
            let element = locate(window, front, key)?;
            let at = seen(window, element.bounds()).center();
            wheel(window, cx, at, px(*pixels));
            Ok(())
        }
        Scroll::Reveal(key) => {
            let depth = bodies(&locate(window, front, key)?).len();
            // The innermost body first, then the one that shows that body.
            for level in 0..depth {
                for _ in 0..12 {
                    let element = locate(window, front, key)?;
                    let Some(path) = bodies(&element).into_iter().nth(level) else {
                        break;
                    };
                    let Some(body) = window
                        .elements()
                        .into_iter()
                        .find(|other| other.path() == path.as_slice())
                    else {
                        break;
                    };
                    let (bounds, clip) = (element.bounds(), body.bounds());
                    let below = bounds.bottom() - clip.bottom();
                    let above = clip.top() - bounds.top();
                    let down = if below > px(0.5) && above < px(0.) {
                        // Not further than its top: an element taller than
                        // the body shows its start.
                        (below + px(8.)).min(-above)
                    } else if above > px(0.5) {
                        -(above + px(8.))
                    } else {
                        break;
                    };
                    if down.abs() < px(0.5) {
                        break;
                    }
                    wheel(window, cx, seen(window, clip).center(), down);
                }
            }
            let element = locate(window, front, key)?;
            if element.visible() {
                Ok(())
            } else {
                Err(format!("`{key}` could not be scrolled into view"))
            }
        }
    }
}
