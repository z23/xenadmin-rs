//! The header of a wizard.

use crate::kit::*;

/// The stepper of a wizard: every step with its number, the current one
/// marked, the earlier ones shown as done. It is a display: the footer the
/// surface gives moves between steps, so that the model validates a step
/// before it is left.
pub fn wizard_frame(id: &str, steps: &[String], current: usize, _cx: &App) -> AnyElement {
    let label = steps
        .get(current)
        .map(|step| format!("Step {} of {}: {step}", current + 1, steps.len()))
        .unwrap_or_default();
    div()
        .id(SharedString::from(format!("{id}-steps")))
        .test_support()
        .role(Role::List)
        .aria_label(SharedString::from(label))
        .w_full()
        .child(
            Stepper::new(SharedString::from(format!("{id}-stepper")))
                .selected_index(current)
                .small()
                .items(
                    steps
                        .iter()
                        .map(|step| StepperItem::new().child(SharedString::from(step.clone()))),
                ),
        )
        .into_any_element()
}
