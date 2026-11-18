//! Area `vm_properties`: the settings of one VM or one template.
//!
//! Rune module: `vm_properties.rs`. Tabs: none.
//!
//! The Rune module was one dialog, "VM properties" / "Template properties":
//! every field in one scrolling column, one review of everything that
//! changed, one submission (`VmCommand::Configure` with one
//! `VmSettingsPatch`). Its successor is a set of pages of the Properties
//! dialog, which the kernel owns ([`crate::properties`]: the commands
//! `vm.properties` and `template.properties`, the surfaces `vm-properties`
//! and `template-properties`).
//!
//! ## One draft, seven pages
//!
//! The six pages of the plan edit ONE draft and a seventh page reviews it.
//! Splitting the editor into pages that each applied their own change would
//! have changed what is submitted together (and so the order of the steps on
//! the host and what a failure leaves applied), so it was not done: the
//! draft is captured when the first of these pages is entered, survives
//! turning to another page (also to a page of another area and back), and
//! is dropped when the dialog closes. "Review changes" on any of the six
//! pages turns the dialog to the last page, Review Changes, which shows the
//! one review and has "Apply changes"; leaving that page is the Rune "Back".
//!
//! | Page (id) | Holds (Rune fields) |
//! | --- | --- |
//! | General (`general`) | Name, Description; the custom-template caption |
//! | CPU and Memory (`cpu-memory`) | halted: Maximum vCPUs, Startup vCPUs, Cores per socket, the four memory limits; running: Live vCPUs, Dynamic memory minimum and maximum; CPU scheduling weight; Shadow memory multiplier |
//! | Boot Options (`boot-options`) | Boot order (halted) |
//! | Start Options (`start-options`) | Start automatically with the pool |
//! | Home Server (`home-server`) | the home host: No preference, or a server |
//! | Advanced (`advanced`) | firmware of an empty custom template, Request Secure Boot policy, certificate readiness with "Refresh boot readiness" |
//! | Review Changes (`review`, not in the plan) | the review of the patch, Back, Apply changes |
//!
//! All seven carry the experimental notice, as the Rune dialog did. The
//! pages Custom Fields and Alerts of the plan are other areas'
//! (`organization`, `schedules_alarms`) and apply by their own commands; the
//! review here is of this area's draft only. Several VMs or templates at
//! once are the `bulk_edit` area's: this area has no page for them.
//!
//! ## Commands
//!
//! | Command | Label | Where | Does |
//! | --- | --- | --- | --- |
//! | `vm.memory-settings` | Memory Settings | local row of the Memory tab of a VM or template | opens the Properties dialog on CPU and Memory |
//!
//! ## For other areas
//!
//! - [`describe`]: the review line of one `VmSettingEdit` (bulk properties
//!   reviews with it).
//! - [`parse_memory`], [`memory_text`]: the memory notation of the fields.
//! - [`commands::MEMORY_SETTINGS`]: the command above.
//! - [`scenes::guest`]: the demo pool with a guest selected.
//!
//! ## What has no successor, and where it went
//!
//! - The buttons "Properties" (`open-properties`) and "Template properties"
//!   (`open-template-properties`) of the details pane and "Properties…" of
//!   the row menu: the kernel's commands.
//! - The dialog key `properties-dialog`, its headings "VM properties" and
//!   "Template properties" and its caption of name and UUID: the kernel's
//!   dialog with its title and subtitle. The two review headings are on the
//!   review page.
//! - The second "connection changed" line of the Rune dialog: one line on
//!   the six pages ([`change::NOT_CURRENT`]), the other is the stale line of
//!   the review and the reason of a disabled Apply ([`change::STALE`]).
//! - The global error line inside the dialog (`s.error`): a refused
//!   submission is on the page (`Editor::error`).
//! - "Changed-block tracking" of the plan's Advanced page: the client has no
//!   such setting of a VM. It is a property of one virtual disk
//!   (`DeviceProperty::DisableCbt`), which is the `vm_devices` area's.
//!
//! ## Scenes
//!
//! `properties`, `properties-running`, `home-host`, `secure-boot`
//! ([`scenes`]).
//!
//! ## Tests
//!
//! The seven Rune tests are in `tests.rs` under their Rune names: three
//! moved (`memory_input_is_exact_and_bounded`,
//! `property_draft_keeps_the_original_target_and_values`,
//! `running_guest_draft_uses_live_cpu_and_dynamic_memory_commands`), four
//! ported to the harness. None retired. What only a window can show (the
//! footer reachable at 513x652) is asked for in
//! `notes/progress/gpui-tests-wanted.md`.
//!
//! The files, in the order to read them: [`draft`] (the Rune logic, carried
//! over mechanically), [`change`] (capture, review, submission, the page
//! events), [`view`] (the pages described), [`commands`], [`scenes`].

use crate::prelude::*;

pub mod change;
pub mod commands;
pub mod draft;
pub mod scenes;
pub mod view;

#[cfg(test)]
mod tests;

pub use draft::{describe, memory_text, parse_memory};

/// The state of the area. `m.areas.vm_properties`.
#[derive(Default)]
pub struct State {
    /// The draft of the VM or template whose Properties dialog is open on
    /// (or has been on) one of this area's pages.
    pub editor: Option<draft::Editor>,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        poll: change::poll,
        on_scope_change: change::drop_draft,
        on_session_change: change::drop_draft,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: change::page_event,
        ..AreaSpec::new("vm_properties")
    }
}
