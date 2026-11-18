//! Area `vm_devices`: the devices of a VM and the disks of a repository.
//!
//! Rune modules: `vm_devices.rs`, `vm_device_actions.rs`,
//! `vm_device_lifecycle.rs`, `vm_device_properties.rs`, `disk_workspace.rs`
//! (and their test files). Plan packages W1 and W2.
//!
//! The Rune Devices page replaced the content area and pinned the selection
//! to its VM. It dissolves into two tabs of the VM: Storage (one row per
//! disk attachment or CD/DVD drive) and Networking (one row per interface),
//! each with the card of the selected row under the grid and "Recent device
//! changes" at the foot. Every button of the page is a command in the local
//! row of its tab and in the menu of a row. The Rune "Virtual disks"
//! workspace was a pool-wide dialog with a list: the list is the Storage tab
//! of a repository, its three modes are three commands on a disk.
//!
//! | Rune control (key) | Command | Label | Where |
//! | --- | --- | --- | --- |
//! | `create-vm-disk` | `vm.disk.new` | New Virtual Disk | Storage > Virtual Disks; Storage tab 10 |
//! | `attach-vm-disk` | `vm.disk.attach` | Attach Virtual Disk | Storage > Virtual Disks; Storage tab 12 |
//! | `add-vm-cd` | `vm.cd.add` | Add CD/DVD Drive | Storage tab 14 |
//! | `detach-disk-<vbd>` | `vm.disk.detach` | Detach | Storage tab 20; row menu |
//! | "Connect disk…" | `vm.disk.activate` | Activate | Storage tab 22; row menu |
//! | "Disconnect disk…" | `vm.disk.deactivate` | Deactivate | Storage tab 24; row menu |
//! | "Insert ISO…" | `vm.cd.insert` | Insert ISO | Storage tab 26; row menu |
//! | "Eject media…" | `vm.cd.eject` | Eject Media | Storage tab 28; row menu |
//! | `disk-properties-<vbd>` | `vm.disk.properties` | Properties | Storage tab 30; row menu |
//! | `disk-manage-<vdi>`, `disk-mode-Grow` | `disk.grow` | Resize | Storage tab 32; row menu; a disk's context menu |
//! | `disk-mode-Copy` | `disk.copy` | Copy | Storage tab 34; row menu; a disk's context menu |
//! | `disk-mode-Owned cleanup` | `disk.cleanup` | Delete Owned Disk | Storage tab 36; row menu; a disk's context menu |
//! | `delete-disk-<vbd>` | `vm.disk.delete` | Delete | Storage tab 38; row menu |
//! | `add-vm-nic` | `vm.nic.add` | Add | Networking tab 10 |
//! | `edit-nic-<vif>` | `vm.nic.properties` | Properties | Networking tab 20; row menu |
//! | `nic-properties-<vif>` | `vm.nic.rate-limit` | Rate Limit | Networking tab 30; row menu |
//! | "Connect NIC…" | `vm.nic.activate` | Activate | Networking tab 40; row menu |
//! | "Disconnect NIC…" | `vm.nic.deactivate` | Deactivate | Networking tab 50; row menu |
//! | `delete-nic-<vif>` | `vm.nic.remove` | Remove | Networking tab 90; row menu |
//!
//! A row of a VM's tabs is an attachment or an interface, not a tree object:
//! the selection is this area's ([`selected_attachment`],
//! [`selected_interface`]) and the scope keeps meaning the VM. A row of a
//! repository's disk list is a virtual disk, a tree object: its selection is
//! the grid selection and a secondary press opens the disk's context menu.
//! On a template every command is hidden: its tabs are information only
//! (decision 3 of `AREAS.md`).
//!
//! | Surface | Id | Kind | Experimental |
//! | --- | --- | --- | --- |
//! | Review device change / Insert ISO media | `device-change` | dialog (review; ISO: choice, then review) | yes |
//! | Create and attach disk, Attach existing disk, Add CD/DVD drive, Add network interface, Change network or MAC, Review device change | `device-lifecycle` | dialog (form, then review; detach and delete open on the review) | yes |
//! | Device properties | `device-properties` | dialog (chooser, editor, review: one change per review) | yes |
//! | Resize / Copy / Delete owned virtual disk | `virtual-disk` | dialog (form, then review; stays open after a submission and lists the pool's disk operations) | yes |
//! | Virtual disk Properties, page General | kernel `disk-properties` | page: name and description, then their review | yes |
//! | Virtual disk Properties, page Size and Location | kernel `disk-properties` | page: information, Resize… and Copy… | yes |
//!
//! All carry the experimental notice: device and disk administration is
//! outside the supported part, and the three device dialogs lacked the
//! notice in Rune only because they were children of the page (decision 1).
//!
//! Scenes, under their Rune names: `vm-devices` (the Storage tab),
//! `device-properties`, `disk-grow`, `disk-copy`, `disk-cleanup`.
//!
//! For other areas (public functions; an area never reaches into
//! [`State`]):
//!
//! - [`selected_attachment`], [`selected_interface`]: the selected row of a
//!   VM's Storage or Networking tab.
//! - [`selected_disk`]: the virtual disk a command on the Storage tab means
//!   (the disk of the selected attachment, or the disk selected in a
//!   repository's list). `transfers` needs it for "Export disk image".
//! - [`parse_size`]: a capacity as bytes, KiB, MiB or GiB.
//! - [`class_message`]: the sentence for an inventory class that cannot be
//!   read.
//! - [`commands`]: the command ids.
//! - [`close`].
//!
//! Commands of other areas this area expects on its tabs and does not name:
//! "Export disk image" (`transfers`), `storage.rescan` and
//! `storage.set-default` (`network`), the repository commands of `storage`
//! and `storage_maintenance`, and "Mount guest tools media"
//! (`open-guest-tools`), which is `vm.install-guest-tools` of `vm_tools`
//! (VM menu and the Storage tab's local row).
//!
//! What has no successor, and where it went:
//!
//! - The Devices page itself, its heading, "Back to inventory"
//!   (`close-vm-devices`) and the pinning of the selection: the tab strip and
//!   the object header. The "VM belongs to another connection" and "no
//!   longer present" texts: the scope cannot show such a VM.
//! - The cards: the grid, and the group under it for the selected row.
//! - "Guest tools and reported addresses": a group at the foot of the
//!   Networking tab (the General tab is `general`'s).
//! - `disks-open`, `disk-select-<id>`, `disk-list-back`, Previous and Next:
//!   the Storage tab of a repository with its search field (`disk-search`);
//!   the paging is gone. A pool-wide list is the Objects view.
//! - The mode buttons `disk-mode-*` and `disk-heading`: three commands and
//!   the title of the dialog.
//! - "Operation history" (`disk-history`) opened the pool history of the
//!   resource editor: it shows the Events tab of the Notifications view.
//! - "Move" of the plan's tab row: these modules never moved a disk
//!   (`DiskOperation` is grow, copy, cleanup); moving is the migration
//!   wizard's. "Add" on a repository: a disk is created for a VM
//!   (`vm.disk.new`).
//! - The dead "Grow virtual disk" path of the device change dialog.
//! - The repository hub's multipathing and path details of an attachment:
//!   not in these modules; the list keeps the columns `general` showed.
//!
//! The files: [`actions`], [`lifecycle`], [`properties`], [`disks`] (the
//! logic carried over), [`commands`], [`tabs`], [`view`], [`scenes`],
//! `tests`.

use xenadmin_core::{ObservedVm, PoolId, VbdId, VdiId, VifId, VmId};

use crate::prelude::*;

pub mod actions;
pub mod commands;
pub mod disks;
pub mod lifecycle;
pub mod properties;
pub mod scenes;
pub mod tabs;
pub mod view;

#[cfg(test)]
mod tests;

pub use actions::parse_size;
pub use tabs::class_message;

/// The state of the area. `m.areas.vm_devices`. At most one of the three
/// device forms is open; the disk panel is a surface of its own.
#[derive(Default)]
pub struct State {
    /// Connect, disconnect, eject, insert ISO.
    pub form: Option<actions::Form>,
    /// Create, attach, detach, delete; add, change and delete a NIC.
    pub lifecycle: Option<lifecycle::Form>,
    /// One property change of a device.
    pub properties: Option<properties::Form>,
    /// Resize, copy or delete one virtual disk.
    pub disks: Option<Box<disks::Panel>>,
    /// The selected row of a VM's Storage tab.
    pub disk_row: Option<VbdId>,
    /// The selected row of a VM's Networking tab.
    pub nic_row: Option<VifId>,
    /// The search field above a repository's disks.
    pub disk_search: String,
}

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands: commands::commands,
        scenes: scenes::scenes,
        surfaces: view::surfaces,
        tab: tabs::tab,
        property_pages: view::pages,
        property_page: view::page,
        on_property_page: view::page_event,
        on_scope_change: leave,
        on_session_change: leave,
        ..AreaSpec::new("vm_devices")
    }
}

/// Closes every surface of the area.
pub fn close(m: &mut Model) {
    clear(m);
}

/// Before another form opens: the forms are mutually exclusive. A page's
/// draft takes its Properties dialog with it.
pub(crate) fn clear(m: &mut Model) {
    m.areas.vm_devices.form = None;
    m.areas.vm_devices.lifecycle = None;
    m.areas.vm_devices.disks = None;
    properties::close(m);
}

/// Another object selected in the tree, another session shown: the forms
/// and the row selections belonged to the previous one. (Rune kept the disk
/// workspace over a profile switch and showed it as stale; it closes here.)
fn leave(m: &mut Model) {
    let state = &mut m.areas.vm_devices;
    state.form = None;
    state.lifecycle = None;
    state.disks = None;
    state.properties = None;
    state.disk_row = None;
    state.nic_row = None;
    state.disk_search.clear();
}

/// The VM as the connection of its pool observes it now.
pub(crate) fn observed(m: &Model, pool: PoolId, vm: VmId) -> Option<ObservedVm> {
    m.snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok())
}

/// The selected attachment of the Storage tab of the VM in scope.
pub fn selected_attachment(m: &Model) -> Option<(PoolId, VmId, VbdId)> {
    let c = commands::subject(m, m.scope())?;
    let row = m.areas.vm_devices.disk_row?;
    c.inventory
        .details
        .vbds
        .get(&row)
        .filter(|vbd| vbd.vm == Some(c.vm))
        .map(|_| (c.pool, c.vm, row))
}

/// The selected interface of the Networking tab of the VM in scope.
pub fn selected_interface(m: &Model) -> Option<(PoolId, VmId, VifId)> {
    let c = commands::subject(m, m.scope())?;
    let row = m.areas.vm_devices.nic_row?;
    c.inventory
        .details
        .vifs
        .get(&row)
        .filter(|vif| vif.vm == Some(c.vm))
        .map(|_| (c.pool, c.vm, row))
}

/// The virtual disk a disk command would act on now.
pub fn selected_disk(m: &Model) -> Option<(PoolId, VdiId)> {
    commands::disk_subject(m, &m.target()).ok()
}
