# xenadmin-rs: front-end migration to GPUI Kit

Nov 13, 2026 · @Brendan Hiley

## Summary

`xenadmin-desktop` moves from Rune to GPUI Kit in six gated phases (0 to 5). The interface is rebuilt around XCP-ng Center's model: tree selection sets scope, a per-object tab strip shows its views, and one command table feeds the menu bar, toolbar and context menus. `xenadmin-core`, `xenadmin-xapi` and `xenadmin-process` do not change.

This plan is written for coding agents. Each phase lists its work packages, its exit gate and the evidence to record. Section headings are stable references; cite them in commits and reports.

One blocker is already known. GPUI 0.3.12 delivers no physical key codes, and the console needs them for QEMU Extended Key Events. Spike S1 in Phase 0 resolves this before any view is ported.

Rules that hold for the whole migration:

- **Spikes before port work.** Phase 0 proves the console, headless verification, the runtime bridge, shell primitives and packaging on GPUI. A failed gate stops the migration before any view is rewritten.
- **Model before views.** Selection, tab sets, command eligibility, drafts and scenes live in a toolkit-free crate, `xenadmin-ui`. The GPUI layer stays thin.
- **Two front ends until cutover.** The Rune binary remains the released artifact until the GPUI binary passes the same recovery gate on the lab. No new features land during the port.
- **No core changes.** If a work package seems to need a change in `xenadmin-core`, stop that package, record a finding under `docs/reports/`, and continue with another package.
- **No new top-level buttons.** Every action enters through the command table defined in "Target information architecture".
- **Information is a tab, action is a dialog.** A surface that only shows state never opens as a modal.
- **Evidence over claims.** Follow `AGENTS.md`: report implemented, fixture-verified and live-verified behaviour separately. Headless output never establishes real Wayland, clipboard, keyboard or Orca behaviour.
- **Pins are exact.** `gpui-kit = "=0.9.1"`. Upgrades happen only between phases.

Non-goals: Windows or macOS builds, pixel parity with XCP-ng Center, changes to core protocol behaviour or journal formats, and new product scope.

Facts in this plan were read from this repository at `d6294bd` (workspace 0.20.0), the published sources of `gpui-kit` 0.9.1, `gpui-component` 0.9.1, `gpui-base` 0.9.1, `gpui-pre` 0.3.12, `gpui-pre-linux` 0.3.12 and `gpui-pre-wgpu` 0.3.12, and the reference client at `xcp-ng/xenadmin@35dc091c`. Code blocks marked "interface notation" are sketches, not compiled code. Re-check an API name against the pinned source before relying on it.

## Current state

The Rune front end is one crate of 119 files and about 63,500 lines, and 103 of those files use Rune directly. The port is a rewrite of that crate, not a transliteration.

| Fact | Value at `d6294bd` | Consequence |
| --- | --- | --- |
| Front-end crate | `crates/xenadmin-desktop`, binary `xenadmin-rs` | Replaced by `crates/xenadmin-gpui` |
| Coupling | 103 of 119 files import `rune` or `rune_kit`, about 59,300 lines | Almost nothing moves unchanged |
| State | One `struct State` in `main.rs`, about 110 fields. Each feature is an `Option` of an `Editor`, `Panel` or `Workspace` | Split into model (`xenadmin-ui`) and views |
| Feature surfaces | 49 `fn dialog(` functions. `main.rs` routes 46 through `rune_kit::root(base, [x::dialog(..)])` | Each needs a destination, see "Target information architecture" |
| Controls in use | `Button::new` 585, `TextInput::new` 132, `Checkbox::new` 46, `MenuItem::new` 11, `TextArea::new` 6, `Table::new` 4, `Tree::new` 1, `SplitView::new` 1, `Toolbar::new` 1. No select control | Choices are rows of buttons and most lists are hand-built rows |
| Text buffers | `rune::TextEdit` appears 624 times in 54 files, inside editor and draft structs | Drafts become plain `String` fields in the model |
| Scenes | 128 named scenes in `AppSpec.scenes`, each `fn(&mut State)` | Scene names are stable identifiers. Port each with its view |
| Tests | 370 `#[test]` in 83 files. 98 construct `rune::Runtime` and use `frame`, `find(key)`, `pointer_down`, `pointer_up` and `runtime.state` | Pure tests move to the model. Runtime tests become `#[gpui_kit::test]` |
| Check pipeline | `scripts/check.sh`: fmt, clippy, workspace tests, then `xenadmin-rs --snap <png> --tree <json> --scene <name> --theme <t> --size <WxH> --scale 1 [--at 1]` for dozens of scenes | Needs a GPUI equivalent, see "Verification" |
| CI | `.github/workflows/ci.yml` on `ubuntu-24.04`. Uploads `artifacts/*.png` and `*.json` as `rune-ui`, then builds the archive and AppImage | Rename, add packages, keep both jobs until cutover |
| Core notification | Core handles expose `notify_on_change(Arc<dyn Fn() + Send + Sync>)`. Core owns its Tokio runtime | Toolkit-neutral. Bridge in "Target architecture" |
| Keyring | Secret Service client (`zbus`) inside `xenadmin-core` | Unaffected |
| File dialogs | 12 `rune::FileDialogOptions` call sites | Replace with portal prompts |
| Console input | `on_key`, `on_key_up`, `on_physical_input` with `PhysicalKeyCode`, app option `.modifier_keys()`, `rune::Pixels` with `SurfaceFit::Fit`, hidden local cursor, drawn guest cursor | Hardest part. Spike S1 |
| Console windows | `console_windows.rs` registers eight lazy secondary windows with `rune::WindowSpec::from_app` | Multi-window on GPUI |
| Charts | `metrics_view.rs` draws with `rune::Shape` and `rune::canvas` | `LineChart` or a custom canvas |
| Preferences | `preferences.rs`: saved searches, hidden history, last profile, `sidebar_width`, `inspector_width`, `columns`, options including interface scale | Keep the stored file readable |
| Lints | Workspace `unsafe_code = "forbid"` and `clippy::dbg_macro = "deny"` | Check macro interplay in spike S0 |
| Real-session driver | Nested compositor, AT-SPI for discovery and activation, physical key injection (`docs/VERIFICATION.md`) | Keep element labels so the driver still finds controls |
| Supported scope | `docs/FEATURES.md`, section "Supported and experimental" | Defines Phase 3 |

The defects this migration removes, stated as requirements:

1. Global actions are a flat row of buttons above the table. They must become menu, toolbar and context-menu commands.
2. Tabs select an object type (Virtual machines, Templates, Hosts, Storage, Networks). They must instead show the views of the selected object.
3. The details pane is a column of buttons that open dialogs. Object information must be visible in tabs without opening anything.
4. Every feature area is a modal dialog. Only actions may be modal.

To list the files that are already free of Rune and can move unchanged:

```sh
grep -LE '\brune(_kit)?::|use rune' crates/xenadmin-desktop/src/*.rs
```

## Target architecture

Three crates carry the front end after cutover: `xenadmin-core` (unchanged), `xenadmin-ui` (new, toolkit-free model) and `xenadmin-gpui` (new, views). GPUI types never leave `xenadmin-gpui`.

```text
crates/
  xenadmin-core/      unchanged. Protocol, pool owners, journal, console worker, keyring.
  xenadmin-xapi/      unchanged. Generated bindings (LGPL).
  xenadmin-process/   unchanged. Sandbox helper.
  xenadmin-ui/        NEW. UI model. Depends on xenadmin-core, serde, uuid. No rune, no gpui.
  xenadmin-gpui/      NEW. GPUI Kit views. Depends on xenadmin-ui, xenadmin-core, gpui-kit.
  xenadmin-keys/      NEW only if spike S1 selects route A. Raw Wayland key codes.
                      The only new crate allowed to use `unsafe`.
  xenadmin-desktop/   Rune front end. Frozen after Phase 1. Deleted in Phase 5.
tools/host-probe/     unchanged.
```

### Dependency rules

- `xenadmin-ui` must not depend on `rune`, `rune-kit` or any `gpui*` crate. `scripts/check.sh` enforces this with `cargo tree`.
- `xenadmin-core` must not depend on `xenadmin-ui`.
- `xenadmin-gpui` reaches GPUI only through `gpui_kit::*`. It adds no direct `gpui` dependency.
- Raw RPC responses, session references and transport details stay inside the core, as `AGENTS.md` already requires.
- The core's Tokio runtime is the only Tokio runtime. The front end never calls `block_on`.

### Ownership

| Concern | Owner | Types |
| --- | --- | --- |
| Connections, snapshots, operations, consoles | `xenadmin-core` | `Backend`, `Connection`, `ConnectionSnapshot`, `Inventory`, `Operation`, `OperationRecord`, `Console`, `ConsoleSnapshot` |
| Selection, scope, tree rows, grid rows, tab set, command table and eligibility, drafts, reviews, scenes, preferences | `xenadmin-ui` | `Model`, `Selection`, `TabId`, `TabSet`, `CommandSpec`, `Eligibility`, `Intent`, per-area `Draft` types, `scenes::REGISTRY`, `Preferences` |
| Widget state: scroll, column widths in use, focus, text buffers, open popovers | `xenadmin-gpui` | `Entity<TableState<D>>`, `Entity<TreeState>`, `Entity<InputState>` |
| Console input state per window: held keys, buttons, wheel remainder | `xenadmin-gpui` | `ConsoleSession` entity |

### `xenadmin-ui` surface

```rust
// interface notation
pub struct Model { /* connections, latest snapshots, selection, open surface, drafts, preferences */ }

impl Model {
    pub fn poll(&mut self) -> Changed;                    // read latest() from core handles; never waits
    pub fn selection(&self) -> &Selection;                // single or multi, by ObjectId
    pub fn select(&mut self, target: SelectionTarget);
    pub fn tabs(&self) -> TabSet;                         // ordered tab ids valid for the selection
    pub fn active_tab(&self) -> TabId;
    pub fn set_active_tab(&mut self, tab: TabId);
    pub fn commands(&self) -> &'static [CommandSpec];     // the single command table
    pub fn eligibility(&self, id: CommandId) -> Eligibility; // Enabled | Disabled(reason) | Hidden
    pub fn invoke(&mut self, id: CommandId) -> Intent;
    pub fn status(&self) -> StatusLine;                   // connection state, freshness, counters
}

pub enum Intent {
    OpenSurface(Surface),      // Dialog(DialogId) | Wizard(WizardId) | Properties(ObjectRef, PageId)
    Submit(Submission),        // already reviewed; hand to core
    ChooseFile(FileRequest),   // the view asks the portal, then calls back into the model
    OpenUrl(String),
    None,
}
```

Model rules:

- Every method is synchronous and returns without waiting for a host.
- Drafts hold `String`, `bool` and enums. They never hold a widget buffer.
- Validation has the shape `fn validate(&Draft, &Inventory) -> Result<Prepared, Vec<FieldError>>`.
- A review is built in the model from a captured observation, exactly as the Rune modules do today. The view only renders it.
- Passwords and other secrets keep their current transient types and `zeroize` handling. A view clears its input buffer as soon as it hands a secret to the model.

### State flow in GPUI

1. `main` runs `gpui_kit::application().run(|cx| { gpui_kit::init(cx); .. })`.
2. `AppStore` is one entity that owns the `Model`. It is the only writer.
3. The core calls a waker on its Tokio threads. The waker sets an `AtomicBool` and sends on a channel only when the flag flips from false to true. Bursts collapse into one wake.
4. A foreground task awaits the channel, clears the flag, and runs `store.update(..)`, which calls `model.poll()` and `cx.notify()` when something changed.
5. Views read the model in `render`. Handlers call `model.invoke(..)` through `store.update(..)` and act on the returned `Intent`.
6. Console frames use their own waker and their own `ConsoleSession` entity. Frame traffic must not re-render the shell.

```rust
// interface notation: the bridge
let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
let pending = Arc::new(AtomicBool::new(false));
let waker: Arc<dyn Fn() + Send + Sync> = {
    let pending = pending.clone();
    Arc::new(move || {
        if !pending.swap(true, Ordering::AcqRel) {
            let _ = tx.unbounded_send(());
        }
    })
};
cx.spawn(async move |cx| {
    while rx.next().await.is_some() {
        pending.store(false, Ordering::Release);
        let _ = store.update(cx, |s, cx| if s.model.poll().any() { cx.notify() });
    }
})
.detach();
```

Any executor-agnostic channel is acceptable. The channel must not require a Tokio context on the receiving side.

### Windows

| Window | Content | Rules |
| --- | --- | --- |
| Main | Shell: title bar, menu bar, toolbar, navigation, tabs, status bar | Root view wrapped in `Root` so dialogs, sheets and notifications render. `app_id` equals the installed desktop entry name |
| Console window, up to eight | One `ConsoleSession` | Opened on demand with `cx.open_window`. Holds a clone of the original `Connection`. Never opens another backend, profile store or keyring session |
| Dialogs, wizards, property dialogs | Layers inside a window | `window.open_dialog`, `window.open_alert_dialog`, `window.close_dialog`. Not separate OS windows |

Lifecycle rules carried over from the Rune `main`:

- Close of the main window: save preferences, release console input, close every console window, disconnect all profiles. Use `window.on_window_should_close` and `cx.on_app_quit`.
- Window deactivation: release held console keys and buttons. Use `cx.observe_window_activation`.
- `--version` answers before anything is claimed or opened. `--demo` and `--scene <name>` stay.
- Single instance: keep `instance_launch` semantics. The toolkit-free part moves to `xenadmin-ui`. The GPUI side activates with `cx.activate(true)` and `window.activate_window()`.
- Interface scale: apply `preferences.options.interface_scale` with `window.set_rem_size` and the theme font size. Spike S4 records what scales.
- Theme: `Theme` with `ThemeMode::Light` and `ThemeMode::Dark`, accent carried from today's `oklch(0.73, 0.15, 55.0)`.

## Target information architecture

The window has five fixed regions, and every feature lands in exactly one of four surface kinds: tab, properties page, wizard or dialog. The tables below are the defaults; a deviation is recorded in the migration ledger with its reason.

### Shell regions

```text
+----------------------------------------------------------------------------------+
| Title bar + menu bar: File View Pool Server VM Storage Templates Tools Window Help |
+----------------------------------------------------------------------------------+
| Toolbar: Back Forward | Add Server  New Pool  New Storage  New VM | power commands |
+-----------------------+----------------------------------------------------------+
| Navigation pane       | Object header: icon, name, state, pool and host          |
|   search box          +----------------------------------------------------------+
|   tree                | Tab strip: General  Memory  Storage  Networking  ...     |
|                       +----------------------------------------------------------+
|   view switcher:      | Tab body                                                 |
|     Infrastructure    |                                                          |
|     Objects           |                                                          |
|     Organization      |                                                          |
|     Saved Searches    |                                                          |
|     Notifications     |                                                          |
+-----------------------+----------------------------------------------------------+
| Status bar: connection state and freshness | operations | unknown outcomes | selection |
+----------------------------------------------------------------------------------+
```

| Region | GPUI Kit component | Fed by |
| --- | --- | --- |
| Title bar | `TitleBar` | Window title from the selection |
| Menu bar | `AppMenuBar` over `cx.set_menus(..)` | Command table. `MenuItem::Action` carries `disabled` and `checked`; rebuild the menus when eligibility changes |
| Toolbar | `Toolbar`, `ToolbarGroup`, `Button` | Command table, toolbar slots. Show a power command only when it is eligible |
| Navigation pane | `h_resizable` panel, `Input`, `Tree` with `.context_menu(..)` | `Model` tree rows for the active view |
| Tab strip | `TabBar`, `Tab` | `Model::tabs()` |
| Tab body | One view entity per `TabId` | Model |
| Status bar | `StatusBar` | `Model::status()` |

Navigation views follow the reference client:

- **Infrastructure**: pool, then hosts, then VMs, templates and storage repositories.
- **Objects**: by type: pools, servers, VMs, templates, storage repositories, networks, virtual disks, snapshots, vApps.
- **Organization**: folders, tags, custom fields, vApps.
- **Saved Searches**.
- **Notifications**: Alerts and Events.

### Tab matrix

`Model::tabs()` returns the tabs of the row that matches the selection, in column order. Unit-test every row.

| Selection | Home | General | Memory | Storage | Networking | NICs | HA | Users | Console | Performance | Snapshots | Search |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Nothing, or the root node | yes |  |  |  |  |  |  |  |  |  |  | yes |
| Pool |  | yes | yes | yes | yes |  | yes | yes |  |  |  | yes |
| Host, live |  | yes | yes | yes | yes | yes |  |  | yes | yes |  | yes |
| Host, not live or disconnected |  | yes |  |  |  |  |  |  |  |  |  | yes |
| VM |  | yes | yes | yes | yes |  |  |  | yes | yes | yes | yes |
| Template |  | yes | yes | yes | yes |  |  |  |  |  |  | yes |
| Storage repository |  | yes |  | yes |  |  |  |  |  |  |  | yes |
| Network |  | yes |  |  | yes |  |  |  |  |  |  | yes |
| Virtual disk |  | yes |  |  |  |  |  |  |  |  |  | yes |
| Several objects, folder, tag, saved search |  |  |  |  |  |  |  |  |  |  |  | yes |

General through Snapshots for pool, host, VM, template, storage repository and virtual disk follow `GetNewTabPages()` in the reference `XenAdmin/MainWindow.cs`. HA and Users on Pool and the Network row are additions for this client. GPU and USB tabs are omitted because specialized hardware is excluded scope.

| Tab | Content | Rune modules to port from |
| --- | --- | --- |
| Home | Connection list with state, Add Server, Connect, Disconnect | `profiles_view.rs`, `profile_credentials.rs` |
| General | `DescriptionList` sections for the object, Properties button | `inventory_information.rs`, `resource_details.rs`, `demo_details.rs` |
| Memory | Memory per host and VM, entry to memory settings | `vm_properties.rs` (memory part), `host_admin.rs` (control-domain memory) |
| Storage on a VM or template | Virtual disk grid with Add, Attach, Detach, Move, Properties, Delete | `vm_devices.rs`, `vm_device_actions.rs`, `vm_device_lifecycle.rs`, `vm_device_properties.rs`, `disk_workspace.rs` |
| Storage on a host or pool | Repository grid with New, Repair, Detach, Reattach, Forget, Destroy, Set as Default | `storage_view.rs`, `storage_maintenance_view.rs`, `storage_graph_view.rs` |
| Storage on a repository | Disks in the repository with Add, Rescan, Move, Copy, Delete | `disk_workspace.rs`, `storage_view.rs` |
| Networking on a VM | Virtual interface grid with Add, Properties, Activate, Deactivate, Remove | `vm_devices.rs`, `vm_device_*` |
| Networking on a host or pool | Network grid and management interfaces, with Add Network, Properties, Remove, Configure | `network_admin.rs`, `resource_admin.rs`, `network_interfaces.rs`, `network_service_ui.rs`, `network_topology_ui.rs`, `network_fabric_ui.rs`, `network_fabric_pool_ui.rs`, `management_network_ui.rs` |
| NICs | Physical interface grid with Create Bond, Delete Bond, Rescan | `network_admin.rs`, `management_bond_ui.rs`, `management_ipv6_bond_ui.rs` |
| HA | HA state, heartbeat repositories, restart priorities, Configure and Disable | `pool_view.rs`, `ha_policy_view.rs` |
| Users | Directory subjects, roles, session permissions | `pool_security_view.rs`, `session_permissions_view.rs` |
| Console | Canvas, console toolbar, recovery, undock | `console_view.rs`, `console_recovery.rs`, `console_export.rs`, `host_console.rs`, `console_windows.rs`, `external_tools.rs` |
| Performance | Charts, range, data sources, presets | `metrics_view.rs`, `metric_presets.rs`, `host_monitoring_view.rs` |
| Snapshots | List or tree, details, Take, Revert, Delete, Export | `vm_snapshots.rs`, `snapshot_inspection.rs`, `snapshot_capture.rs`, `bulk_export_snapshots.rs` |
| Search | Result grid for the scope, filters, save search | `search_view.rs`, `search_files.rs`, the inventory table in `main.rs` |
| Notifications, Events | Operations, tasks, unknown outcomes, recheck, acknowledge, history, export | `activity.rs`, `activity_export.rs`, `failure_help.rs` |
| Notifications, Alerts | Pool messages with reviewed dismissal | `activity.rs` (messages) |

### Surface kinds

| Kind | Use for | Built with |
| --- | --- | --- |
| Tab | A persistent view of the selected object | View entity under the tab strip |
| Properties dialog | Editable settings of one object, grouped in pages | `Dialog` with a vertical page list |
| Wizard | Multi-step creation or transfer | `Dialog` with `Stepper`; the last step is the existing review |
| Dialog | A single-step action with review or confirmation | `Dialog` or `AlertDialog` |

- The review-before-submit step that every Rune workflow has is preserved in every wizard and dialog.
- A surface for an experimental area keeps its notice, as `docs/FEATURES.md` requires.
- Partial and unknown outcomes are shown where the action was started and in Events. They are never only a transient notification.

### Properties pages

| Object | Pages | Rune modules |
| --- | --- | --- |
| VM | General, Custom Fields, CPU and Memory, Boot Options, Start Options, Alerts, Home Server, Advanced (firmware, Secure Boot, changed-block tracking) | `vm_properties.rs`, `vm_placement.rs`, `organization_view.rs`, `alarms_view.rs`, `ha_policy_view.rs` |
| Host | General, Custom Fields, Alerts, Multipathing, Log Destination, Power On | `host_admin.rs`, `host_power_view.rs`, `storage_graph_view.rs`, `alarms_view.rs` |
| Pool | General, Custom Fields, Email Options, Network Options, Security | `pool_view.rs`, `pool_security_view.rs`, `alarms_view.rs`, `resource_admin.rs` |
| Storage repository | General, Custom Fields, Alerts, Read Caching | `storage_view.rs`, `storage_maintenance_view.rs` |
| Network | General, Custom Fields, Network Settings | `network_admin.rs`, `network_interfaces.rs` |
| Virtual disk | General, Custom Fields, Size and Location | `disk_workspace.rs`, `vm_device_properties.rs` |
| Several VMs | Bulk properties | `bulk_properties.rs`, `bulk_properties_model.rs` |

### Wizards and dialogs

| Surface | Kind | Rune modules |
| --- | --- | --- |
| Add Server, with certificate review | Dialog | `profiles_view.rs`, `profile_trust.rs`, `profile_credentials.rs` |
| Reconnect As | Dialog | `profile_reconnect.rs` |
| Add Server to Pool | Wizard | `profile_pool_join.rs` |
| Change Server Password | Dialog | `profile_root_password.rs` |
| New VM | Wizard: Template, Name, Installation Media, Home Server, CPU and Memory, Storage, Networking, Cloud-init, Review | `vm_lifecycle.rs`, `vm_creation.rs`, `vm_creation_cloud.rs`, `cloud_init_view.rs` |
| Copy VM, Convert to Template, Delete VM | Dialogs | `vm_lifecycle.rs`, `vm_deletion.rs`, `bulk_delete.rs` |
| Migrate or Move VM, within a pool or across pools | Wizard: Destination, Storage, Networking, Review | `vm_migration.rs`, `bulk_move.rs`, `cross_pool_vm.rs`, `cross_pool_vm_view.rs`, `bulk_cross_pool.rs` |
| Force and recovery power actions, batch power | Dialogs | `vm_recovery_view.rs`, `bulk_power.rs`, the force review in `main.rs` |
| Take, Revert, Delete Snapshot | Dialogs | `vm_snapshots.rs` |
| Snapshot Schedule Policies | Dialog with a list and a New Policy wizard | `snapshot_policy_view.rs` |
| Import | Wizard: Source, Type, Destination, Storage, Networking, Review | `transfers.rs`, `ovf_transfer_ui.rs` |
| Export | Wizard | `transfers.rs`, `bulk_export.rs`, `bulk_export_snapshots.rs` |
| Convert Disk Image | Wizard | `local_conversion_view.rs` |
| New Storage Repository | Wizard: Type, Name, Location, Review | `storage_view.rs`, `storage_local_view.rs` |
| Repair, Detach, Forget, Destroy repository | Dialogs | `storage_view.rs`, `storage_maintenance_view.rs`, `storage_graph_view.rs` |
| New, Attach, Move, Copy, Resize virtual disk | Dialogs | `disk_workspace.rs`, `vm_device_actions.rs`, `vm_device_lifecycle.rs` |
| New Network | Wizard: Type, Name, Settings, Review | `resource_admin.rs`, `network_admin.rs`, `network_fabric_ui.rs`, `network_fabric_pool_ui.rs` |
| Create Bond | Dialog | `management_bond_ui.rs`, `management_ipv6_bond_ui.rs` |
| IP Address Configuration | Dialog; staged workflows keep each stage review | `management_network_ui.rs`, `network_interfaces.rs`, `network_service_ui.rs`, `network_topology_ui.rs` |
| Configure HA | Wizard | `pool_view.rs`, `ha_policy_view.rs` |
| vApps | Dialog with a list and an editor | `appliance_view.rs` |
| Enter Maintenance Mode, Reboot, Shut Down, Restart Toolstack | Dialogs | `host_admin.rs` |
| Host backup, restore, status report, crashdump deletion | Dialogs | `host_support_view.rs` |
| Make into Standalone Server | Dialog | `pool_dissolve_view.rs` |
| Install Guest Tools | Dialog | `guest_tools_view.rs` |
| External Tools | Dialog | `external_tools.rs`, `external_tool_command.rs` |
| Options, Help | Dialogs | `settings.rs` |
| Inventory report | Dialog | `inventory_report.rs` |

### Menus

Seed the command table from this list. Labels follow the reference `MainWindow.resx`; items marked (+) exist only in this client. Do not use the reference product's name in any label.

| Menu | Items |
| --- | --- |
| File | Import, Import Search, Import Server List, Export Server List, Exit |
| View | Custom Templates, Built-in Templates, Local Storage, Hidden Objects, Toolbar |
| Pool | New Pool, Add Server, Remove Server, Reconnect As, Disconnect, vApps, High Availability, VM Snapshot Schedule Policies, Change Server Password, Make into Standalone Server, Properties |
| Server | Add, Reboot, Power On, Shut Down, Restart Toolstack, Connect, Disconnect, Reconnect As, Connect All, Disconnect All, Back Up, Restore From Backup, Enter or Exit Maintenance Mode, Control Domain Memory, Password (Change, Forget Password), Remove from Pool, Destroy, Properties |
| VM | New VM, Start/Shut Down (Start, Shut Down, Reboot, Suspend, Resume, Force Shut Down, Force Reboot, recovery start (+)), Start on Server, Resume on Server, Migrate to Server, Move VM, Copy VM, Take a Snapshot, Assign to Snapshot Schedule, Assign to vApp, Convert to Template, Export, Install Guest Tools (+), Send Ctrl+Alt+Del, Delete VM, Properties |
| Storage | New Storage, Repair, Set as Default, Virtual Disks (New Virtual Disk, Attach Virtual Disk), Detach, Reattach, Forget, Destroy, Properties |
| Templates | New VM wizard, Create VM From Selection, Export to File, Copy, Delete Template, Properties |
| Tools | Options, External Tools (+), Convert Disk Image (+), Server Status Report, Inventory Report (+) |
| Window | One entry per open console window |
| Help | Help, View Log Files, Project website, About |

Toolbar, left to right: Back, Forward; Add Server, New Pool, New Storage, New VM; Start, Shut Down, Reboot, Resume, Suspend, Force Shut Down, Force Reboot.

### Command table

Each `CommandSpec` has: `id`, `label`, menu path, optional toolbar slot, the object kinds whose context menu shows it, optional shortcut, `supports_multi`, an eligibility function and an invoke function that returns an `Intent`.

- The menu bar, the toolbar and every context menu are generated from the table. A view never builds its own menu.
- Eligibility delegates to the checks the core already makes for display and submission. A disabled command carries a reason, shown as a tooltip and in the tree dump.
- A tab may host a local action row (for example Add and Detach above the disk grid). Each button there is still a `CommandSpec` whose target is the grid selection.
- On the GPUI side there is one action per command, declared with `actions!` and handled once on the shell.

### Selection and scope

- The tree selection sets the scope. A grid selection inside a tab sets the action target and does not change the scope.
- Selection is held as `ObjectId` (pool UUID, kind, object UUID) and survives inventory updates. When the object disappears, select its parent and show a status notice.
- Back and Forward step through `(selection, tab)` pairs.
- The last active tab is remembered per object kind.
- A multi-selection shows only the Search tab. Bulk commands come from the context menu and the menus.

## Component mapping

Every Rune and rune-kit construct in use has a GPUI Kit 0.9.1 counterpart except two: physical key input and a hidden cursor. Both are console concerns and belong to spike S1.

Paths are relative to `gpui_kit::component` unless they start with `cx.` or `window.`.

| Rune or rune-kit today | Sites | GPUI Kit target | Notes |
| --- | --- | --- | --- |
| `AppSpec`, `rune::App`, `fn view(&State, &mut Cx) -> El` | `main.rs` | `gpui_kit::application().run`, `gpui_kit::init`, `gpui_kit::open_window`, views that `impl Render` | Wrap the root view in `Root` |
| `el()`, `row()`, `col()`, `text()` | everywhere | `div()`, `h_flex()`, `v_flex()`, text children |  |
| `.key("name")` | everywhere | `.id("name")` then `.test_support()` | Keep the same strings. Place `.test_support()` before `.track_focus(..)` |
| `.label(..)`, `.role(..)` | accessibility | Accessibility label and role on the element | Keep the label strings; the AT-SPI driver uses them |
| `Button::new`, `ButtonVariant` | 585 | `button::Button` and its variants | Most become commands in menus and toolbars. Do not port one to one |
| `TextInput::new` with `TextEdit` | 132, 624 | `input::Input` with `Entity<InputState>`; subscribe to `InputEvent` | The draft `String` in the model is the source of truth |
| `TextArea::new` | 6 | `input::Textarea` with its state entity |  |
| `Checkbox::new` | 46 | `checkbox::Checkbox` |  |
| Rows of buttons used as a choice | many | `select::Select`, `radio`, `combobox` | Replace choice-by-buttons |
| `Table`, `TableColumn`, `TableState`, `Cell`, `SortDir` | 4 | `table::DataTable` with `TableState<D>` and a `TableDelegate` | Sort in `perform_sort`. Row menu through the delegate's `context_menu`. Hand-built row lists also become `DataTable` |
| `Tree`, `TreeItem`, `TreeModel` | 1 | `tree::tree(&state, render_item)` with `Entity<TreeState>`, `TreeItem::new(id, label).children(..)`, `.context_menu(..)` | Rebuild items from the model. Preserve expansion and selection by id |
| `SplitView`, `PaneState` | 1 | `resizable::h_resizable`, `v_resizable` | Persist widths to preferences |
| `Toolbar` | 1 | `toolbar::Toolbar`, `ToolbarGroup` |  |
| `MenuItem`, `rune_kit::context_menu` | 11 | `menu::ContextMenu`, `PopupMenu`, `DropdownMenu`; `menu::AppMenuBar` for the bar | Built from the command table |
| `rune_kit::root(base, [dialog])`, `.modal(..)` | 46, 51 | `Root`, `window.open_dialog`, `window.open_alert_dialog`, `window.close_dialog` |  |
| Multi-step dialogs | several | `dialog` with `stepper::Stepper`, `StepperItem` |  |
| Key and value detail lists | details pane | `description_list::DescriptionList` |  |
| `Page` enum and type tabs | `main.rs` | `tab::TabBar`, `Tab` | Tabs follow the tab matrix, not object types |
| `ScrollSpec` | 60 | Scrollable containers; `virtual_list` for long lists |  |
| `rune::Shape`, `rune::canvas` | `metrics_view.rs` | `chart::LineChart` or `AreaChart`; otherwise a `canvas` with `plot` primitives | Unavailable samples render as gaps, never as zero |
| `rune::Pixels`, `SurfaceFit` | console | Custom element that calls `window.paint_image` with an `Arc<RenderImage>`; release replaced frames with `window.drop_image` | Spike S1 measures cost |
| `on_key`, `on_key_up`, `.modifier_keys()` | console | `.on_key_down`, `.on_key_up`, `.on_modifiers_changed`, `cx.intercept_keystrokes` | Keysym path only |
| `on_physical_input`, `PhysicalKeyCode` | console | None | Spike S1 |
| `rune::Cursor::Hidden` | console | None; `CursorStyle` has no hidden variant | Spike S1 decides |
| `on_pointer_move_local`, `PointerButton`, `on_wheel` | console | `MouseMoveEvent`, `MouseDownEvent`, `MouseUpEvent` with `MouseButton::Left`, `Right`, `Middle`; `ScrollWheelEvent` with `ScrollDelta::Pixels` or `Lines` | Right-button drag becomes native. Remove the "Hold right button" control |
| `rune::FileDialogOptions` | 12 | `cx.prompt_for_paths`, `cx.prompt_for_new_path` | Tests use `simulate_new_path_selection` and `did_prompt_for_new_path` |
| `tx.read_clipboard`, clipboard writes | console, exports | `cx.read_from_clipboard`, `cx.write_to_clipboard` | Only on an explicit user action |
| URL requests | sidebar link, help | `cx.open_url`; tests read `cx.opened_url()` |  |
| `Waker` | `setup` | The channel bridge in "Target architecture" |  |
| `app.window(WindowSpec::from_app(..))` | console windows | `cx.open_window` |  |
| `.on_focus(..)`, `.on_close(..)` | app | `cx.observe_window_activation`, `window.on_window_should_close`, `cx.on_app_quit` |  |
| `theme.rs` | theme | `Theme`, `ThemeMode`, `ActiveTheme` | Carry the accent colour |
| Inline `Icon` SVG bodies | `main.rs` | `Icon` loaded through an `AssetSource` | Ship the project's own SVGs as assets |
| `rune::Runtime` tests | 98 | `#[gpui_kit::test]` with `gpui_kit::test::*` | See "Verification" |
| `--snap`, `--tree`, scenes | `check.sh` | `HeadlessAppContext`, `capture_screenshot`, element snapshots | See "Verification" |

### Adapter and shared widgets

Keep every Kit import that views use behind one module, `crates/xenadmin-gpui/src/kit.rs`, so a Kit upgrade touches one file. Build these shared widgets once in Phase 2 and use them everywhere:

| Widget | Purpose | Replaces |
| --- | --- | --- |
| `ObjectGrid` | `DataTable` bound to model rows, with sort, multi-select, context menu and column widths persisted to preferences | The inventory table and every hand-built row list |
| `PropertyList` | `DescriptionList` sections with copyable values | The details pane |
| `FormRow`, `FieldError` | Label, control, help text, validation message | Ad hoc label and input rows |
| `ReviewPanel` | Affected resources, consequences, required confirmation | Every review step |
| `OutcomeBanner` | Progress, partial result, unknown outcome, with Recheck and Acknowledge | Per-dialog outcome text |
| `ExperimentalNotice` | The notice `docs/FEATURES.md` requires | The notice above each experimental dialog |
| `WizardFrame` | `Dialog` with `Stepper`, Back, Next, Cancel and a final review | Long dialogs |
| `PropertiesFrame` | `Dialog` with a vertical page list | Property dialogs |

## Phase 0: spikes and gate G0

Six spikes run in parallel and gate everything else. S1 is the only one whose failure can cancel the migration.

Spike code lives in `crates/xenadmin-gpui/examples/` on a branch. Each spike ends with a report at `docs/reports/<date>-gpui-spike-<id>.md` that states what was run, on which system, and what was not established.

| Spike | Question | Blocks |
| --- | --- | --- |
| S0 | Does the pinned Kit build under this workspace's toolchain, lints and CI? | Everything |
| S1 | Can the console meet its input and display requirements on GPUI? | The migration |
| S2 | Can scenes render to PNG and an element tree headlessly, deterministically, in CI? | Phase 2 onward |
| S3 | Does the core-to-GPUI bridge stay responsive with a large inventory? | Phase 2 |
| S4 | Do the shell primitives behave: menus, context menus, tabs, dialogs, wizards, file prompts, keyboard? | Phase 2 |
| S5 | Does a packaged build start and integrate on the target desktops? | Phase 3 gate |

### S0: build and policy

1. Add `gpui-kit = "=0.9.1"` to `[workspace.dependencies]`. Create `crates/xenadmin-gpui` with the Kit's hello window.
2. Build with the repository toolchain and `--locked`. Run clippy with `-D warnings`.
3. If a GPUI macro expansion conflicts with `unsafe_code = "forbid"`, set that lint to `deny` for `xenadmin-gpui` only and record why. The core and `xenadmin-ui` keep `forbid`.
4. Confirm Rune and GPUI build side by side in one workspace.
5. Record: clean and incremental build time, release binary size, crates added to `Cargo.lock`, and the exact system packages needed on Arch and on Ubuntu 24.04.

Pass: `cargo build --locked --workspace` and clippy succeed on both distributions and in CI.

### S1: console

Requirements, taken from `console_view.rs` and `docs/decisions/0003-console-protocol-adapter.md`:

1. Show the latest complete framebuffer in Fit and in Actual size. No stale frame after a console is replaced.
2. Key down and key up for every key, including auto-repeat.
3. Physical key identity, sent as XT make codes, when the server negotiated QEMU Extended Key Events. Keysyms otherwise.
4. A discrete press and release for each modifier, left and right distinguished.
5. CapsLock, NumLock, ScrollLock, PrintScreen, Pause and ContextMenu are delivered.
6. Held keys are tracked. All input is released on focus loss, window deactivation and close.
7. While the console has focus, no application binding, menu accelerator or focus traversal consumes a key.
8. Pointer motion in canvas-local coordinates; left, middle and right button down and up; wheel with pixel or line deltas; Shift+wheel pans in Actual size.
9. The guest cursor is drawn with its hotspot and the local cursor is hidden over the canvas.
10. Fullscreen, and detaching into an independent window, up to eight.
11. Clipboard actions and "type clipboard as keys" work as today.

What GPUI 0.3.12 gives, read from source:

- `KeyDownEvent` and `KeyUpEvent` carry only `Keystroke { modifiers, key, key_char }`.
- `gpui-pre-linux/src/linux/wayland/client.rs` receives `wl_keyboard::Event::Key { key, state }`, does not forward modifier keys as key events, and drops the raw code.
- Modifier changes arrive as `ModifiersChangedEvent { modifiers, capslock }`, with no left or right.
- `Window` implements `HasWindowHandle` and `HasDisplayHandle`.
- `MouseDownEvent.button` covers `Left`, `Right` and `Middle`. `ScrollDelta` is `Pixels` or `Lines`.
- `CursorStyle` has no hidden variant.
- `cx.intercept_keystrokes` observes keystrokes before bindings are dispatched.

Routes for requirements 3 to 5, in the order to try them:

**Route A: second keyboard listener.**

1. Create `crates/xenadmin-keys`. It is the only new crate that may contain `unsafe`, documented the way `xenadmin-process` documents its boundary.
2. Take the console window's Wayland display from its raw display handle. Wrap it as a foreign display with `wayland-client`, create a private event queue, bind the seat and request a second `wl_keyboard`.
3. Record `Enter`, `Leave`, `Key { key, state }` and `Modifiers` for the console window's surface.
4. GPUI's loop reads the socket. Drain the private queue with `dispatch_pending` from the console element's key and modifier handlers and from a per-frame tick while the console has focus.
5. Re-key the existing `scancode()` table from `PhysicalKeyCode` to evdev codes. Put it in `xenadmin-ui`.
6. Emit `ConsoleInput::PhysicalKey { keysym, keycode, down }` exactly as today.
7. Test event ordering against GPUI's keystrokes, a keymap change, two console windows, and focus arriving with keys already held.
8. Optional, only if small: request the compositor's keyboard-shortcuts-inhibit protocol for the focused console, and hide the pointer with `wl_pointer.set_cursor`.

**Route B: patched GPUI.**

1. Vendor `gpui-pre` and `gpui-pre-linux` 0.3.12 under `vendor/` with `[patch.crates-io]`, as `vendor/vnc-rs` is today.
2. Add an opt-in per-window raw key callback fired from `wl_keyboard::Event::Key` before keystroke translation, for modifier keys too, carrying the evdev code and the pressed state. Do not change `Keystroke`.
3. Add a hidden cursor style if it is small.
4. Record the patch in a decision record. It must be re-applied on every Kit upgrade.

**Route C: keysyms only.** Derive keysyms from `Keystroke` and modifier presses from state changes. This loses extended key events and left or right modifiers. It is a regression of a supported area and needs an explicit operator decision.

S1 test matrix, on the lab's graphical guest and plain guest. Drive input with the compositor's physical key injection, as `docs/VERIFICATION.md` requires. Take proof from the guest's own capture of its input devices.

| Area | Check |
| --- | --- |
| Printable keys | Every key of a US layout, both shift states, arrives as the same physical key |
| Other host layout | With extended keys negotiated, a German or French host layout delivers the same physical positions |
| Function and navigation | F1 to F12, arrows, Home, End, Page Up, Page Down, Insert, Delete, keypad with NumLock on and off |
| Modifiers | Left and right Shift, Ctrl and Alt, AltGr, Super, each as its own press and release |
| Special keys | CapsLock, NumLock, ScrollLock, PrintScreen, Pause, ContextMenu |
| Chords | Ctrl+C, Ctrl+Alt+F2, Ctrl+Alt+Del through the button |
| Application keys | Tab, Escape, F10, Alt+letter and Ctrl+W reach the guest and trigger nothing in the client |
| Repeat and release | Auto-repeat works. Focus loss produces key-up for every held key |
| Pointer | Guest coordinates within 1 pixel at 100% and when scaled; right-button drag; wheel steps |
| Frames | A 1920x1080 guest updating 30 times a second for 60 seconds: no growth in memory, latency p95 recorded, on a GPU and on lavapipe |
| Windows | Two detached consoles typed into alternately; no cross-delivery |
| Fallback | A server without extended keys still receives correct keysyms |

Pass: requirements 1 to 8, 10 and 11 hold through route A or route B. Requirement 9 may ship with a visible local cursor if the report records it.

Kill: if neither route A nor route B meets requirements 3 to 5, stop the migration and report. Do not adopt route C without an operator decision.

### S2: headless verification

1. Enable `gpui-kit/test-support` for tests and for the scene tool.
2. Render a scene offscreen: `HeadlessAppContext::with_platform(text_system, assets, gpui_kit::platform::current_headless_renderer)`, open a window with `show: false` and `focus: false`, call `window.render_frame(cx)`, then `cx.capture_screenshot(handle)` and write a PNG.
3. Dump the element tree as JSON from the Kit's element snapshots: id path, role, label, value, bounds, focus, checked, disabled.
4. Write one `#[gpui_kit::test]` that finds an element by id, clicks it, presses keys and asserts model state.

Pass: two renders of the same scene are byte-identical; the scene tool runs in CI on Mesa lavapipe; render time per scene is recorded. Known limit: the Kit cannot enumerate anonymous rendered text, so assertions use ids and accessibility labels.

### S3: runtime bridge and scale

1. Connect `Backend` and the demo inventory to `AppStore` through the channel bridge.
2. Port the `demo` and `large-inventory` fixtures (16 hosts, 2,000 VMs).
3. Show a `Tree` and a `DataTable` and apply 100 synthetic inventory updates a second for 60 seconds.

Pass: no `block_on` in `xenadmin-gpui`; selection by id survives updates; scrolling stays responsive, with frame times recorded; no repaint while nothing changes; shutdown runs in the documented order.

### S4: shell primitives

1. `AppMenuBar` with three menus built from a stub command table, with disabled and checked items that change with the selection.
2. Context menus on tree rows and table rows.
3. `TabBar` switching views. A `Dialog` with a validated form. A `Stepper` wizard with Back, Next and a review. An `AlertDialog` confirmation.
4. Portal file prompts for open and save. Record what happens when no portal is running.
5. Keyboard: Tab visits controls, arrows move in `Tree` and `DataTable`, Enter activates, Escape closes a dialog.
6. Light and dark themes with the accent. Interface scale at 100% and 125%.

Pass: each item works in a native Wayland session and in a `#[gpui_kit::test]`.

### S5: desktop integration and packaging

1. Native Wayland sessions on Arch and Ubuntu 24.04. An X11 start-up check only; X11 stays experimental.
2. Start-up with software rendering and no GPU.
3. AT-SPI: the lab's driver can discover and activate controls, and labels match the strings carried from Rune.
4. Single instance: a second launch activates the first.
5. Client-side decorations, display scale 1 and 2, minimum window size.
6. `scripts/package.sh --appimage` produces the archive and AppImage. `scripts/collect-notices.py` and `packaging/test_notices.py` cover the new dependency graph, including assets shipped by `gpui-kit-assets`.

Pass: a clean-install run on both distributions, recorded like `docs/reports/2026-11-11-clean-install.md`.

### Gate G0

- S0 to S5 pass, or each deviation is recorded and accepted.
- The S1 route is chosen and written as a decision record.
- `docs/decisions/0005-gpui-kit-front-end.md` is merged.

## Phases 1 to 5

Each phase ends at a gate with recorded evidence. Inside Phases 2 to 4, work packages run in parallel once their dependencies are merged. Phase 1 does not depend on GPUI and may run alongside Phase 0. Phase 2 starts only after both G0 and G1.

| Phase | Delivers | Exit gate |
| --- | --- | --- |
| 0 Spikes | Evidence that GPUI Kit can carry the console, the checks, the bridge, the shell and packaging | G0 |
| 1 Model | `xenadmin-ui` with the shared model, command table and scene registry; the Rune front end uses the shared parts | G1 |
| 2 Shell | A GPUI window with menus, toolbar, navigation, tabs and status bar; Home, General and Search; connections and trust | G2 |
| 3 Supported core | Everything `docs/FEATURES.md` lists as supported | G3, the recovery gate on the GPUI build |
| 4 Experimental areas | The remaining ledger rows, in six waves | G4 |
| 5 Cutover | Rune removed, binary renamed, documents updated, release | G5 |

### Phase 1: model extraction

The model is extracted in two tiers so that the released Rune front end is not rewritten on its way out.

- **Tier A, shared now.** Code with no text buffers. The Rune crate switches to it in this phase, and its existing tests prove the extraction changed nothing.
- **Tier B, ported with each view.** Draft and editor structs that hold `TextEdit`. They are re-declared in `xenadmin-ui` with `String` fields when their view is ported in Phases 2 to 4. The Rune crate keeps its own copy until it is deleted.

Steps:

1. Create `crates/xenadmin-ui` with the workspace lints and the MIT licence.
2. Move the Rune-free files unchanged (the `grep -L` command in "Current state" lists them). Move their tests with them.
3. Extract from `main.rs`: `Node` and tree building, `DisplayRow` with row projection, sort and filter, the demo fixtures (`demo`, `demo_sized`), selection handling, and the part of `poll` that reads core handles.
4. Add `Selection`, `TabId`, `TabSet` and the tab matrix as a pure function. Test every row of the matrix.
5. Build the command table. Enumerate every action reachable in the Rune interface: each `Button::new` that submits or opens a surface, and each `MenuItem`. Give each a `CommandId`, a menu path and its object kinds. Implement eligibility by calling the checks that exist today.
6. Add `Intent`, `Surface`, `DialogId`, `WizardId` and `PageId`.
7. Add `xenadmin_ui::scenes` with all 128 scene names. Mark each as ported or not ported. The Tier A scenes (`demo`, `large-inventory`, `host-version-notice`, `unavailable-actions`) are ported in this phase.
8. Move `Preferences` to `xenadmin-ui`. The stored file format does not change.
9. Switch `xenadmin-desktop` to Tier A from `xenadmin-ui`.
10. Add the dependency guard to `scripts/check.sh`.
11. Create `docs/GPUI-MIGRATION-LEDGER.md` (format in "Repository and documentation changes").
12. Freeze `xenadmin-desktop`. Only release-blocking fixes are allowed, and each adds a row to the ledger's mirror list.

Gate G1:

- `./scripts/check.sh` passes with the Rune front end using Tier A.
- `cargo tree -p xenadmin-ui` shows no `rune`, `rune-kit` or `gpui` crate.
- The tab matrix tests pass.
- The ledger lists every command, every scene and every desktop test.
- `git diff` shows no change under `crates/xenadmin-core` or `crates/xenadmin-xapi`.

### Phase 2: shell

| Package | Deliverable | Depends on |
| --- | --- | --- |
| P2.1 | App skeleton: `AppStore`, bridge, `Root`, theme, preferences, shutdown order, `--version`, `--demo`, `--scene`, single instance, development inspection socket | G0 and G1 |
| P2.2 | Scene tool: PNG and tree dump for any scene; CI job and artifacts | P2.1 |
| P2.3 | Menu bar and command dispatch: actions, `cx.set_menus` rebuilt on eligibility change, disabled reasons | P2.1 |
| P2.4 | Toolbar | P2.3 |
| P2.5 | Navigation pane: view switcher, `Tree` with context menus, search filter, selection by `ObjectId`, expansion kept across updates | P2.3 |
| P2.6 | Object header, tab strip, Back and Forward, last tab per kind | P2.5 |
| P2.7 | Status bar | P2.1 |
| P2.8 | Shared widgets from "Component mapping" and `kit.rs` | P2.1 |
| P2.9 | Home tab, Add Server, certificate review, credentials, Reconnect As | P2.8 |
| P2.10 | General tab for every object kind | P2.6, P2.8 |
| P2.11 | Search tab and the Objects view: `ObjectGrid` with sort, multi-select and persisted column widths | P2.6, P2.8 |

Gate G2:

- The GPUI build connects to the lab pool with a pinned certificate and shows live inventory updates.
- Selection stays on the same object across updates.
- These scenes render through the scene tool: `demo`, `demo` in light theme, `connect`, `connect-insecure`, `large-inventory`, `connections`, `connection-settings`, `connection-trust`, `certificate-review`, `host-version-notice`, `inventory-context`, `general-information`, `search-pools`, `unavailable-actions`, `interface-large`.
- A keyboard-only interaction test reaches the tree, the tab strip, the grid and a menu command.
- Review finds no action button outside the toolbar, a tab's local action row, or a dialog.

### Phase 3: supported core

| Package | Scope | Rune modules |
| --- | --- | --- |
| P3.1 | VM power: Start, Shut Down, Reboot, Suspend, Resume, force actions with review, Start on Server, batch power | Power handling in `main.rs`, `bulk_power.rs`, `vm_recovery_view.rs` (force part), `vm_placement.rs` |
| P3.2 | Events: progress, partial and unknown outcomes, recheck, acknowledgement, history, hide and restore, export; status bar counters; failure help | `activity.rs`, `activity_export.rs`, `failure_help.rs` |
| P3.3 | Console tab for VM and host, recovery, screenshot export, clipboard actions, typing the clipboard, fullscreen, detached windows, Window menu | `console_view.rs`, `console_recovery.rs`, `console_export.rs`, `host_console.rs`, `console_windows.rs` |
| P3.4 | Snapshots tab: create, inspect, restore, delete | `vm_snapshots.rs`, `snapshot_inspection.rs`, `snapshot_capture.rs` |
| P3.5 | Performance tab | `metrics_view.rs`, `metric_presets.rs` |
| P3.6 | Host maintenance: enter and leave with the evacuation review, restart toolstack, reboot and shut down through maintenance | `host_admin.rs`, supported subset only |
| P3.7 | Move a VM within its pool: live, with or without disks; halted disk move | `vm_migration.rs` |
| P3.8 | Connection recovery in the interface: stale state, session renewal, coordinator change, changed certificate | `profile_reconnect.rs`, `profiles_view.rs`, status bar |

Gate G3:

1. The recovery gate from `README.md`, run with the packaged GPUI AppImage on an empty state directory: with XO shut down, locate the XO VM, start it and use its console.
2. For each supported area, its successful path and its recorded failure or interruption paths are re-run on the lab through the GPUI build wherever the path involves the front end. That includes a client closed in the middle of an operation and reconciled from the journal on restart, a cancelled migration, and each refused review.
3. One driven desktop session (nested compositor, AT-SPI, physical key injection) covers connections, tables, search, console keyboard input and the host console.
4. Every scene and interaction test of the supported areas is ported.
5. `./scripts/check.sh` still passes for the Rune build.

### Phase 4: experimental areas

Waves are independent of each other. Inside a wave, port the tab first, then dialogs, then wizards.

| Wave | Area | Rune modules |
| --- | --- | --- |
| W1 | VM administration | `vm_properties.rs`, `vm_placement.rs`, `vm_devices.rs`, `vm_device_actions.rs`, `vm_device_lifecycle.rs`, `vm_device_properties.rs`, `disk_workspace.rs` (VM side), `vm_lifecycle.rs`, `vm_creation.rs`, `vm_creation_cloud.rs`, `cloud_init_view.rs`, `vm_deletion.rs`, `bulk_delete.rs`, `bulk_properties.rs`, `bulk_properties_model.rs`, `guest_tools_view.rs`, `vm_recovery_view.rs` (recovery boot), `external_tools.rs`, `external_tool_command.rs` |
| W2 | Storage | `storage_view.rs`, `storage_local_view.rs`, `storage_maintenance_view.rs`, `storage_graph_view.rs`, `disk_workspace.rs` (repository side) |
| W3 | Networking | `network_admin.rs`, `resource_admin.rs` (network creation), `network_interfaces.rs`, `network_service_ui.rs`, `network_topology_ui.rs`, `network_fabric_ui.rs`, `network_fabric_pool_ui.rs`, `management_network_ui.rs`, `management_bond_ui.rs`, `management_ipv6_bond_ui.rs` |
| W4 | Host and pool administration | `host_admin.rs` (remainder), `host_power_view.rs`, `host_support_view.rs`, `host_monitoring_view.rs`, `pool_view.rs`, `ha_policy_view.rs`, `pool_security_view.rs`, `session_permissions_view.rs`, `pool_dissolve_view.rs`, `profile_pool_join.rs`, `profile_root_password.rs`, `resource_admin.rs` (pool administration) |
| W5 | Transfers and cross-pool moves | `transfers.rs`, `ovf_transfer_ui.rs`, `local_conversion_view.rs`, `bulk_export.rs`, `bulk_export_snapshots.rs`, `cross_pool_vm.rs`, `cross_pool_vm_view.rs`, `bulk_cross_pool.rs`, `bulk_move.rs` |
| W6 | Organization, schedules, alarms, tools | `organization_view.rs`, `appliance_view.rs`, saved-search import and export in `search_view.rs` and `search_files.rs`, `snapshot_policy_view.rs`, `alarms_view.rs`, `settings.rs`, `inventory_report.rs` |

Gate G4:

- Every ledger row is marked ported, with its scenes and interaction tests.
- Every experimental surface shows its notice.
- A ledger row keeps its qualification only if its live evidence did not depend on the Rune interface. Otherwise it is marked "front end re-run pending".

### Phase 5: cutover

1. Delete `crates/xenadmin-desktop` and the `rune` and `rune-kit` workspace dependencies.
2. Rename `crates/xenadmin-gpui` to `crates/xenadmin-desktop`. The binary is `xenadmin-rs`.
3. Read the existing preferences file. Map `sidebar_width` and `columns`. Ignore `inspector_width`.
4. Apply every edit listed in "Repository and documentation changes".
5. Run the clean-install check on Arch and Ubuntu 24.04 with the archive and the AppImage.
6. Write the release report and the changelog entry. State the layout change, the minimum window size and the accessibility status.

Gate G5:

- `./scripts/check.sh` passes and `cargo tree --workspace` shows no `rune` crate.
- The packaged recovery gate is repeated on the release artifact.
- CI builds the archive and AppImage with complete notices.

### Definition of done for a work package

1. The model part is in `xenadmin-ui`, with the pure tests of the Rune module moved or ported.
2. The view is in `crates/xenadmin-gpui/src/<area>/` and uses the shared widgets.
3. Its commands are in the command table with eligibility and reason strings.
4. Its scenes are ported under their existing names and render through the scene tool.
5. Its `rune::Runtime` tests are ported to `#[gpui_kit::test]`, or retired with a reason in the ledger.
6. An experimental surface shows its notice.
7. Review, partial-outcome and unknown-outcome behaviour matches the Rune module. Recheck never replays a write.
8. The ledger rows are updated.
9. `crates/xenadmin-core` and `crates/xenadmin-xapi` are untouched.

### Rules for parallel work

- One package is one branch and one pull request.
- A package edits its own area directory, plus one file each for its commands (`xenadmin-ui/src/commands/<area>.rs`), its scenes (`xenadmin-ui/src/scenes/<area>.rs`) and its ledger rows.
- `kit.rs` and the shared widgets change only in pull requests owned by P2.8.
- Lab runs follow the verification boundaries in `AGENTS.md`: the designated disposable lab only, and never a production host.
- When a package is blocked, record the blocker in the ledger and take another package. Do not work around a blocker by editing the core.

## Verification

Three layers replace today's checks one for one: model unit tests, GPUI interaction tests and headless scene renders. Live lab evidence is re-recorded only where it depended on the front end.

### What replaces what

| Rune today | GPUI Kit replacement |
| --- | --- |
| `xenadmin-rs --snap out.png --scene X --theme T --size WxH --scale 1 [--at 1]` | The scene tool: `HeadlessAppContext::with_platform(.., gpui_kit::platform::current_headless_renderer)`, a window opened with `show: false`, `window.render_frame(cx)`, `cx.capture_screenshot(handle)` |
| `--tree out.json` | JSON dump of the Kit's element snapshots: id path, role, label, value, bounds, focus, checked, disabled |
| `rune::Runtime::new(init(), view, size, Env { system_dark, scale })` | `#[gpui_kit::test]` with `TestAppContext`; open a window at that size and theme; `simulate_window_scale_factor_change` for scale |
| `runtime.frame(dt)` | `window.render_frame(cx)`, `cx.run_until_parked()`, `wait_for` for deferred effects |
| `runtime.find("key")` | `window.find("key")` or `window.try_find("key")`, returning an `ElementSnapshot` with bounds |
| `runtime.pointer_down(x, y)`, `pointer_up(x, y)` | `window.click("key", cx)`; `TestEventExt::pointer_down`, `pointer_move`, `pointer_up` for gestures |
| Key input through the runtime | `window.press("enter", cx)`, `window.input("text", cx)`, `TestEventExt::key_down`, `key_up` |
| `runtime.state.<field>` | `store.read_with(cx, ..)` on the model |
| `rt.file_dialog()`, `rt.resolve_file_dialog(..)` | `cx.did_prompt_for_new_path()`, `cx.simulate_new_path_selection(..)` |
| `runtime.take_url_requests()` | `cx.opened_url()` |
| CI artifact `rune-ui` | CI artifact `gpui-ui` |

### Commands

The scene tool's binary name and flags are fixed by package P2.2. The proposal keeps today's flags.

```sh
# model
cargo test -p xenadmin-ui --locked

# interaction tests: headless windows, no GPU
cargo test -p xenadmin-gpui --locked

# scene renders: needs a Vulkan or GL adapter; Mesa lavapipe is enough
cargo run -p xenadmin-gpui --bin xenadmin-scenes --features scenes --locked -- \
  --scene demo --theme dark --size 1440x900 --scale 1 \
  --snap artifacts/gpui/demo.png --tree artifacts/gpui/demo.json

# guards
! cargo tree -p xenadmin-ui -e normal --locked | grep -E '(rune|gpui)'
! grep -rn 'block_on' crates/xenadmin-gpui/src
```

### Porting the 370 desktop tests

- A test that does not construct `rune::Runtime` moves to `xenadmin-ui` with the logic it tests.
- A test that constructs `rune::Runtime` is rewritten as a `#[gpui_kit::test]` using the table above.
- A test of Rune-specific layout may be retired. The ledger records the reason.
- The ledger lists every test with its disposition: moved, ported or retired.
- Every element a test or the AT-SPI driver needs has `.id(..)` and `.test_support()`.
- The Kit has no query for anonymous rendered text. Assert through ids, accessibility labels and model state.
- Scene sizes today are 1440x900 and 513x652. The compact size changes with the minimum window size; see "Decisions and open questions".

### What headless checks do not establish

The rule in `AGENTS.md` carries over unchanged. Headless renders and interaction tests do not establish real Wayland input, the system clipboard, physical keys and scancodes, portal dialogs, or Orca behaviour. Those need a real or nested session.

### Live evidence

| Evidence was recorded through | Status after the port |
| --- | --- |
| The core facade: `crates/xenadmin-core/tests/lab_live.rs`, `tools/host-probe` | Stays valid |
| The packaged Rune desktop, a driven desktop session, or the operator's seat | Supported areas: re-recorded with the GPUI build at G3. Experimental areas: marked "front end re-run pending" |

The driven-session harness keeps its method: a nested compositor with a headless output, AT-SPI for discovery and activation, physical key injection, and guest-side proof. Update its element lookups for the new tree. Keep accessibility labels identical to the Rune labels wherever the control still exists.

### CI

- Keep the Rune job until Phase 5. Add a GPUI job that runs the model tests, the interaction tests and the scene renders, and uploads `artifacts/gpui/*.png` and `*.json`.
- Add the system packages that S0 recorded.
- Raise the job timeout only if S0's measured build time requires it.
- At cutover, rename the workflow from "Rust and Rune checks" and remove the Rune job.

### Performance and accessibility records

- `large-inventory`: scroll and selection frame times, recorded at G2 and G5.
- Console: the S1 frame test, repeated at G3 on the packaged build.
- Idle: no repaint while nothing changes.
- Accessibility: store the tree dump of the main scenes. `docs/FEATURES.md` keeps "screen readers: not supported" until a screen reader has actually been used and the run is recorded.

## Dependencies, build, packaging and licensing

Pin `gpui-kit` exactly and reach GPUI only through it. The Kit pins its GPUI snapshot exactly, so one version line fixes the whole stack.

```toml
# Cargo.toml (workspace), during Phases 1 to 4
[workspace]
members = [
  "crates/xenadmin-core", "crates/xenadmin-ui", "crates/xenadmin-gpui",
  "crates/xenadmin-desktop", "crates/xenadmin-process", "crates/xenadmin-xapi",
  "tools/host-probe",
]

[workspace.dependencies]
gpui-kit = { version = "=0.9.1" }   # default features: component, assets
```

```toml
# crates/xenadmin-gpui/Cargo.toml
[dependencies]
xenadmin-core = { path = "../xenadmin-core" }
xenadmin-ui = { path = "../xenadmin-ui" }
gpui-kit.workspace = true

[dev-dependencies]
gpui-kit = { workspace = true, features = ["test-support"] }
```

| Crate | Version | Role |
| --- | --- | --- |
| `gpui-kit` | 0.9.1, published 2026-11-09 | Facade. GPUI is `gpui_kit::*`; layers are `gpui_kit::platform`, `gpui_kit::base`, `gpui_kit::component`, `gpui_kit::assets`, `gpui_kit::test` |
| `gpui-component` | 0.9.1 | Styled components |
| `gpui-base` | 0.9.1 | Unstyled behaviour and state |
| `gpui-pre` | 0.3.12, pinned `=0.3.12` by the Kit | GPUI itself, a published snapshot of the Zed repository |
| `gpui-pre-linux` | 0.3.12 | Wayland and X11 back ends; wgpu over Vulkan or OpenGL; AccessKit over AT-SPI; desktop portals through `ashpd` |
| `gpui-pre-wgpu` | 0.3.12 | Renderer, including the headless renderer used for scene renders |

The Kit released 0.6.0 on 2026-09-03 and 0.9.1 on 2026-11-09, about one release a week. Version 0.6.0 alone renamed or redesigned the table, dialog, dock and input APIs. Treat every upgrade as work.

### Dependency rules

- Do not add `gpui`, `gpui-pre` or `gpui-component` as direct dependencies. Route B of spike S1 is the only exception.
- Do not enable the Kit's `gpui-fast` feature. It swaps in a different GPUI fork and needs its own spike.
- Do not enable `tree-sitter*`, `syntect`, `speech` or `decimal`.
- The scene tool needs `test-support`. Put it behind a crate feature named scenes that only the scene binary requires, so the release binary is built without it.
- `Cargo.lock` stays committed. Every command uses `--locked`.
- Upgrade the Kit only between phases, in one pull request. Re-run S0 and S2 and the full test suite. Update the reviewed version in `docs/SOURCES.md`. If route B is active, re-apply the patch first.

### System packages

CI installs today: `build-essential pkg-config libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 libfontconfig1-dev libvulkan1 mesa-vulkan-drivers desktop-file-utils`. GPUI's Linux back end also builds X11 support through `x11rb` and `xkbcommon`'s X11 feature, so expect development packages for libxcb and libxkbcommon-x11. S0 records the exact list for Arch and Ubuntu 24.04, and `docs/RUNNING.md` takes it from there.

At run time the client needs a Vulkan loader or OpenGL. Software rendering through Mesa is accepted by the renderer; S5 confirms it on a machine without a GPU.

### Packaging

- `scripts/package.sh`, `packaging/appimage.py` and `packaging/appimage-inputs.json` keep their roles. Add any run-time library S5 finds missing.
- The window's `app_id` matches `packaging/xenadmin-rs.desktop`, so the compositor shows the installed icon.
- Kit assets (icons, default fonts) are embedded through the `assets` feature. The project's own icons ship through an `AssetSource`.
- The installer and removal flow in `docs/INSTALLATION.md` does not change.

### Licensing

These notes are general information for keeping the notices complete. They are not legal advice.

- GPUI Kit and `gpui-pre` are published under Apache-2.0. Original work in this repository stays MIT. `crates/xenadmin-xapi` stays LGPL 2.1 with its linking exception.
- Binary archives already carry dependency and embedded-font notices. `scripts/collect-notices.py` and `packaging/test_notices.py` must cover the new dependency graph, including the Apache-2.0 texts and whatever `gpui-kit-assets` bundles.
- If route B vendors patched GPUI crates, keep their licence files and state in each changed file that it was modified.
- This plan copies no code or assets from the reference client. The rule in `AGENTS.md` about preserving its licence still applies if that ever changes.

## Risks, mitigations and kill criteria

One risk can cancel the migration: console key fidelity. The rest change cost or scope, not the decision.

| # | Risk | How it shows | Mitigation | If it holds |
| --- | --- | --- | --- | --- |
| R1 | GPUI gives no physical key codes or left and right modifiers | S1 matrix rows fail | Route A, then route B | Kill: stop at G0 if both routes fail. Route C only by operator decision |
| R2 | Kit churn: weekly releases, exact GPUI pin, renamed APIs | An upgrade breaks many files | Exact pin; upgrades only between phases; all Kit imports behind `kit.rs`; shared widgets | Stay on 0.9.1 until cutover |
| R3 | No hidden cursor style | Two cursors over the console | Route A or B may add hiding | Ship with a visible local cursor and record it |
| R4 | Two front ends drift during the port | A fix lands in one only | Rune crate frozen after G1; mirror list in the ledger | Shorten Phase 4 by deferring low-use experimental rows |
| R5 | The old structure returns: a dialog per feature, button rows | Review finds actions outside the command table | The rules in "Summary"; G2 review item; shared widgets | Reject the package |
| R6 | Headless renders differ between machines | PNG bytes differ across runners | Compare tree JSON for assertions; keep PNGs as artifacts, not goldens | Assertions rely on interaction tests and tree dumps only |
| R7 | Large inventories or console frames are slow | S3 or S1 frame times are poor | Virtualized `DataTable` and `Tree`; drop replaced frame images; console in its own entity | Limit console update rate; record the limit |
| R8 | GPUI macros conflict with `unsafe_code = "forbid"` | S0 fails to compile | `deny` for `xenadmin-gpui` only, recorded | None needed beyond the record |
| R9 | No desktop portal in a minimal session | File prompts fail | S4 records the behaviour; show a clear error and accept a typed path | Typed-path fallback stays |
| R10 | No Vulkan on a workstation | Start-up fails or falls back | S5 checks OpenGL and software paths; AppImage inputs updated | Document the run-time requirement |
| R11 | Secrets linger in input buffers | A password stays in a widget state after submit | Views clear the buffer when they hand the secret to the model; masked inputs; no clipboard history for them | Record the residual exposure; it exists today in `TextEdit` too |
| R12 | The compact layout is lost | Scenes at 513x652 no longer fit | Decide the minimum size (see next section); collapse the navigation pane on narrow windows | Minimum 800x600 |
| R13 | Front-end evidence must be re-recorded | G3 takes longer than the port | Re-record only supported areas at G3; mark the rest pending | Experimental rows keep their pending mark |
| R14 | The Wayland side listener in route A is fragile across compositors | Works on one compositor only | Test on the compositors of both target distributions in S1 | Fall to route B |
| R15 | X11 has no raw key path | Console on X11 loses extended keys | X11 stays experimental; keysym path there | Stated in release notes |

### If the migration stops at G0

Phase 1 is still worth doing. The tab matrix, the command table and the `xenadmin-ui` model do not depend on GPUI. With them, either restructure the Rune interface around the same information architecture, or evaluate egui, whose test harness and inspection tooling are the closest substitute for the scene checks. That choice returns to the operator.

## Repository and documentation changes

The repository's own documents name Rune as the front end in about a dozen places. Each is listed here with the phase that changes it, so no agent has to search for them.

### New files

| Path | Content | Phase |
| --- | --- | --- |
| `docs/GPUI-MIGRATION.md` | This plan, exported as Markdown | 0 |
| `docs/GPUI-MIGRATION-LEDGER.md` | Commands, scenes, tests, mirror fixes, deviations | 1 |
| `docs/decisions/0005-gpui-kit-front-end.md` | Decision record: context, choice, alternatives considered (stay on Rune, egui, GTK4, Qt), consequences | 0 |
| `docs/decisions/0006-console-key-codes.md` | The route chosen in S1 and its maintenance cost | 0 |
| `docs/reports/<date>-gpui-spike-s0.md` to `-s5.md` | Spike reports | 0 |
| `docs/reports/<date>-gpui-gate-g1.md` to `-g5.md` | Gate evidence | 1 to 5 |
| `crates/xenadmin-ui/` | Model crate | 1 |
| `crates/xenadmin-gpui/` | GPUI front end | 0 (skeleton), 2 onward |
| `crates/xenadmin-keys/` | Raw key codes, only with route A | 0 |

### Edits

| File | Change | Phase |
| --- | --- | --- |
| `Cargo.toml` | Add members and `gpui-kit`; at cutover remove `rune`, `rune-kit` and the old member | 0, 1, 5 |
| `scripts/check.sh` | Add the dependency guards, the model and GPUI tests and the scene renders; at cutover remove the Rune renders | 1, 2, 5 |
| `.github/workflows/ci.yml` | Add the GPUI job and packages; at cutover rename the workflow and the `rune-ui` artifact | 2, 5 |
| `.github/workflows/release.yml` | Package the GPUI binary at cutover | 5 |
| `packaging/*` | Run-time inputs, notices, desktop entry check | 0, 5 |
| `AGENTS.md` | "with Rune and Rust" becomes GPUI Kit; "Keep Rune and platform-window types inside the desktop adapter" names GPUI; "Pin Rune to a reviewed full commit" becomes the exact Kit pin; the headless-screenshot rule names GPUI. Add the rules from "Summary" | 0 for the rules, 5 for the rest |
| `README.md` | Description, screenshots, build and run commands, the Development section's toolchain and Rune pin | 5 |
| `docs/PLAN.md` | Link this plan; "The new UI uses Rune"; the checks named "Rune" in "Current state" | 0, 5 |
| `docs/ARCHITECTURE.md` | "Deployment and ownership" (Rune owns the main thread), the workspace table, the console and metrics adapter section (`Pixels`), `console_windows.rs`, every "wakes Rune" | 2 for the new crates, 5 for the rest |
| `docs/DECISION.md` | Mentions of Rune responsiveness and the open Rune questions | 5 |
| `docs/decisions/0003-console-protocol-adapter.md` | Mark "Rune behavior and remaining gaps" as superseded by 0006; the right-button workaround is gone | 3 |
| `docs/FEATURES.md` | "Real Rune window" in M0; re-state the supported list only if G3 evidence differs | 3, 5 |
| `docs/SOURCES.md` | Replace the Rune section with the reviewed Kit and GPUI versions and source links | 0, each upgrade |
| `docs/VERIFICATION.md` | "Rune headless screenshots never substitute" names GPUI; new commands | 2, 5 |
| `docs/DEVELOPMENT.md` | Inspection and scene tooling for the GPUI build | 2 |
| `docs/RUNNING.md`, `docs/INSTALLATION.md` | Linux dependencies and run-time requirements from S0 and S5 | 5 |
| `docs/PARITY-IMPLEMENTATION.md` | Mark rows whose live evidence depended on the Rune interface | 4 |
| `CHANGELOG.md` | Cutover release entry | 5 |
| Repository description on GitHub | Remove "with Rune UI" | 5 |

### Ledger format

`docs/GPUI-MIGRATION-LEDGER.md` has five tables. Agents update their own rows in the same pull request as the code.

| Table | Columns |
| --- | --- |
| Commands | `CommandId`, label, menu path, object kinds, Rune source (`file::function`), model status, GPUI status, package |
| Scenes | scene name, Rune source, surface, status (not ported, ported, retired), package |
| Tests | Rune test path and name, kind (pure, runtime), disposition (moved, ported, retired with reason), new location |
| Mirror fixes | Rune commit, summary, mirrored in (commit) or not applicable |
| Deviations | package, what differs from this plan, reason |

## Decisions and open questions

Agents proceed on the defaults below. Nothing here blocks Phase 0 or Phase 1. An item marked "operator" needs a human answer by the gate named; until then the default stands.

| # | Question | Default | Decided by | Needed by |
| --- | --- | --- | --- | --- |
| D1 | Route for console key codes | Route A (second Wayland keyboard listener), then route B (patched GPUI) | S1 evidence | G0 |
| D2 | Accept keysym-only input (route C) if A and B fail | No. Stop the migration | Operator | G0 |
| D3 | Minimum window size and narrow layout | Minimum 800x600. The navigation pane collapses to a toggle below 1100 px. The 513x652 scenes are re-specified at 800x600 | Operator | G2 |
| D4 | Tabs beyond the reference: HA and Users on Pool, tabs for Network | As in the tab matrix | Operator may veto | G2 |
| D5 | Local cursor over the console when hiding is unavailable | Ship a visible local cursor with the drawn guest cursor; record the deviation | Agent | G3 |
| D6 | Offer the GPUI build to users before cutover | After G3, ship it in the archive as a preview binary beside the Rune binary | Operator | G3 |
| D7 | X11 | Stays experimental. The console uses keysyms there | Already decided in `docs/FEATURES.md` | G3 |
| D8 | Theme | Kit default tokens, the current accent colour, light and dark | Agent | G2 |
| D9 | Crate and binary names after cutover | Crate `xenadmin-desktop`, binary `xenadmin-rs` | Agent | G5 |
| D10 | Version of the cutover release | The next minor version after the last Rune release | Operator | G5 |
| D11 | Screen reader support statement | Remains "not supported" until a screen reader run is recorded | Already decided in `docs/FEATURES.md` | G5 |
| D12 | Kit upgrades during the migration | Stay on 0.9.1 unless a needed fix lands upstream | Agent, per gate | Each gate |
| D13 | Experimental rows that are costly to port | Port all rows. Deferring a row is an operator decision recorded in the ledger | Operator | G4 |

### Assumptions not yet verified

These were inferred from source reading or from module names. Check each one at the step named before building on it.

- GPUI macros compile under this workspace's lints (S0).
- A second `wl_keyboard` bound through the foreign display receives the same key events as GPUI's own, on each target compositor (S1).
- `cx.intercept_keystrokes`, or a key context on the console element, is enough to keep every key away from application bindings (S1).
- `window.paint_image` with a fresh `RenderImage` per frame, plus `window.drop_image`, holds memory steady at console frame rates (S1).
- `AppMenuBar` reflects `disabled` and `checked` from `MenuItem::Action` after `cx.set_menus` is called again (S4).
- `LineChart` can show gaps for missing samples; otherwise use a custom canvas (P3.5).
- `window.set_rem_size` scales Kit components as today's interface scale does (S4).
- The activation token path used by `instance_launch` has a GPUI equivalent (S5).
- The module-to-surface tables in "Target information architecture" were built from module and scene names, not from reading every module. Confirm each when its package starts and record a deviation if the content does not fit.
- The menu list follows the reference `MainWindow.resx`; items that this client does not implement are left out when the command table is built, not stubbed.
