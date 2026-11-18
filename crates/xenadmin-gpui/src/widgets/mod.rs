//! The shared widgets the renderer draws descriptions with.
//!
//! | Widget | Draws |
//! | --- | --- |
//! | [`form_row`] ([`FormRow`]), [`field_error`] | a label, a control, help text and the validation message |
//! | [`property_list`] | a label and value list with copyable values |
//! | [`review_panel`] | the review step: lines, affected resources, warnings, the stale notice |
//! | [`outcome_banner`] | progress, a result or an unknown outcome, with its actions |
//! | [`experimental_notice`] | the notice `docs/FEATURES.md` requires on an experimental surface |
//! | [`wizard_frame`] | the stepper header of a wizard |
//! | [`properties_frame`] | the vertical page list of a properties dialog beside its page |
//! | [`object_grid::ObjectGrid`] | a table: sort, selection by id, context menu, column widths |
//! | [`picture`] | a picture the model holds, scaled to fit |
//!
//! Each is a function from plain data to an element. None keeps state of
//! its own except the grid, whose table entity lives in the widget cache.

mod experimental_notice;
mod form_row;
pub mod object_grid;
mod outcome_banner;
mod picture;
mod properties_frame;
mod property_list;
mod review_panel;
mod text;
mod wizard_frame;

pub use experimental_notice::experimental_notice;
pub use form_row::{FormRow, field_error, form_row};
pub use object_grid::ObjectGrid;
pub use outcome_banner::outcome_banner;
pub use picture::picture;
pub use properties_frame::{PAGE_LIST_WIDTH, properties_frame};
pub use property_list::property_list;
pub use review_panel::review_panel;
pub use text::{PREFORMATTED_RUN, heading, missing_custom, notice, panel, preformatted, text};
pub use wizard_frame::wizard_frame;
