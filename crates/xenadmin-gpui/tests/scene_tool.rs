//! The scene tool's scrolling: what is below the fold of a tab or of a
//! dialog is brought into a frame of the real size before the capture.
//! (Only the tree is made here: no graphics adapter is needed.)

use serde_json::Value;
use xenadmin_gpui::scenes::{self, Options, Scroll};

const SMALL: Options = Options {
    width: 800.,
    height: 600.,
    mode: xenadmin_gpui::kit::ThemeMode::Dark,
    scale: 1.,
};

fn tree(name: &str, options: &Options, scrolls: &[Scroll]) -> Result<Value, String> {
    let setup = scenes::setup(name).ok_or_else(|| format!("no scene is named {name}"))?;
    let output = scenes::render_scrolled(name, setup, options, scrolls, false)?;
    assert_eq!(
        output.missing,
        Vec::<String>::new(),
        "the completeness of {name} is judged before anything is scrolled"
    );
    Ok(output.tree)
}

/// Left, top, right and bottom of the visible element with a key.
fn bounds(tree: &Value, key: &str) -> [f64; 4] {
    let element = tree["elements"]
        .as_array()
        .expect("elements")
        .iter()
        .find(|element| element["id"] == key && !element["bounds"].is_null())
        .unwrap_or_else(|| panic!("`{key}` is not an element"));
    let b: Vec<f64> = element["bounds"]
        .as_array()
        .expect("bounds")
        .iter()
        .map(|value| value.as_f64().expect("a number"))
        .collect();
    [b[0], b[1], b[0] + b[2], b[1] + b[3]]
}

fn inside(tree: &Value, key: &str, body: &str) -> bool {
    let (inner, outer) = (bounds(tree, key), bounds(tree, body));
    inner[1] >= outer[1] - 0.5 && inner[3] <= outer[3] + 0.5
}

#[test]
fn reveal_brings_a_chart_below_the_fold_of_a_tab_into_the_frame() {
    let plot = "metrics-chart-3-plot";
    let before = tree("performance", &SMALL, &[]).expect("the scene");
    assert!(
        !inside(&before, plot, "tab-scroll"),
        "the fourth chart starts below the fold at 800x600"
    );
    let after = tree("performance", &SMALL, &[Scroll::Reveal(plot.into())]).expect("the scene");
    assert!(
        inside(&after, plot, "tab-scroll"),
        "{:?} in {:?}",
        bounds(&after, plot),
        bounds(&after, "tab-scroll")
    );
    // Only what is in the body moved.
    assert_eq!(bounds(&before, "tab-body"), bounds(&after, "tab-body"));
}

#[test]
fn reveal_scrolls_the_body_of_a_long_dialog() {
    let wide = Options::default();
    let (field, body) = ("join-confirmation", "pool-join-body");
    let before = tree("pool-join", &wide, &[]).expect("the scene");
    assert!(!inside(&before, field, body));
    let after = tree("pool-join", &wide, &[Scroll::Reveal(field.into())]).expect("the scene");
    assert!(
        inside(&after, field, body),
        "{:?} in {:?}",
        bounds(&after, field),
        bounds(&after, body)
    );
}

#[test]
fn scroll_turns_the_wheel_over_an_element_by_pixels_in_the_order_given() {
    let chart = "metrics-chart-0-title";
    let before = tree("performance", &SMALL, &[]).expect("the scene");
    let down = Scroll::by("tab-scroll=150").expect("a step");
    let after = tree("performance", &SMALL, std::slice::from_ref(&down)).expect("the scene");
    assert_eq!(bounds(&after, chart)[1], bounds(&before, chart)[1] - 150.);
    // Down and then partly back up.
    let back = Scroll::by("tab-scroll=-50").expect("a step");
    let after = tree("performance", &SMALL, &[down, back]).expect("the scene");
    assert_eq!(bounds(&after, chart)[1], bounds(&before, chart)[1] - 100.);
}

#[test]
fn a_step_that_cannot_run_is_an_error_and_not_a_frame() {
    let error = tree(
        "performance",
        &SMALL,
        &[Scroll::Reveal("no-such-key".into())],
    )
    .expect_err("no element has that key");
    assert!(error.contains("no-such-key"), "{error}");
    for wrong in ["tab-scroll", "tab-scroll=", "=40", "tab-scroll=far"] {
        assert!(Scroll::by(wrong).is_err(), "{wrong}");
    }
    assert_eq!(
        Scroll::by("tab-scroll=-12.5"),
        Ok(Scroll::By {
            key: "tab-scroll".into(),
            pixels: -12.5
        })
    );
}
