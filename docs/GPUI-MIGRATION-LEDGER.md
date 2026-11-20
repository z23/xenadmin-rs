# GPUI migration ledger

The record of the move of the desktop front end from Rune to GPUI Kit 0.9.1, as the plan asks for it
([GPUI-MIGRATION.md](GPUI-MIGRATION.md), "Ledger format"): what every command, scene and test of the Rune
front end became, which Rune defects the port corrected, and where the port differs from the plan or from
the Rune client. It replaces the ledger of the earlier phases, which described a preview shell that no
longer exists.

State after the cutover of the code (2026-11-19): all 29 areas of the Rune front end have a port in
`crates/xenadmin-ui/src/areas/<id>/`; the shell and the one generic renderer are in
`crates/xenadmin-desktop`, which was `crates/xenadmin-gpui` until the cutover; the Rune crate, which had
the path `crates/xenadmin-desktop` until then, is deleted, and the client binary `xenadmin-rs` is built
from the GPUI Kit front end. The ledger was first written while the Rune crate existed (commit `89d700c`
is the last that has it) and had its final pass after the cutover: the window halves of the tests, the
changes of the last two rounds of work and the new paths. In the columns that describe the Rune client
("Rune file", "Rune test", "Rune behaviour", "what it was") the file names are those of the deleted crate.

## What has been verified, and what has not

`AGENTS.md` asks that implemented, fixture-verified, live-host-verified and planned behaviour be reported
separately. For the GPUI front end, every status in this ledger means the following and nothing more.

| level | what it covers here |
| --- | --- |
| implemented | The 238 commands, the surfaces behind them and the 129 Rune scenes exist in the new crates |
| fixture-verified | Model tests of `xenadmin-ui` (979 test functions; four are ignored: three that print a table and the upgrade test, which needs a state directory of the released client) and headless tests of `xenadmin-desktop` (260 test functions, 30 of them ignored: the 29 live tests and utilities of `tests/lab_live.rs` and one measurement) on synthetic inventories; a few tests connect the real backend to the local fixture server (`xenadmin_ui::testing`). Every registered scene renders in the headless scene tool. "Headless" means a window that is never shown: no compositor, no pointer or keyboard device, no clipboard, no portal dialog, no screen reader |
| live-host-verified, headless | The supported areas, on the project's lab on 2026-11-19, by the ignored tests of `tests/lab_live.rs`, which run the real shell and core without a window. Their successful paths: connecting with pinned certificates and following the inventory; power and events; a power batch; a guest's console with keys through the canvas and a host's text console; screenshot export; snapshots; performance; host maintenance with a real evacuation; live migration both ways; disconnect, connect and a second start; a client closed in the middle of an operation ([report](reports/2026-11-19-gpui-live-headless.md); first run at commit `e2d4572`, all repeated, passing, after the cutover at `56fedbd`). Their refusals, interruptions and recoveries, by fourteen further scenarios the same day: reboot, suspend, resume, the force actions and Start on Server; a batch that stops on a failure; the refused reviews; migration with disks, its cancellation and a halted VM's disk move; a snapshot with memory; the console's separate window, Type Clipboard as Keys and Ctrl+Alt+Del; a member's toolstack restart, reboot and shutdown through maintenance; recovery after a restart of the coordinator's toolstack, a lost member and a certificate that is not the pinned one ([report](reports/2026-11-19-gpui-live-paths.md), which has every scenario's evidence directory, its findings and what was left out). Not run through the GPUI front end: a coordinator change and maintenance of the coordinator, the host's own refusal of an evacuation, the keyring. The experimental areas: nothing live; their live evidence under `docs/reports/` was recorded through the core's own scenarios or through the Rune client |
| verified in a virtual machine with injected input | On 2026-11-19, the AppImage CI built at commit `e808a2f`, as a Wayland client of sway 1.12 on the display of a QEMU machine, drawn by a software renderer, with every key and pointer event injected at the machine's devices by a script ([report](reports/2026-11-19-gpui-vm-session.md)): connection to the lab with pinned certificates and a live update; the recovery gate of `README.md`; tables and search; a guest's console through the Wayland key path for console key codes (decision 0006, route A), with the guest recording the keys; a host's console; the clipboard with the window focused; the portal file chooser; a console in a separate window; fullscreen; server-side decorations; the client on the accessibility bus, with Orca's output for the demo's views read from its log. Two defects found there (a second start that showed nothing, a held key that did not repeat in a console) are fixed in the tree and were confirmed in the same machine with builds of the fixes |
| verified by a person in a real window | Nothing. No person has used the GPUI client in a Wayland or X11 session, no hardware GPU has drawn it, and it has not run on Hyprland, GNOME or KDE; on X11 its window was opened once, without a console (the clean-install run). No person has used a screen reader with it. The only run on a real seat is the spike of gate G0, whose open points are the accepted limits of [decision 0006](decisions/0006-console-key-codes.md), listed under Deviations |
| packaged | CI built the archive and the AppImage at commit `e808a2f`. Both were installed and started on untouched Ubuntu 24.04 and Arch images in virtual machines, with software rendering and no person: checksums, the installer, the window under a headless Sway (the wlroots headless backend: it offers a seat, and no input was given there) and, on Ubuntu, in an X session with scripted input, the probe offline, uninstall ([report](reports/2026-11-19-gpui-clean-install.md)); the AppImage ran the driven session of the row above, the recovery gate included. The fixes made on 2026-11-19 are in no package, the corrected package lists were not installed alone on an untouched image, and there is no release artifact |
| planned | A person's run of a package at a desk, with a hardware GPU and the recovery gate; CI packages of the merged tree; the live paths still missing; the measurement of `large-inventory` on an idle machine; the release |

The counts above are counts of test functions in the tree (`scripts/gpui_ledger_tables.py check` prints
them; the functions two macros of the window tests write are counted). The suites were run as a whole
by `scripts/check.sh` at the cutover (`581ba08`), during this ledger's final pass and, on the tree with
the branches of 2026-11-19 merged, in a run started at commit `1c84ac5`, which passed; the gate reports of
2026-11-19 (`docs/reports/2026-11-19-gpui-gate-g2.md` to `-g5.md`) say which run shows what.

## How this ledger is generated and checked

Places are abbreviated: `ui:x` is `crates/xenadmin-ui/src/x`, `gpui:x` is `crates/xenadmin-desktop/x`
(the GPUI front end; the prefix is kept from the time the crate was `crates/xenadmin-gpui`).

```sh
# the registry, as the scene tool prints it (scripts/check.sh writes the same file)
cargo run -p xenadmin-desktop --features scenes --locked --quiet --bin xenadmin-scenes -- --commands > artifacts/ui/commands.txt
# the whole ledger against the repository
python3 scripts/gpui_ledger_tables.py check --commands artifacts/ui/commands.txt
# and against the deleted Rune crate, from the last commit that has it
git archive 89d700c crates/xenadmin-desktop | tar -x -C "$DIR"
XENADMIN_RUNE_SOURCES="$DIR/crates/xenadmin-desktop/src" python3 scripts/gpui_ledger_tables.py check --commands artifacts/ui/commands.txt
```

| table | generated by | needs the Rune crate |
| --- | --- | --- |
| Commands | `scripts/gpui_commands_table.py`, every column | no |
| Scenes | `scripts/gpui_ledger_tables.py scenes`, every column | no (the 129 Rune names are kept in `ui:scene.rs`) |
| Tests | `scripts/gpui_ledger_tables.py tests` gives the Rune file, the test, its kind and the successors that carry its name; the disposition, the other successors and the note are written by hand | yes for the first three columns; `check` verifies the successors without it |
| Mirror fixes, Deviations | by hand, from the engineers' results and the headers of the areas | no |
| Appendix | `scripts/gpui_ledger_tables.py keys` gives the Rune file and the key; the rest is by hand | yes |

The Rune columns of the Tests table and the whole key list of the appendix were generated on 2026-11-19
while the Rune crate still existed. Now that it is deleted they are a record: `check` still verifies the
Commands table, the Scenes table, that every successor named in the Tests table is a test function of the
file it names, that a Rune test whose name begins a row of `gpui:tests/wanted.rs` has that row's area test
as a successor, the counts, and that every table is well formed. With `XENADMIN_RUNE_SOURCES` it compares
the Rune columns and the appendix again; the final pass was checked that way too.

### Tests by disposition

| disposition | tests | meaning |
| --- | --- | --- |
| moved | 154 | a test that built no `rune::Runtime`, now a test of `xenadmin-ui` (or of the GPUI crate where its subject went) |
| ported | 119 | a test that drove a `rune::Runtime`, rewritten against the model (the `Harness` presses the keys of the description) or as a headless GPUI test |
| split | 94 | a model test and a headless window test in `crates/xenadmin-desktop/tests` |
| retired | 4 | no successor is wanted; the reason is in the row |
| total | 371 |  |

Of the 371 Rune tests, 156 built no `rune::Runtime` (kind `pure`) and 215 drove one (kind
`runtime`). 171 of the runtime tests ran in a compact window (513x652, or 480 wide) alone or beside
1440x900. A model test has no window, so that a control stays inside the window at a size is asserted only
by the split rows and by the renderer's own tests. The window halves are in
`crates/xenadmin-desktop/tests`: `wanted.rs` is a table of 77 rows (a scene, its sizes, what must be
inside the window and what must be reachable by scrolling), run by one test per area, and
`wanted_events.rs` holds the tests of what a press, a key or a file prompt does. The Rune compact size,
513x652, cannot occur in the main window any more: a window half checks 800x600, the smallest size, and
1440x900. 5 ported rows still carry the note "model half": by its name the Rune test promised that
controls stay reachable or visible at its window sizes, and no window test names it. The note was
assigned from the name of the Rune test, not by reading each body. 14 split rows have a window half that
was matched to them by area, scene and subject (their note says so) where the row of `wanted.rs` does
not carry the Rune test's name. The assertions of a successor were not compared with those of the Rune
test it succeeds. No row is open.

## Commands

238 commands in 29 registrations (`kernel` and 28 areas; the area `general` registers none).
The registry is the only source of the ten menus, the toolbar, the context menus and the local action rows.
"object kinds" is the context menu the command is offered in, "local row" the tab whose action row shows
it. A command is implemented and, where its area has tests, fixture-verified. The headless lab runs of
2026-11-19 invoked those of the supported areas that their [report](reports/2026-11-19-gpui-live-headless.md)
names (Add Server and Connect, Shut Down and Start singly and as a batch, Take a Snapshot, Revert and
Delete, Enter and Exit Maintenance Mode, Migrate to Server, Disconnect, Save Screenshot, Release Input,
Options), and the second run of that day those its [report](reports/2026-11-19-gpui-live-paths.md) names
(among them Reboot, Suspend, Resume, the force actions, Start on Server, Move VM with its disks, the
cancellation of a migration, Take a Snapshot with memory, Open in Separate Window, Fullscreen, Type
Clipboard as Keys, Send Ctrl+Alt+Del, Restart Toolstack, and Reboot and Shut Down of a server); no other
command has been invoked against a host. In a window, commands were invoked only in the driven session
in a virtual machine ([report](reports/2026-11-19-gpui-vm-session.md)), never by a person.

```sh
cargo run -p xenadmin-desktop --features scenes --locked --quiet --bin xenadmin-scenes -- --commands > artifacts/ui/commands.txt
python3 scripts/gpui_commands_table.py artifacts/ui/commands.txt
```

| CommandId | label | menu path | object kinds | toolbar | local row | shortcut | area |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `nav.back` | Back | - | - | Navigation.10 | - | alt-left | `kernel` |
| `nav.forward` | Forward | - | - | Navigation.20 | - | alt-right | `kernel` |
| `view.custom-templates` | Custom Templates | View [1.10] | - | - | - | - | `kernel` |
| `view.builtin-templates` | Built-in Templates | View [1.20] | - | - | - | - | `kernel` |
| `view.local-storage` | Local Storage | View [1.30] | - | - | - | - | `kernel` |
| `view.toolbar` | Toolbar | View [2.10] | - | - | - | - | `kernel` |
| `view.zoom-in` | Larger Interface | View [3.10] | - | - | - | ctrl-= | `kernel` |
| `view.zoom-out` | Smaller Interface | View [3.20] | - | - | - | ctrl-- | `kernel` |
| `view.zoom-reset` | Reset Interface Size | View [3.30] | - | - | - | ctrl-0 | `kernel` |
| `object.copy-uuid` | Copy UUID | - | Pool, Server, VM, Template, Storage repository, Network, Virtual disk, Snapshot, vApp | - | - | - | `kernel` |
| `file.exit` | Exit | File [9.10] | - | - | - | ctrl-q | `kernel` |
| `pool.properties` | Properties | Pool [9.10] | Pool | - | General.200 | - | `kernel` |
| `server.properties` | Properties | Server [9.10] | Server | - | General.200 | - | `kernel` |
| `vm.properties` | Properties | VM [9.10] | VM | - | General.200 | - | `kernel` |
| `storage.properties` | Properties | Storage [9.10] | Storage repository | - | General.200 | - | `kernel` |
| `template.properties` | Properties | Templates [9.10] | Template | - | General.200 | - | `kernel` |
| `network.properties` | Properties | - | Network | - | General.200 | - | `kernel` |
| `disk.properties` | Properties | - | Virtual disk | - | General.200 | - | `kernel` |
| `vm.start` | Start | VM > Start/Shut Down [2.10] | VM | Power.10 | - | - | `power` |
| `vm.shutdown` | Shut Down | VM > Start/Shut Down [2.20] | VM | Power.20 | - | - | `power` |
| `vm.reboot` | Reboot | VM > Start/Shut Down [2.30] | VM | Power.30 | - | - | `power` |
| `vm.suspend` | Suspend | VM > Start/Shut Down [2.40] | VM | Power.50 | - | - | `power` |
| `vm.resume` | Resume | VM > Start/Shut Down [2.50] | VM | Power.40 | - | - | `power` |
| `vm.pause` | Pause | VM > Start/Shut Down [2.60] | VM | - | - | - | `power` |
| `vm.unpause` | Unpause | VM > Start/Shut Down [2.70] | VM | - | - | - | `power` |
| `vm.force-shutdown` | Force Shut Down | VM > Start/Shut Down [2.80] | VM | Power.60 | - | - | `power` |
| `vm.force-reboot` | Force Reboot | VM > Start/Shut Down [2.90] | VM | Power.70 | - | - | `power` |
| `vm.recovery-start` | Start in Recovery Mode | VM > Start/Shut Down [2.100] | VM | - | - | - | `power` |
| `vm.recovery-restore-boot` | Restore Saved Boot Order | VM > Start/Shut Down [2.105] | VM | - | - | - | `power` |
| `vm.start-on-server` | Start on Server | VM [2.120] | VM | - | - | - | `power` |
| `vm.resume-on-server` | Resume on Server | VM [2.130] | VM | - | - | - | `power` |
| `vm.power-selection` | Power actions for selection… | - | VM | - | - | - | `power` |
| `vm.check-availability` | Check action availability | - | VM | - | General.10 | - | `power` |
| `vm.snapshot` | Take a Snapshot | VM [4.10] | VM | - | Snapshots.10 | - | `snapshots` |
| `snapshot.revert` | Revert | - | - | - | Snapshots.20 | - | `snapshots` |
| `snapshot.properties` | Edit properties | - | - | - | Snapshots.30 | - | `snapshots` |
| `snapshot.new-vm` | Create VM | - | - | - | Snapshots.40 | - | `snapshots` |
| `snapshot.new-template` | Create template | - | - | - | Snapshots.50 | - | `snapshots` |
| `snapshot.delete` | Delete | - | - | - | Snapshots.90 | - | `snapshots` |
| `vm.memory-settings` | Memory Settings | - | - | - | Memory.10 | - | `vm_properties` |
| `vm.new` | New VM | VM [1.10] | - | New.40 | - | - | `vm_lifecycle` |
| `vm.copy` | Copy VM | VM [3.30] | VM | - | - | - | `vm_lifecycle` |
| `vm.convert-to-template` | Convert to Template | VM [5.10] | VM | - | - | - | `vm_lifecycle` |
| `vm.cloud-init-drive` | Cloud-init Drive | VM [6.30] | VM | - | Storage.60 | - | `vm_lifecycle` |
| `vm.delete` | Delete VM | VM [7.10] | VM | - | - | - | `vm_lifecycle` |
| `templates.new` | New VM wizard | Templates [1.10] | - | - | - | - | `vm_lifecycle` |
| `templates.create-vm` | Create VM From Selection | Templates [1.20] | Template | - | - | - | `vm_lifecycle` |
| `templates.copy` | Copy | Templates [2.20] | Template | - | - | - | `vm_lifecycle` |
| `templates.delete` | Delete Template | Templates [3.10] | Template | - | - | - | `vm_lifecycle` |
| `vm.migrate` | Migrate to Server | VM [3.10] | VM | - | - | - | `migration` |
| `vm.move` | Move VM | VM [3.20] | VM | - | - | - | `migration` |
| `vm.move-to-pool` | Move to Another Pool | VM [3.25] | VM | - | - | - | `cross_pool` |
| `file.import` | Import | File [1.10] | - | - | - | - | `transfers` |
| `vm.export` | Export | VM [5.20] | VM | - | - | - | `transfers` |
| `templates.export` | Export to File | Templates [2.10] | Template | - | - | - | `transfers` |
| `disk.export-image` | Export Disk Image | - | Virtual disk | - | Storage.88 | - | `transfers` |
| `tools.convert-disk-image` | Convert Disk Image | Tools [2.20] | - | - | - | - | `transfers` |
| `network.new` | Add Network | - | - | - | Networking.10 | - | `network` |
| `network.vlan.add` | Add VLAN Interface | - | Network | - | Networking.20 | - | `network` |
| `network.remove` | Remove Network | - | Network | - | Networking.30 | - | `network` |
| `nic.ipv4` | Configure IPv4 | - | - | - | Networking.40 | - | `network` |
| `nic.ipv6` | Configure IPv6 | - | - | - | Networking.50 | - | `network` |
| `nic.purpose` | Interface Purpose | - | - | - | Networking.60 | - | `network` |
| `nic.plug` | Plug Interface | - | - | - | - | - | `network` |
| `nic.unplug-protection` | Protect from Unplugging | - | - | - | - | - | `network` |
| `nic.vlan.delete` | Delete VLAN Interface | - | - | - | - | - | `network` |
| `server.bond.mode` | Change Bond Mode | - | - | - | - | - | `network` |
| `server.bond.hash` | Change Bond Hash | - | - | - | - | - | `network` |
| `server.bond.create` | Create Bond | - | - | - | NICs.10 | - | `network` |
| `server.bond.delete` | Delete Bond | - | - | - | NICs.20 | - | `network` |
| `server.nics.rescan` | Rescan | - | - | - | NICs.30 | - | `network` |
| `server.scheduling` | Disable New VM Placement | Server [5.12] | Server | - | - | - | `network` |
| `storage.set-default` | Set as Default | Storage [2.20] | Storage repository | - | Storage.30 | - | `network` |
| `storage.rescan` | Rescan | Storage [2.30] | Storage repository | - | Storage.35 | - | `network` |
| `network.vlan.create-pool` | Create Pool-wide VLANs | - | Network | - | Networking.110 | - | `network_services` |
| `network.vlan.retag-pool` | Retag Pool-wide VLANs | - | Network | - | Networking.120 | - | `network_services` |
| `network.service.nbd-tls` | Enable NBD with TLS | - | Network | - | Networking.130 | - | `network_services` |
| `network.service.nbd-plaintext` | Enable NBD without TLS | - | Network | - | Networking.140 | - | `network_services` |
| `network.fabric.add` | Add Tunnel or SR-IOV Interface | - | Network | - | Networking.150 | - | `network_services` |
| `network.fabric.complete-pool` | Complete Pool Coverage | - | Network | - | Networking.160 | - | `network_services` |
| `network.fabric.remove` | Remove Tunnel or SR-IOV Interface | - | Network | - | Networking.170 | - | `network_services` |
| `network.fabric.remove-pool` | Remove Pool Coverage | - | Network | - | Networking.180 | - | `network_services` |
| `server.management.configure` | Configure Management IP | - | Server | - | Networking.210 | - | `management_network` |
| `server.management.move` | Move Management Interface | - | Server | - | Networking.220 | - | `management_network` |
| `server.management.bond.create` | Create Management Bond | - | Server | - | NICs.210 | - | `management_network` |
| `server.management.bond.remove` | Remove Management Bond | - | Server | - | NICs.220 | - | `management_network` |
| `server.management.bond.ipv6` | Staged IPv6 Management Bond | - | Server | - | NICs.230 | - | `management_network` |
| `server.management.bond.ipv6.continue` | Review Next IPv6 Handover Stage | - | Server, Pool | - | NICs.240, Networking.240 | - | `management_network` |
| `server.management.bond.ipv6.leave` | Leave Partial IPv6 Handover | - | Server, Pool | - | NICs.250, Networking.250 | - | `management_network` |
| `server.join-pool` | Add to Pool | Pool [1.10] | Pool, Server | - | - | - | `pool` |
| `pool.remove-server` | Remove Server | Pool [1.20] | Pool | - | - | - | `pool` |
| `pool.ha.configure` | High Availability | Pool [2.20] | Pool | - | HA.10 | - | `pool` |
| `pool.ha.disable` | Disable HA | - | - | - | HA.20 | - | `pool` |
| `pool.ha.failures-to-tolerate` | Failure Tolerance | - | - | - | HA.30 | - | `pool` |
| `pool.ha.overcommit` | HA Overcommit | - | - | - | HA.40 | - | `pool` |
| `pool.ha.timeout` | HA Timeout Default | - | - | - | HA.50 | - | `pool` |
| `vm.ha.restart-priority` | Restart Priority | - | - | - | HA.60 | - | `pool` |
| `pool.change-server-password` | Change Server Password | Pool [3.10] | Pool | - | - | - | `pool` |
| `pool.make-standalone` | Make into Standalone Server | Pool [3.20] | Pool | - | - | - | `pool` |
| `server.change-password` | Change Password | Server > Password [5.30] | Server | - | - | - | `pool` |
| `pool.designate-coordinator` | Hand Over Coordinator | Server [6.5] | Server | - | - | - | `pool` |
| `server.remove-from-pool` | Remove from Pool | Server [6.10] | Server | - | - | - | `pool` |
| `pool.security.refresh` | Refresh security status | - | - | - | Users.10 | - | `pool_security` |
| `pool.directory.join` | Join Active Directory… | - | - | - | Users.20 | - | `pool_security` |
| `pool.directory.leave` | Leave Active Directory… | - | - | - | Users.30 | - | `pool_security` |
| `pool.subject.add` | Add Directory Subject… | - | - | - | Users.40 | - | `pool_security` |
| `pool.subject.role` | Change Subject Roles… | - | - | - | Users.50 | - | `pool_security` |
| `pool.subject.remove` | Remove Directory Subject… | - | - | - | Users.60 | - | `pool_security` |
| `server.certificate.install` | Install TLS Certificate… | Server [5.50] | - | - | - | - | `pool_security` |
| `server.certificate.reset` | Reset TLS Certificate… | Server [5.60] | - | - | - | - | `pool_security` |
| `storage.new` | New Storage | Storage [1.10] | Pool, Server | New.30 | Storage.10 | - | `storage` |
| `storage.trim` | Reclaim Freed Space | Storage [2.40] | Storage repository | - | Storage.40 | - | `storage` |
| `storage.detach` | Detach | Storage [4.10] | Storage repository | - | Storage.50 | - | `storage` |
| `storage.reattach` | Reattach | Storage [4.20] | Storage repository | - | Storage.60 | - | `storage` |
| `storage.attachment.create` | Create Missing Host Attachment | Storage [4.22] | Storage repository | - | Storage.62 | - | `storage` |
| `storage.attachment.remove` | Remove Detached Attachment | Storage [4.24] | Storage repository | - | Storage.64 | - | `storage` |
| `storage.forget` | Forget | Storage [4.30] | Storage repository | - | Storage.70 | - | `storage` |
| `storage.destroy` | Destroy | Storage [4.40] | Storage repository | - | Storage.80 | - | `storage` |
| `vm.disk.new` | New Virtual Disk | Storage > Virtual Disks [3.10] | - | - | Storage.10 | - | `vm_devices` |
| `vm.disk.attach` | Attach Virtual Disk | Storage > Virtual Disks [3.20] | - | - | Storage.12 | - | `vm_devices` |
| `vm.cd.add` | Add CD/DVD Drive | - | - | - | Storage.14 | - | `vm_devices` |
| `vm.disk.detach` | Detach | - | - | - | Storage.20 | - | `vm_devices` |
| `vm.disk.activate` | Activate | - | - | - | Storage.22 | - | `vm_devices` |
| `vm.disk.deactivate` | Deactivate | - | - | - | Storage.24 | - | `vm_devices` |
| `vm.cd.insert` | Insert ISO | - | - | - | Storage.26 | - | `vm_devices` |
| `vm.cd.eject` | Eject Media | - | - | - | Storage.28 | - | `vm_devices` |
| `vm.disk.properties` | Properties | - | - | - | Storage.30 | - | `vm_devices` |
| `disk.grow` | Resize | - | Virtual disk | - | Storage.82 | - | `vm_devices` |
| `disk.copy` | Copy | - | Virtual disk | - | Storage.84 | - | `vm_devices` |
| `disk.cleanup` | Delete Owned Disk | - | Virtual disk | - | Storage.86 | - | `vm_devices` |
| `vm.disk.delete` | Delete | - | - | - | Storage.38 | - | `vm_devices` |
| `vm.nic.add` | Add | - | - | - | Networking.10 | - | `vm_devices` |
| `vm.nic.properties` | Properties | - | - | - | Networking.20 | - | `vm_devices` |
| `vm.nic.rate-limit` | Rate Limit | - | - | - | Networking.30 | - | `vm_devices` |
| `vm.nic.activate` | Activate | - | - | - | Networking.40 | - | `vm_devices` |
| `vm.nic.deactivate` | Deactivate | - | - | - | Networking.50 | - | `vm_devices` |
| `vm.nic.remove` | Remove | - | - | - | Networking.90 | - | `vm_devices` |
| `org.tag.add` | Add tag to selection | - | Pool, Server, VM, Template, Storage repository, Network, Virtual disk, Snapshot | - | - | - | `organization` |
| `org.tag.remove` | Remove tag from selection | - | Pool, Server, VM, Template, Storage repository, Network, Virtual disk, Snapshot | - | - | - | `organization` |
| `org.folder.move` | Move selection to folder | - | Pool, Server, VM, Template, Storage repository, Network, Virtual disk, Snapshot | - | - | - | `organization` |
| `org.field.set` | Set custom field on selection | - | Pool, Server, VM, Template, Storage repository, Network, Virtual disk, Snapshot | - | - | - | `organization` |
| `org.field.clear` | Clear custom field on selection | - | Pool, Server, VM, Template, Storage repository, Network, Virtual disk, Snapshot | - | - | - | `organization` |
| `org.tag.rename` | Rename tag across pool | - | Tag | - | - | - | `organization` |
| `org.tag.delete` | Delete tag across pool | - | Tag | - | - | - | `organization` |
| `org.folder.create` | Create folder | - | Folder, Pool | - | - | - | `organization` |
| `org.folder.rename` | Rename folder across pool | - | Folder | - | - | - | `organization` |
| `org.folder.delete` | Delete folder across pool | - | Folder | - | - | - | `organization` |
| `pool.vapps` | vApps | Pool [2.10] | Pool, vApp | - | - | - | `organization` |
| `vm.assign-to-vapp` | Assign to vApp | VM [4.30] | VM | - | - | - | `organization` |
| `vapp.start` | Start vApp | - | vApp | - | - | - | `organization` |
| `vapp.shutdown` | Shut down vApp | - | vApp | - | - | - | `organization` |
| `vapp.force-shutdown` | Force shutdown | - | vApp | - | - | - | `organization` |
| `server.reboot` | Reboot | Server [2.10] | Server | - | - | - | `host_admin` |
| `server.power-on` | Power On | Server [2.15] | Server | - | - | - | `host_admin` |
| `server.shutdown` | Shut Down | Server [2.20] | Server | - | - | - | `host_admin` |
| `server.restart-toolstack` | Restart Toolstack | Server [2.30] | Server | - | - | - | `host_admin` |
| `server.maintenance` | Enter Maintenance Mode | Server [5.10] | Server | - | - | - | `host_admin` |
| `server.evacuate` | Evacuate Guests | Server [5.15] | Server | - | - | - | `host_admin` |
| `server.control-domain-memory` | Control Domain Memory | Server [5.20] | Server | - | Memory.10 | - | `host_admin` |
| `server.destroy` | Destroy | Server [6.20] | Server | - | - | - | `host_admin` |
| `pool.vm-autostart` | VM Autostart | Pool [2.40] | Pool | - | - | - | `host_admin` |
| `performance.data-sources` | Data Sources… | - | - | - | Performance.10 | - | `performance` |
| `performance.presets` | Graph Presets… | - | - | - | Performance.20 | - | `performance` |
| `performance.reconnect` | Reconnect Metrics | - | - | - | Performance.30 | - | `performance` |
| `server.monitoring-recording` | Monitoring Recording… | Server [5.50] | - | - | Performance.40 | - | `performance` |
| `storage.repair` | Repair | Storage [2.10] | Storage repository | - | Storage.20 | - | `storage_maintenance` |
| `storage.multipath-workflow` | Multipathing with Storage Reconnection | - | - | - | - | - | `storage_maintenance` |
| `server.backup` | Back Up | Server [4.10] | - | - | - | - | `host_services` |
| `server.restore-from-backup` | Restore From Backup | Server [4.20] | - | - | - | - | `host_services` |
| `tools.server-status-report` | Server Status Report | Tools [3.10] | - | - | - | - | `host_services` |
| `server.power-on-settings` | BMC / Power-On Settings | - | - | - | - | - | `host_services` |
| `pool.snapshot-schedules` | VM Snapshot Schedule Policies | Pool [2.30] | Pool | - | - | - | `schedules_alarms` |
| `vm.assign-snapshot-schedule` | Assign to Snapshot Schedule | VM [4.20] | VM | - | - | - | `schedules_alarms` |
| `tools.alarm-service-refresh` | Refresh Alarm Service | Tools [3.30] | Pool | - | - | - | `schedules_alarms` |
| `server.add` | Add Server… | Server [1.10] | Nothing | New.10 | Home.10 | - | `connections` |
| `server.connect` | Connect… | Server [3.10] | Pool, Connection | - | Home.20 | - | `connections` |
| `server.cancel-connect` | Cancel connection attempt | - | Pool, Connection | - | Home.30 | - | `connections` |
| `server.disconnect` | Disconnect | Server [3.20] | Pool, Server, Connection | - | Home.40 | - | `connections` |
| `pool.disconnect` | Disconnect | Pool [1.40] | - | - | - | - | `connections` |
| `server.reconnect-as` | Reconnect As… | Server [3.30] | Pool, Server, Connection | - | Home.50 | - | `connections` |
| `pool.reconnect-as` | Reconnect As… | Pool [1.30] | - | - | - | - | `connections` |
| `server.review-certificate` | Review this endpoint's certificate… | - | Pool, Connection | - | Home.60 | - | `connections` |
| `server.forget-password` | Forget Password | Server > Password [5.40] | Pool, Connection | - | - | - | `connections` |
| `server.remove-connection` | Remove connection | - | Pool, Connection | - | Home.70 | - | `connections` |
| `server.connect-all` | Connect All | Server [3.40] | - | - | Home.80 | - | `connections` |
| `server.disconnect-all` | Disconnect All | Server [3.50] | - | - | Home.90 | - | `connections` |
| `search.open-result` | Open in Pool | - | Pool, Server, VM, Template, Snapshot, Storage repository, Network, Virtual disk | - | Search.10 | - | `search` |
| `search.save` | Save Search | - | - | - | Search.20 | - | `search` |
| `search.export` | Export Saved Searches | - | - | - | Search.30 | - | `search` |
| `search.remove` | Delete Saved Search | - | Saved search | - | Search.40 | - | `search` |
| `file.import-search` | Import Search | File [1.20] | - | - | - | - | `search` |
| `tools.inventory-report` | Inventory Report | Tools [3.20] | - | - | - | - | `search` |
| `vm.export-selection` | Export | VM [5.20] | VM | - | - | - | `bulk_export` |
| `templates.export-selection` | Export to File | Templates [2.10] | Template | - | - | - | `bulk_export` |
| `snapshot.export` | Export selection… | - | - | - | Snapshots.60 | - | `bulk_export` |
| `vm.delete-several` | Delete VM | VM [7.10] | VM | - | - | - | `bulk_edit` |
| `templates.delete-several` | Delete Template | Templates [3.10] | Template | - | - | - | `bulk_edit` |
| `tools.options` | Options | Tools [1.10] | - | - | - | - | `settings` |
| `help.help` | Help | Help [1.10] | - | - | - | - | `settings` |
| `help.website` | Project website | Help [2.10] | - | - | - | - | `settings` |
| `help.about` | About | Help [9.10] | - | - | - | - | `settings` |
| `vm.console` | Open Console | - | VM | - | Console.10 | - | `console` |
| `server.console` | Open Host Text Console | - | Server | - | Console.11 | - | `console` |
| `console.reconnect` | Reconnect | - | - | - | Console.20 | - | `console` |
| `console.actual-size` | Actual Size | - | - | - | Console.30 | - | `console` |
| `console.fullscreen` | Fullscreen | - | - | - | Console.40 | - | `console` |
| `console.undock` | Open in Separate Window | - | - | - | Console.50 | - | `console` |
| `console.release-input` | Release Input | - | - | - | Console.60 | - | `console` |
| `console.save-screenshot` | Save Screenshot… | - | - | - | Console.70 | - | `console` |
| `vm.send-ctrl-alt-del` | Send Ctrl+Alt+Del | VM [6.20] | - | - | Console.80 | - | `console` |
| `console.send-ctrl-alt-f1` | Ctrl+Alt+F1 | - | - | - | Console.90 | - | `console` |
| `console.send-alt-tab` | Alt+Tab | - | - | - | Console.100 | - | `console` |
| `console.send-ctrl-c` | Ctrl+C | - | - | - | Console.110 | - | `console` |
| `console.send-ctrl-d` | Ctrl+D | - | - | - | Console.120 | - | `console` |
| `console.send-clipboard` | Send Clipboard to VM | - | - | - | Console.130 | - | `console` |
| `console.type-clipboard` | Type Clipboard as Keys… | - | - | - | Console.140 | - | `console` |
| `console.copy-remote-clipboard` | Copy VM Clipboard | - | - | - | Console.150 | - | `console` |
| `console.close` | Close Console | - | - | - | Console.200 | - | `console` |
| `window.console-1` | Console Window 1 | Window [1.1] | - | - | - | - | `console` |
| `window.console-2` | Console Window 2 | Window [1.2] | - | - | - | - | `console` |
| `window.console-3` | Console Window 3 | Window [1.3] | - | - | - | - | `console` |
| `window.console-4` | Console Window 4 | Window [1.4] | - | - | - | - | `console` |
| `window.console-5` | Console Window 5 | Window [1.5] | - | - | - | - | `console` |
| `window.console-6` | Console Window 6 | Window [1.6] | - | - | - | - | `console` |
| `window.console-7` | Console Window 7 | Window [1.7] | - | - | - | - | `console` |
| `window.console-8` | Console Window 8 | Window [1.8] | - | - | - | - | `console` |
| `window.console.close-all` | Close All Consoles | Window [2.10] | - | - | - | - | `console` |
| `vm.install-guest-tools` | Install Guest Tools | VM [6.10] | VM | - | Storage.90 | - | `vm_tools` |
| `vm.connect-external` | Connect with SSH, RDP or Local Tool… | - | VM | - | Console.200 | - | `vm_tools` |
| `tools.external-tools` | External Tools | Tools [2.10] | - | - | - | - | `vm_tools` |
| `events.cancel-task` | Review cancellation… | - | - | - | Events.10 | - | `events` |
| `events.explain` | Explain outcome… | - | - | - | Events.20 | - | `events` |
| `events.hide` | Hide completed entry | - | - | - | Events.30 | - | `events` |
| `events.recheck` | Recheck outcome | - | - | - | Events.40 | - | `events` |
| `events.acknowledge` | Acknowledge unknown outcome… | - | - | - | Events.50 | - | `events` |
| `events.request-cancel` | Request cancellation | - | - | - | Events.60 | - | `events` |
| `events.export` | Export filtered entries… | - | - | - | Events.90, Alerts.90 | - | `events` |
| `alerts.dismiss` | Review dismissal… | - | - | - | Alerts.10 | - | `events` |

## Scenes

The 129 scene names of the Rune client (`ui:scene.rs`), each with the area that registers it now.
"renders headlessly" is `gpui:tests/renderer.rs::every_scene_renders_with_the_keys_of_its_description`:
the scene tool draws the scene in a window that is not shown and every key its description requires is an
element of the frame. It is not a comparison with the Rune picture and not a run in a real window. The
scene tool also knows scenes that Rune did not have (the renderer's `gallery-*` scenes and the kernel's
`host-properties`); they are not rows here.

```sh
python3 scripts/gpui_ledger_tables.py scenes
```

| scene | area | registered in | status |
| --- | --- | --- | --- |
| `demo` | `kernel` | `ui:demo.rs` | registered, renders headlessly |
| `large-inventory` | `kernel` | `ui:demo.rs` | registered, renders headlessly |
| `host-version-notice` | `kernel` | `ui:demo.rs` | registered, renders headlessly |
| `interface-large` | `kernel` | `ui:demo.rs` | registered, renders headlessly |
| `inventory-context` | `search` | `ui:areas/search/mod.rs` | registered, renders headlessly |
| `general-information` | `general` | `ui:areas/general/mod.rs` | registered, renders headlessly |
| `options` | `settings` | `ui:areas/settings/mod.rs` | registered, renders headlessly |
| `help` | `settings` | `ui:areas/settings/mod.rs` | registered, renders headlessly |
| `console-details` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `batch-power` | `power` | `ui:areas/power/scenes.rs` | registered, renders headlessly |
| `batch-properties` | `bulk_edit` | `ui:areas/bulk_edit/scenes.rs` | registered, renders headlessly |
| `batch-delete` | `bulk_edit` | `ui:areas/bulk_edit/scenes.rs` | registered, renders headlessly |
| `lifecycle-permissions` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `batch-move` | `migration` | `ui:areas/migration/scenes.rs` | registered, renders headlessly |
| `batch-cross-pool` | `cross_pool` | `ui:areas/cross_pool/scenes.rs` | registered, renders headlessly |
| `batch-export` | `bulk_export` | `ui:areas/bulk_export/scenes.rs` | registered, renders headlessly |
| `snapshot-batch-export` | `bulk_export` | `ui:areas/bulk_export/scenes.rs` | registered, renders headlessly |
| `console-recovery` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `console-frame` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `console-actual-size` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `console-typing-review` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `multiple-consoles` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `guest-connections` | `vm_tools` | `ui:areas/vm_tools/scenes.rs` | registered, renders headlessly |
| `external-tools` | `vm_tools` | `ui:areas/vm_tools/scenes.rs` | registered, renders headlessly |
| `host-console` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |
| `search-pools` | `search` | `ui:areas/search/mod.rs` | registered, renders headlessly |
| `performance` | `performance` | `ui:areas/performance/scenes.rs` | registered, renders headlessly |
| `metric-presets` | `performance` | `ui:areas/performance/scenes.rs` | registered, renders headlessly |
| `unavailable-actions` | `kernel` | `ui:demo.rs` | registered, renders headlessly |
| `vm-devices` | `vm_devices` | `ui:areas/vm_devices/scenes.rs` | registered, renders headlessly |
| `properties` | `vm_properties` | `ui:areas/vm_properties/scenes.rs` | registered, renders headlessly |
| `properties-running` | `vm_properties` | `ui:areas/vm_properties/scenes.rs` | registered, renders headlessly |
| `vm-create` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `device-properties` | `vm_devices` | `ui:areas/vm_devices/scenes.rs` | registered, renders headlessly |
| `home-host` | `vm_properties` | `ui:areas/vm_properties/scenes.rs` | registered, renders headlessly |
| `firmware-create` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `creation-cloud` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `creation-cloud-pending` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `secure-boot` | `vm_properties` | `ui:areas/vm_properties/scenes.rs` | registered, renders headlessly |
| `vm-migration` | `migration` | `ui:areas/migration/scenes.rs` | registered, renders headlessly |
| `vm-recovery` | `power` | `ui:areas/power/scenes.rs` | registered, renders headlessly |
| `vm-recovery-force` | `power` | `ui:areas/power/scenes.rs` | registered, renders headlessly |
| `cross-pool` | `cross_pool` | `ui:areas/cross_pool/scenes.rs` | registered, renders headlessly |
| `archive-export` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `ha-policy` | `pool` | `ui:areas/pool/scenes.rs` | registered, renders headlessly |
| `failure-help` | `events` | `ui:areas/events/scenes.rs` | registered, renders headlessly |
| `checkpoint-export` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `xva-compression` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `cloud-init` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `cloud-init-review` | `vm_lifecycle` | `ui:areas/vm_lifecycle/scenes.rs` | registered, renders headlessly |
| `ovf-import` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `ovf-export` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `disk-export` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `snapshots` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `snapshot-create` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `snapshot-ancestry` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `snapshot-information` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `snapshot-restore` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `snapshot-edit` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `snapshot-capture` | `snapshots` | `ui:areas/snapshots/scenes.rs` | registered, renders headlessly |
| `connections` | `connections` | `ui:areas/connections/scenes.rs` | registered, renders headlessly |
| `connection-trust` | `connections` | `ui:areas/connections/scenes.rs` | registered, renders headlessly |
| `certificate-review` | `connections` | `ui:areas/connections/scenes.rs` | registered, renders headlessly |
| `network-interfaces` | `network` | `ui:areas/network/scenes.rs` | registered, renders headlessly |
| `network-services` | `network_services` | `ui:areas/network_services/scenes.rs` | registered, renders headlessly |
| `network-fabric` | `network_services` | `ui:areas/network_services/scenes.rs` | registered, renders headlessly |
| `network-fabric-pool` | `network_services` | `ui:areas/network_services/scenes.rs` | registered, renders headlessly |
| `network-fabric-pool-select` | `network_services` | `ui:areas/network_services/scenes.rs` | registered, renders headlessly |
| `network-sriov` | `network_services` | `ui:areas/network_services/scenes.rs` | registered, renders headlessly |
| `network-ipv6` | `network` | `ui:areas/network/scenes.rs` | registered, renders headlessly |
| `network-topology` | `network_services` | `ui:areas/network_services/scenes.rs` | registered, renders headlessly |
| `management-network` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `management-dhcp` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `management-vlan` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `management-bond` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `management-ipv6-bond` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `management-bond-guests` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `management-vlan-bond` | `management_network` | `ui:areas/management_network/scenes.rs` | registered, renders headlessly |
| `host-administration` | `host_admin` | `ui:areas/host_admin/scenes.rs` | registered, renders headlessly |
| `host-memory` | `host_admin` | `ui:areas/host_admin/scenes.rs` | registered, renders headlessly |
| `host-monitoring` | `performance` | `ui:areas/performance/scenes.rs` | registered, renders headlessly |
| `local-conversion` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `local-conversion-review` | `transfers` | `ui:areas/transfers/scenes.rs` | registered, renders headlessly |
| `host-monitoring-catalog` | `performance` | `ui:areas/performance/scenes.rs` | registered, renders headlessly |
| `host-support` | `host_services` | `ui:areas/host_services/scenes.rs` | registered, renders headlessly |
| `host-restore` | `host_services` | `ui:areas/host_services/scenes.rs` | registered, renders headlessly |
| `host-crashdump-delete` | `host_services` | `ui:areas/host_services/scenes.rs` | registered, renders headlessly |
| `host-power-settings` | `host_services` | `ui:areas/host_services/scenes.rs` | registered, renders headlessly |
| `snapshot-schedules` | `schedules_alarms` | `ui:areas/schedules_alarms/scenes.rs` | registered, renders headlessly |
| `snapshot-schedule-review` | `schedules_alarms` | `ui:areas/schedules_alarms/scenes.rs` | registered, renders headlessly |
| `alarms` | `schedules_alarms` | `ui:areas/schedules_alarms/scenes.rs` | registered, renders headlessly |
| `alarms-review` | `schedules_alarms` | `ui:areas/schedules_alarms/scenes.rs` | registered, renders headlessly |
| `alarms-email` | `schedules_alarms` | `ui:areas/schedules_alarms/scenes.rs` | registered, renders headlessly |
| `network-create` | `network` | `ui:areas/network/scenes.rs` | registered, renders headlessly |
| `activity` | `events` | `ui:areas/events/scenes.rs` | registered, renders headlessly |
| `organization` | `organization` | `ui:areas/organization/scenes.rs` | registered, renders headlessly |
| `organization-review` | `organization` | `ui:areas/organization/scenes.rs` | registered, renders headlessly |
| `appliances` | `organization` | `ui:areas/organization/scenes.rs` | registered, renders headlessly |
| `appliance-review` | `organization` | `ui:areas/organization/scenes.rs` | registered, renders headlessly |
| `disk-cleanup` | `vm_devices` | `ui:areas/vm_devices/scenes.rs` | registered, renders headlessly |
| `disk-grow` | `vm_devices` | `ui:areas/vm_devices/scenes.rs` | registered, renders headlessly |
| `disk-copy` | `vm_devices` | `ui:areas/vm_devices/scenes.rs` | registered, renders headlessly |
| `storage-create` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-review` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-local` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-local-review` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-hba` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-hba-review` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-repair` | `storage_maintenance` | `ui:areas/storage_maintenance/scenes.rs` | registered, renders headlessly |
| `storage-graph` | `storage_maintenance` | `ui:areas/storage_maintenance/scenes.rs` | registered, renders headlessly |
| `storage-multipath` | `storage_maintenance` | `ui:areas/storage_maintenance/scenes.rs` | registered, renders headlessly |
| `storage-maintenance` | `storage_maintenance` | `ui:areas/storage_maintenance/scenes.rs` | registered, renders headlessly |
| `storage-cache` | `storage_maintenance` | `ui:areas/storage_maintenance/scenes.rs` | registered, renders headlessly |
| `storage-discovery` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `storage-auth-review` | `storage` | `ui:areas/storage/scenes.rs` | registered, renders headlessly |
| `pool-security` | `pool_security` | `ui:areas/pool_security/scenes.rs` | registered, renders headlessly |
| `pool-join` | `pool` | `ui:areas/pool/scenes.rs` | registered, renders headlessly |
| `root-password` | `pool` | `ui:areas/pool/scenes.rs` | registered, renders headlessly |
| `pool-dissolve` | `pool` | `ui:areas/pool/scenes.rs` | registered, renders headlessly |
| `guest-tools` | `vm_tools` | `ui:areas/vm_tools/scenes.rs` | registered, renders headlessly |
| `guest-tools-review` | `vm_tools` | `ui:areas/vm_tools/scenes.rs` | registered, renders headlessly |
| `pool-security-review` | `pool_security` | `ui:areas/pool_security/scenes.rs` | registered, renders headlessly |
| `pool-policies` | `pool` | `ui:areas/pool/scenes.rs` | registered, renders headlessly |
| `pool-policy-review` | `pool` | `ui:areas/pool/scenes.rs` | registered, renders headlessly |
| `pool-administration` | `network` | `ui:areas/network/scenes.rs` | registered, renders headlessly |
| `connection-settings` | `connections` | `ui:areas/connections/scenes.rs` | registered, renders headlessly |
| `connect` | `connections` | `ui:areas/connections/scenes.rs` | registered, renders headlessly |
| `connect-insecure` | `connections` | `ui:areas/connections/scenes.rs` | registered, renders headlessly |
| `console-window` | `console` | `ui:areas/console/scenes.rs` | registered, renders headlessly |

## Tests

Every `#[test]` of the Rune crate (`crates/xenadmin-desktop/src` at commit `89d700c`; 371), in the order of the files. The successor is
`place::function`. A successor found by the Rune test's own name (or `rune_` and its name) is listed by
the script; the others are named in a comment at the new test or in the header of its file.

- moved: the test built no `rune::Runtime`; it moved with the logic it tests.
- ported: the test drove a `rune::Runtime`; it is rewritten against the model, where the `Harness` presses
  the keys of the described surface, or as a headless GPUI test.
- split: a model test and a headless window test in `crates/xenadmin-desktop/tests`.
- retired: no successor is wanted. The reason is in the note.
- open: half a successor. It is a defect of the port's test coverage, not a decision. No row has it now.

The note "model half" is explained under "Tests by disposition". A successor in
`gpui:tests/wanted.rs` is the test of the row's area (`gpui:tests/wanted.rs::storage` runs every row of
the area `storage`); the row of the table is the one whose description begins with the Rune test's name.
`check` prints "to review" for a row whose Rune test a file of `crates/xenadmin-desktop/tests` names
while the row has no successor there.

```sh
python3 scripts/gpui_ledger_tables.py tests    # the first three columns and the successors by name
```

| Rune file | Rune test | kind | disposition | successor | note |
| --- | --- | --- | --- | --- | --- |
| `activity_tests.rs` | `activity_review_captures_exact_target_and_never_dispatches_from_synthetic_or_stale_views` | runtime | split | `ui:areas/events/tests.rs::activity_review_captures_exact_target_and_never_dispatches_from_synthetic_or_stale_views`; `gpui:tests/areas.rs::the_task_cancellation_review_opens_from_the_local_row_and_submits_nothing_in_the_demo` |  |
| `activity_tests.rs` | `message_review_keeps_original_content_digest_when_selection_or_inventory_changes` | pure | moved | `ui:areas/events/tests.rs::message_review_keeps_original_content_digest_when_selection_or_inventory_changes` |  |
| `activity_tests.rs` | `clear_buttons_empty_the_activity_filters_and_restore_the_rows` | runtime | split | `ui:areas/events/tests.rs::emptying_a_filter_leaves_the_other_and_restores_the_rows`; `gpui:tests/wanted_events.rs::clear_buttons_empty_the_activity_filters_and_restore_the_rows` | the clear button is the Kit input's own now; the model test types the empty text and the window test clicks the button of each filter field |
| `activity_tests.rs` | `filters_cover_message_body_priority_task_state_and_cached_pools` | pure | moved | `ui:areas/events/tests.rs::filters_cover_message_body_priority_task_state_and_cached_pools` |  |
| `activity_tests.rs` | `history_hiding_is_terminal_only_pool_scoped_restorable_and_survives_disconnect` | pure | moved | `ui:areas/events/tests.rs::history_hiding_is_terminal_only_pool_scoped_restorable_and_survives_disconnect` |  |
| `activity_tests.rs` | `public_export_projection_excludes_task_refs_private_plans_and_resource_baselines` | pure | moved | `ui:areas/events/tests.rs::public_export_projection_excludes_task_refs_private_plans_and_resource_baselines` |  |
| `activity_tests.rs` | `save_dialog_exports_captured_selection_even_after_switch_and_cancel_writes_nothing` | runtime | ported | `ui:areas/events/tests.rs::save_dialog_exports_captured_selection_even_after_switch_and_cancel_writes_nothing` |  |
| `activity_tests.rs` | `outcome_help_opens_from_tasks_and_both_history_families_without_mutating_or_retargeting` | runtime | ported | `ui:areas/events/tests.rs::outcome_help_opens_from_tasks_and_both_history_families_without_mutating_or_retargeting` |  |
| `activity_tests.rs` | `successful_history_and_message_text_never_offer_error_code_guidance` | pure | moved | `ui:areas/events/tests.rs::successful_history_and_message_text_never_offer_error_code_guidance` |  |
| `alarms_view.rs` | `alarm_review_converts_percent_and_retains_original_pool_at_both_widths` | runtime | split | `ui:areas/schedules_alarms/tests.rs::alarm_review_converts_percent_and_retains_original_pool`; `gpui:tests/wanted.rs::schedules_alarms` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `alarms_view.rs` | `invalid_threshold_and_custom_source_refuse_review_but_explicit_disable_is_captured` | pure | moved | `ui:areas/schedules_alarms/tests.rs::invalid_threshold_and_custom_source_refuse_review_but_explicit_disable_is_captured` |  |
| `alarms_view.rs` | `alarm_email_and_refresh_reviews_explain_configuration_and_delivery_separately` | pure | moved | `ui:areas/schedules_alarms/tests.rs::alarm_email_and_refresh_reviews_explain_configuration_and_delivery_separately` |  |
| `appliance_view.rs` | `power_review_captures_members_order_delays_and_requires_force_confirmation` | pure | moved | `ui:areas/organization/tests.rs::power_review_captures_members_order_delays_and_requires_force_confirmation` |  |
| `appliance_view.rs` | `membership_review_preserves_exact_group_and_refuses_stealing_other_group_vm` | pure | moved | `ui:areas/organization/tests.rs::membership_review_preserves_exact_group_and_refuses_stealing_other_group_vm` |  |
| `appliance_view.rs` | `compact_review_keeps_controls_visible_and_refuses_preview_submission` | runtime | split | `ui:areas/organization/tests.rs::compact_review_keeps_controls_visible_and_refuses_preview_submission`; `gpui:tests/wanted.rs::organization`; `gpui:tests/wanted_events.rs::compact_review_keeps_controls_visible_and_refuses_preview_submission` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_cross_pool_tests.rs` | `the_inventory_selection_offers_the_move_to_another_pool_beside_the_same_pool_move` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_the_selection_offers_the_move_to_another_pool_beside_the_same_pool_move` |  |
| `bulk_cross_pool_tests.rs` | `the_review_lists_every_vm_with_its_mapping_both_pools_and_a_reachable_footer` | runtime | split | `ui:areas/cross_pool/tests.rs::rune_the_review_lists_every_vm_with_its_mapping_and_both_pools`; `gpui:tests/wanted.rs::cross_pool` | the window half is the row of `gpui:tests/wanted.rs` for the scene `batch-cross-pool` ("the batch cross-pool review"), matched to this test by its area, scene and subject, not by name |
| `bulk_cross_pool_tests.rs` | `each_stage_asks_for_its_choices_and_the_acknowledgement_is_given_once_per_review` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_each_stage_asks_for_its_choices_and_the_acknowledgement_is_given_once_per_review` |  |
| `bulk_cross_pool_tests.rs` | `a_vm_or_destination_that_does_not_fit_refuses_the_whole_selection_and_omits_none` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_a_vm_or_destination_that_does_not_fit_refuses_the_whole_selection_and_omits_none` |  |
| `bulk_cross_pool_tests.rs` | `a_review_that_no_longer_matches_the_pools_or_lacks_a_current_session_is_not_submitted` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_a_review_that_no_longer_matches_the_pools_or_lacks_a_current_session_is_not_submitted` |  |
| `bulk_cross_pool_tests.rs` | `history_shows_a_batch_with_each_vm_and_offers_recheck_stop_and_acknowledgement_on_the_batch_only` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_a_batch_is_rechecked_stopped_and_acknowledged_through_the_batch_only` | the list is the Events tab's; asserted here: the rules it asks this area for |
| `bulk_delete_tests.rs` | `compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer` | runtime | split | `ui:areas/bulk_edit/tests.rs::compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer`; `gpui:tests/wanted.rs::bulk_edit`; `gpui:tests/wanted_events.rs::compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_delete_tests.rs` | `changed_or_missing_target_refuses_the_entire_review` | runtime | ported | `ui:areas/bulk_edit/tests.rs::changed_or_missing_target_refuses_the_entire_review` |  |
| `bulk_delete_tests.rs` | `selected_disk_data_is_explicit_and_new_ownership_cannot_be_silently_omitted` | runtime | ported | `ui:areas/bulk_edit/tests.rs::selected_disk_data_is_explicit_and_new_ownership_cannot_be_silently_omitted` |  |
| `bulk_delete_tests.rs` | `template_and_read_only_reviews_do_not_drop_ineligible_rows` | runtime | ported | `ui:areas/bulk_edit/tests.rs::template_and_read_only_reviews_do_not_drop_ineligible_rows` |  |
| `bulk_export_snapshots_tests.rs` | `several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one` | runtime | split | `ui:areas/bulk_export/tests.rs::several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one`; `gpui:tests/wanted_events.rs::several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one` |  |
| `bulk_export_snapshots_tests.rs` | `review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer` | runtime | split | `ui:areas/bulk_export/tests.rs::review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer`; `gpui:tests/wanted.rs::bulk_export` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_export_snapshots_tests.rs` | `including_memory_is_chosen_for_each_checkpoint_by_itself_and_ends_a_review` | runtime | ported | `ui:areas/bulk_export/tests.rs::including_memory_is_chosen_for_each_checkpoint_by_itself_and_ends_a_review` |  |
| `bulk_export_snapshots_tests.rs` | `a_checkpoint_in_the_selection_leaves_only_the_xva_formats` | runtime | ported | `ui:areas/bulk_export/tests.rs::a_checkpoint_in_the_selection_leaves_only_the_xva_formats` |  |
| `bulk_export_snapshots_tests.rs` | `a_changed_snapshot_memory_image_or_destination_blocks_the_whole_review` | runtime | ported | `ui:areas/bulk_export/tests.rs::a_changed_snapshot_memory_image_or_destination_blocks_the_whole_review` |  |
| `bulk_export_snapshots_tests.rs` | `history_names_each_snapshot_of_a_batch_and_stops_after_the_current_snapshot` | runtime | ported | `ui:areas/bulk_export/tests.rs::history_names_each_snapshot_of_a_batch_and_stops_after_the_current_snapshot` |  |
| `bulk_export_tests.rs` | `compact_and_wide_export_review_keeps_exact_targets_and_safe_footer` | runtime | split | `ui:areas/bulk_export/tests.rs::compact_and_wide_export_review_keeps_exact_targets_and_safe_footer`; `gpui:tests/wanted.rs::bulk_export` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_export_tests.rs` | `native_output_selection_is_bound_to_editor_identity_and_format` | runtime | split | `ui:areas/bulk_export/tests.rs::native_output_selection_is_bound_to_editor_identity_and_format`; `gpui:tests/wanted_events.rs::native_output_selection_is_bound_to_editor_identity_and_format` |  |
| `bulk_export_tests.rs` | `overlapping_destinations_and_changed_sources_block_entire_review` | runtime | ported | `ui:areas/bulk_export/tests.rs::overlapping_destinations_and_changed_sources_block_entire_review` |  |
| `bulk_export_tests.rs` | `batch_history_keeps_parent_reachable_and_children_have_no_independent_controls` | runtime | ported | `ui:areas/bulk_export/tests.rs::batch_history_keeps_parent_reachable_and_children_have_no_independent_controls` |  |
| `bulk_move_tests.rs` | `compact_and_wide_review_capture_all_vms_and_keep_safe_footer` | runtime | split | `ui:areas/migration/tests.rs::rune_batch_review_captures_all_vms_and_keeps_safe_footer`; `gpui:tests/wanted.rs::migration`; `gpui:tests/wanted_events.rs::compact_and_wide_review_capture_all_vms_and_keep_safe_footer` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_move_tests.rs` | `mapping_edits_are_per_target_and_review_exact_prior_and_new_values` | runtime | ported | `ui:areas/migration/tests.rs::rune_batch_mapping_edits_are_per_target_and_review_exact_prior_and_new_values` |  |
| `bulk_move_tests.rs` | `changed_or_unsupported_targets_refuse_whole_selection_without_omission` | pure | moved | `ui:areas/migration/tests.rs::rune_batch_changed_or_unsupported_targets_refuse_whole_selection_without_omission` |  |
| `bulk_power_tests.rs` | `multi_selection_survives_inventory_refresh_and_reviews_captured_vms_at_both_sizes` | runtime | split | `ui:areas/power/tests.rs::rune_batch_reviews_the_captured_vms_in_order_and_demo_cannot_run_it`; `gpui:tests/wanted_events.rs::multi_selection_survives_inventory_refresh_and_reviews_captured_vms_at_both_sizes` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_power_tests.rs` | `context_menu_targets_clicked_row_and_keeps_non_vm_menus_relevant` | runtime | split | `ui:areas/power/tests.rs::rune_context_menu_of_several_vms_has_the_batch_and_other_kinds_do_not`; `gpui:tests/wanted_events.rs::context_menu_targets_clicked_row_and_keeps_non_vm_menus_relevant` | that the menu opens on the clicked row is the grid's: `gpui:tests/shell.rs::a_grid_row_that_stands_for_an_object_has_its_context_menu_at_the_pointer` |
| `bulk_power_tests.rs` | `batch_uses_only_visible_selection_and_rejects_over_limit_without_silent_subset` | runtime | split | `ui:areas/power/tests.rs::rune_batch_rejects_over_limit_without_silent_subset`; `gpui:tests/wanted_events.rs::batch_uses_only_visible_selection_and_rejects_over_limit_without_silent_subset` | the limit; which rows are visible and selected is the grid's now |
| `bulk_properties_tests.rs` | `property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes` | runtime | split | `ui:areas/bulk_edit/tests.rs::property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes`; `gpui:tests/wanted.rs::bulk_edit`; `gpui:tests/wanted_events.rs::property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `bulk_properties_tests.rs` | `property_batch_names_every_unchanged_target_and_refuses_all_noop` | runtime | ported | `ui:areas/bulk_edit/tests.rs::property_batch_names_every_unchanged_target_and_refuses_all_noop` |  |
| `bulk_properties_tests.rs` | `property_batch_missing_unsupported_and_changed_targets_block_whole_review` | runtime | ported | `ui:areas/bulk_edit/tests.rs::property_batch_missing_unsupported_and_changed_targets_block_whole_review` |  |
| `bulk_properties_tests.rs` | `property_batch_all_existing_property_kinds_bind_per_target_previous_values` | pure | moved | `ui:areas/bulk_edit/tests.rs::property_batch_all_existing_property_kinds_bind_per_target_previous_values` |  |
| `bulk_properties_tests.rs` | `property_batch_live_limits_capabilities_and_stale_review_refuse_submission` | pure | moved | `ui:areas/bulk_edit/tests.rs::property_batch_live_limits_capabilities_and_stale_review_refuse_submission` |  |
| `bulk_properties_tests.rs` | `property_batch_picker_and_template_entry_work_with_keyboard_menu_capture` | runtime | split | `ui:areas/bulk_edit/tests.rs::property_batch_picker_and_template_entry_work_with_keyboard_menu_capture`; `gpui:tests/wanted_events.rs::property_batch_picker_and_template_entry_work_with_keyboard_menu_capture` |  |
| `checkpoint_transfer_tests.rs` | `checkpoint_export_explicitly_reviews_memory_and_captured_identity_at_both_sizes` | runtime | split | `ui:areas/transfers/tests.rs::checkpoint_export_explicitly_reviews_memory_and_captured_identity_at_both_sizes`; `gpui:tests/wanted.rs::transfers` | the window half is the row of `gpui:tests/wanted.rs` for the scene `checkpoint-export` ("checkpoint export"), matched to this test by its area, scene and subject, not by name |
| `checkpoint_transfer_tests.rs` | `checkpoint_import_review_uses_separate_action_and_retains_suspended_state` | runtime | ported | `ui:areas/transfers/tests.rs::checkpoint_import_review_uses_separate_action_and_retains_suspended_state` |  |
| `cloud_init_view_tests.rs` | `cloud_init_review_keeps_target_and_resources_at_compact_and_wide_sizes` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::cloud_init_review_keeps_target_and_resources` | model half; by its name the Rune test promised reachability at its window sizes, and no window test names it |
| `cloud_init_view_tests.rs` | `cloud_init_bad_inputs_preserve_editor_and_pool_changes_disable_review` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::cloud_init_bad_inputs_preserve_editor_and_pool_changes_disable_review` |  |
| `cloud_init_view_tests.rs` | `cloud_init_cancelling_preparation_drops_late_media_without_reopening` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::cloud_init_cancelling_preparation_drops_late_media_without_reopening` |  |
| `cloud_init_view_tests.rs` | `cloud_init_slot_validation_refuses_occupied_and_noncanonical_slots` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::cloud_init_slot_validation_refuses_occupied_and_noncanonical_slots` |  |
| `cloud_init_view_tests.rs` | `creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr` | runtime | split | `ui:areas/vm_lifecycle/tests.rs::creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr`; `gpui:tests/wanted_events.rs::creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr` |  |
| `compression_transfer_tests.rs` | `compression_choice_invalidates_destination_and_review_keeps_exact_source_and_format` | runtime | ported | `ui:areas/transfers/tests.rs::compression_choice_invalidates_destination_and_review_keeps_exact_source_and_format` |  |
| `compression_transfer_tests.rs` | `changing_compression_discards_pending_picker_result_and_package_changes_clear_it` | runtime | ported | `ui:areas/transfers/tests.rs::changing_compression_discards_pending_picker_result_and_package_changes_clear_it` |  |
| `console_export.rs` | `chooser_captures_one_frame_and_cancel_or_console_switch_does_not_write` | runtime | split | `ui:areas/console/tests.rs::chooser_captures_one_frame_and_cancel_or_console_switch_does_not_write`; `gpui:tests/console.rs::save_screenshot_asks_for_a_path_and_writes_a_png` |  |
| `console_export.rs` | `screenshot_round_trip_is_private_and_invalid_frames_do_not_replace_files` | pure | moved | `ui:areas/console/export.rs::screenshot_round_trip_is_private_and_invalid_frames_do_not_replace_files` |  |
| `console_recovery_tests.rs` | `external_move_reboot_and_generation_change_close_then_reopen_once` | pure | moved | `ui:areas/console/recovery.rs::external_move_reboot_and_generation_change_close_then_reopen_once` |  |
| `console_recovery_tests.rs` | `retry_budget_is_bounded_and_brief_connections_do_not_reset_it` | pure | moved | `ui:areas/console/recovery.rs::retry_budget_is_bounded_and_brief_connections_do_not_reset_it` |  |
| `console_recovery_tests.rs` | `stale_missing_halted_and_read_only_targets_cannot_reopen_or_reset_failures` | pure | moved | `ui:areas/console/recovery.rs::stale_missing_halted_and_read_only_targets_cannot_reopen_or_reset_failures` |  |
| `console_recovery_tests.rs` | `optional_boot_metadata_never_causes_a_spurious_reopen_and_opt_out_is_honored` | pure | moved | `ui:areas/console/recovery.rs::optional_boot_metadata_never_causes_a_spurious_reopen_and_opt_out_is_honored` |  |
| `console_recovery_tests.rs` | `synchronous_admission_failure_consumes_attempts_and_releases_old_input` | pure | moved | `ui:areas/console/recovery.rs::synchronous_admission_failure_consumes_attempts_and_releases_old_input` |  |
| `console_recovery_tests.rs` | `inventory_observation_uses_pinned_pool_and_vm_and_rejects_missing_or_wrong_kind` | pure | moved | `ui:areas/console/recovery.rs::inventory_observation_uses_pinned_pool_and_vm_and_rejects_missing_or_wrong_kind` |  |
| `console_recovery_tests.rs` | `automatic_reconnect_toggle_and_pinned_identity_render_at_both_window_sizes` | runtime | ported | `ui:areas/console/tests.rs::automatic_reconnect_toggle_and_pinned_identity_render_at_both_window_sizes` |  |
| `console_view.rs` | `console_defaults_apply_only_when_opening_a_new_target` | pure | moved | `ui:areas/console/tests.rs::console_defaults_apply_only_when_opening_a_new_target` |  |
| `console_view.rs` | `clipboard_text_is_typed_only_after_a_review_that_never_shows_it` | runtime | ported | `ui:areas/console/tests.rs::clipboard_text_is_typed_only_after_a_review_that_never_shows_it` |  |
| `console_view.rs` | `a_repeat_after_release_does_not_press_the_key_again` | pure | moved | `ui:areas/console/canvas.rs::a_repeat_after_release_does_not_press_the_key_again` |  |
| `console_view.rs` | `the_console_canvas_is_laid_out_in_fit_and_actual_size_modes` | runtime | split | `ui:areas/console/tests.rs::the_console_canvas_is_laid_out_in_fit_and_actual_size_modes`; `gpui:tests/console.rs::the_canvas_is_laid_out_in_fit_and_actual_size` |  |
| `console_view.rs` | `pointer_matches_letterboxing_actual_size_and_resize` | pure | moved | `ui:areas/console/canvas.rs::pointer_matches_letterboxing_actual_size_and_resize` |  |
| `console_view.rs` | `wheel_notches_and_precise_motion_reach_guest_without_losing_small_notches` | pure | moved | `ui:areas/console/canvas.rs::wheel_notches_and_precise_motion_reach_guest_without_losing_small_notches` |  |
| `console_view.rs` | `extended_scancodes_follow_qemu_make_code_encoding` | pure | moved | `ui:areas/console/canvas.rs::extended_scancodes_follow_qemu_make_code_encoding` |  |
| `console_windows.rs` | `independent_console_windows_keep_targets_frames_and_input_state_separate` | runtime | split | `ui:areas/console/tests.rs::independent_console_windows_keep_targets_frames_and_input_state_separate`; `gpui:tests/console.rs::two_console_windows_keep_pictures_and_input_apart` |  |
| `console_windows.rs` | `reusable_slot_rejects_old_adoption_close_and_focus_requests` | pure | moved | `ui:areas/console/tests.rs::reusable_slot_rejects_old_adoption_close_and_focus_requests` |  |
| `console_windows.rs` | `window_capacity_is_bounded_and_closing_returns_the_slot` | pure | moved | `ui:areas/console/tests.rs::window_capacity_is_bounded_and_closing_returns_the_slot` |  |
| `console_windows.rs` | `manager_is_reachable_with_eight_targets_at_compact_and_wide_sizes` | runtime | ported | `ui:areas/console/tests.rs::manager_is_reachable_with_eight_targets_at_compact_and_wide_sizes` | model half; by its name the Rune test promised reachability at its window sizes, and no window test names it |
| `console_windows.rs` | `a_console_window_is_drawn_at_the_interface_size_and_follows_changes_to_it` | runtime | ported | `gpui:tests/console.rs::the_pointer_stays_canvas_local_at_a_larger_interface_size` | the pointer half. Handing the options from window to window is retired: every window reads the one model's options |
| `console_windows.rs` | `moving_console_invalidates_old_file_dialog_without_disabling_new_window_export` | runtime | ported | `ui:areas/console/tests.rs::moving_console_invalidates_old_file_dialog_without_disabling_new_window_export` |  |
| `cross_pool_vm_tests.rs` | `reviewed_mapping_and_host_transport_warning_render_at_both_widths_without_demo_dispatch` | runtime | split | `ui:areas/cross_pool/tests.rs::rune_reviewed_mapping_and_host_transport_warning_without_demo_dispatch`; `gpui:tests/wanted.rs::cross_pool` | the window half is the row of `gpui:tests/wanted.rs` for the scene `cross-pool` ("the cross-pool review"), matched to this test by its area, scene and subject, not by name |
| `cross_pool_vm_tests.rs` | `choosing_a_pool_requires_explicit_device_mappings_and_new_transport_review` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_choosing_a_pool_requires_explicit_device_mappings_and_new_transport_review` |  |
| `cross_pool_vm_tests.rs` | `current_source_membership_and_destination_attachments_gate_review` | pure | moved | `ui:areas/cross_pool/tests.rs::rune_current_source_membership_and_destination_attachments_gate_review` |  |
| `cross_pool_vm_tests.rs` | `halted_copy_review_preserves_source_and_uses_distinct_identity_semantics` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_halted_copy_review_preserves_source_and_uses_distinct_identity_semantics` |  |
| `cross_pool_vm_tests.rs` | `offline_history_keeps_both_pool_namespaces_and_partial_resources_after_source_disappears` | runtime | ported | `ui:areas/cross_pool/tests.rs::rune_offline_history_keeps_both_pool_namespaces_and_recheck_needs_both_pools` | the Rune history list is the History list of the Events tab; asserted here: what that list is given |
| `cross_pool_vm_tests.rs` | `destination_console_requires_proven_result_and_exact_current_pool_vm_host_and_power_state` | pure | moved | `ui:areas/cross_pool/tests.rs::rune_destination_console_requires_proven_result_and_exact_current_pool_vm_host_and_power_state` |  |
| `disk_workspace_tests.rs` | `compact_and_wide_copy_review_capture_target_and_block_demo_dispatch` | runtime | split | `ui:areas/vm_devices/tests.rs::copy_review_captures_target_and_blocks_demo_dispatch`; `gpui:tests/wanted.rs::vm_devices` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `disk_workspace_tests.rs` | `growth_rejects_shrink_and_copy_requires_name_and_chosen_storage` | pure | moved | `ui:areas/vm_devices/tests.rs::growth_rejects_shrink_and_copy_requires_name_and_chosen_storage` |  |
| `disk_workspace_tests.rs` | `cleanup_requires_local_ownership_and_current_consumer_graph` | pure | moved | `ui:areas/vm_devices/tests.rs::cleanup_requires_local_ownership_and_current_consumer_graph` |  |
| `disk_workspace_tests.rs` | `cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation_at_both_sizes` | runtime | split | `ui:areas/vm_devices/tests.rs::cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation`; `gpui:tests/wanted.rs::vm_devices`; `gpui:tests/wanted_events.rs::cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation_at_both_sizes` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `external_tool_command.rs` | `local_launch_preserves_argv_without_shell_or_secret_environment` | pure | moved | `ui:areas/vm_tools/command.rs::local_launch_preserves_argv_without_shell_or_secret_environment` |  |
| `external_tool_command.rs` | `command_requires_reviewed_executable_and_bounded_typed_inputs` | pure | moved | `ui:areas/vm_tools/command.rs::command_requires_reviewed_executable_and_bounded_typed_inputs` |  |
| `external_tool_command.rs` | `rdp_uri_escapes_username_and_brackets_ipv6_without_password` | pure | moved | `ui:areas/vm_tools/command.rs::rdp_uri_escapes_username_and_brackets_ipv6_without_password` |  |
| `external_tool_command.rs` | `local_declarations_are_versioned_bounded_and_do_not_accept_hidden_fields` | pure | moved | `ui:areas/vm_tools/command.rs::local_declarations_are_versioned_bounded_and_do_not_accept_hidden_fields` |  |
| `external_tools_tests.rs` | `reviewed_guest_command_keeps_target_and_rejects_changed_input_at_both_sizes` | runtime | split | `ui:areas/vm_tools/tests.rs::rune_reviewed_guest_command_keeps_target_and_rejects_changed_input`; `gpui:tests/wanted.rs::vm_tools` | the window half is the row of `gpui:tests/wanted.rs` for the scene `guest-connections` ("the footer of the guest connections"), matched to this test by its area, scene and subject, not by name |
| `external_tools_tests.rs` | `local_tool_configuration_saves_separate_arguments_and_can_remove_every_tool` | runtime | ported | `ui:areas/vm_tools/tests.rs::rune_local_tool_configuration_saves_separate_arguments_and_can_remove_every_tool` |  |
| `external_tools_tests.rs` | `captured_guest_checks_pool_generation_power_and_current_reported_ip` | pure | moved | `ui:areas/vm_tools/tests.rs::rune_captured_guest_checks_pool_generation_power_and_current_reported_ip` |  |
| `external_tools_tests.rs` | `delayed_command_review_cannot_apply_after_edit_or_close` | pure | moved | `ui:areas/vm_tools/tests.rs::rune_delayed_command_review_cannot_apply_after_edit_or_close` |  |
| `failure_help.rs` | `exact_envelopes_only_and_unknown_outcome_takes_precedence_over_host_advice` | pure | moved | `ui:areas/events/failure_help.rs::exact_envelopes_only_and_unknown_outcome_takes_precedence_over_host_advice` |  |
| `failure_help.rs` | `captured_help_remains_read_only_and_close_is_visible_at_both_sizes` | runtime | split | `ui:areas/events/tests.rs::captured_help_remains_read_only_and_close_is_visible_at_both_sizes`; `gpui:tests/areas.rs::the_close_button_of_failure_help_is_inside_the_window_and_closes_it` |  |
| `guest_tools_view.rs` | `tools_review_captures_explicit_media_and_identity_and_fixed_footer_at_both_sizes` | runtime | split | `ui:areas/vm_tools/tests.rs::rune_tools_review_captures_explicit_media_and_identity`; `gpui:tests/wanted.rs::vm_tools` | the window half is the row of `gpui:tests/wanted.rs` for the scene `guest-tools` ("the guest tools review after choosing an image"), matched to this test by its area, scene and subject, not by name |
| `guest_tools_view.rs` | `tools_review_explains_existing_media_and_missing_devices_without_issuing_commands` | runtime | ported | `ui:areas/vm_tools/tests.rs::rune_tools_review_explains_existing_media_and_missing_devices_without_issuing_commands` |  |
| `guest_tools_view.rs` | `tools_panel_refuses_device_view_from_another_pool` | pure | moved | `ui:areas/vm_tools/tests.rs::rune_tools_panel_refuses_device_view_from_another_pool` |  |
| `ha_policy_view.rs` | `ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes` | runtime | split | `ui:areas/pool/tests.rs::ha_policy_review_binds_vm_and_shared_vapp_settings`; `gpui:tests/wanted.rs::pool`; `gpui:tests/wanted_events.rs::ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `ha_policy_view.rs` | `ha_policy_edits_refuse_invalid_inputs_and_keep_timeout_precedence_explicit` | pure | moved | `ui:areas/pool/tests.rs::ha_policy_edits_refuse_invalid_inputs_and_keep_timeout_precedence_explicit` |  |
| `ha_policy_view.rs` | `ha_policy_missing_membership_refuses_review_and_selecting_vm_resets_draft` | pure | moved | `ui:areas/pool/tests.rs::ha_policy_missing_membership_refuses_review_and_selecting_vm_resets_draft` |  |
| `host_admin.rs` | `maintenance_review_keeps_host_and_guest_identity_at_both_widths` | runtime | split | `ui:areas/host_admin/tests.rs::maintenance_review_keeps_host_and_guest_identity`; `gpui:tests/areas.rs::the_confirm_button_of_host_administration_is_on_screen_and_submits_nothing_in_the_demo` |  |
| `host_admin.rs` | `memory_editor_rejects_zero_overflow_and_physical_overcommit` | pure | moved | `ui:areas/host_admin/tests.rs::memory_editor_rejects_zero_overflow_and_physical_overcommit` |  |
| `host_admin.rs` | `logging_review_keeps_exact_old_and_new_destination` | pure | moved | `ui:areas/host_admin/tests.rs::logging_review_keeps_exact_old_and_new_destination` |  |
| `host_admin.rs` | `autostart_is_explicitly_pool_scoped_and_unknown_is_not_false` | pure | moved | `ui:areas/host_admin/tests.rs::autostart_is_explicitly_pool_scoped_and_unknown_is_not_false` |  |
| `host_admin.rs` | `forget_requires_offline_disabled_noncoordinator_and_explicit_power_off_ack` | pure | moved | `ui:areas/host_admin/tests.rs::forget_requires_offline_disabled_noncoordinator_and_explicit_power_off_ack` |  |
| `host_console.rs` | `host_console_keeps_host_details_and_never_offers_guest_power_or_device_actions` | runtime | ported | `ui:areas/console/tests.rs::host_console_keeps_host_details_and_never_offers_guest_power_or_device_actions` |  |
| `host_monitoring_view_tests.rs` | `destructive_review_keeps_captured_source_and_reachable_controls_at_both_widths` | runtime | split | `ui:areas/performance/tests.rs::destructive_review_keeps_captured_source_and_reachable_controls`; `gpui:tests/wanted.rs::performance` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `host_monitoring_view_tests.rs` | `refresh_discards_catalog_and_review_and_enabled_source_cannot_start_again` | pure | moved | `ui:areas/performance/tests.rs::refresh_discards_catalog_and_review_and_enabled_source_cannot_start_again` |  |
| `host_monitoring_view_tests.rs` | `history_uses_captured_pool_after_switching_the_visible_inventory` | runtime | ported | `ui:areas/performance/tests.rs::history_uses_captured_pool_after_switching_the_visible_inventory` |  |
| `host_power_view.rs` | `review_has_private_password_and_captured_target_at_both_widths` | runtime | split | `ui:areas/host_services/tests.rs::review_has_private_password_and_captured_target`; `gpui:tests/wanted.rs::host_services`; `gpui:tests/wanted_events.rs::review_has_private_password_and_captured_target_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `host_power_view.rs` | `configuration_review_refuses_invalid_inputs_and_unknown_configuration` | pure | moved | `ui:areas/host_services/tests.rs::configuration_review_refuses_invalid_inputs_and_unknown_configuration` |  |
| `host_power_view.rs` | `switching_mode_discards_pending_password_and_review` | pure | moved | `ui:areas/host_services/tests.rs::switching_mode_discards_pending_password_and_review` |  |
| `host_support_view.rs` | `host_archive_review_captures_target_and_keeps_compact_footer_visible` | runtime | split | `ui:areas/host_services/tests.rs::host_archive_review_captures_target`; `gpui:tests/wanted.rs::host_services`; `gpui:tests/wanted_events.rs::host_archive_review_captures_target_and_keeps_compact_footer_visible` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `host_support_view.rs` | `crashdump_delete_review_captures_exact_dump_and_keeps_footer_visible` | runtime | split | `ui:areas/host_services/tests.rs::crashdump_delete_review_captures_exact_dump`; `gpui:tests/wanted.rs::host_services` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `host_support_view.rs` | `crashdump_catalogue_pages_make_every_captured_candidate_reachable` | runtime | split | `ui:areas/host_services/tests.rs::crashdump_catalogue_pages_make_every_captured_candidate_reachable`; `gpui:tests/wanted_events.rs::crashdump_catalogue_pages_make_every_captured_candidate_reachable` |  |
| `host_support_view.rs` | `crashdump_delete_refuses_unadvertised_review_or_changed_host` | runtime | ported | `ui:areas/host_services/tests.rs::crashdump_delete_refuses_unadvertised_review_or_changed_host` |  |
| `host_support_view.rs` | `stale_native_file_callback_cannot_set_a_reopened_host_review` | runtime | ported | `ui:areas/host_services/tests.rs::stale_native_file_callback_cannot_set_a_reopened_host_review` |  |
| `host_support_view.rs` | `unknown_capabilities_and_changed_host_version_cannot_be_reviewed` | runtime | ported | `ui:areas/host_services/tests.rs::unknown_capabilities_and_changed_host_version_cannot_be_reviewed` |  |
| `host_support_view.rs` | `restore_native_open_inspects_captured_host_and_requires_exact_confirmation` | runtime | split | `ui:areas/host_services/tests.rs::restore_native_open_inspects_captured_host_and_requires_exact_confirmation`; `gpui:tests/wanted.rs::host_services` |  |
| `host_support_view.rs` | `restore_rejects_wrong_host_and_cancels_obsolete_inspections` | runtime | ported | `ui:areas/host_services/tests.rs::restore_rejects_wrong_host_and_cancels_obsolete_inspections` |  |
| `instance_launch.rs` | `previews_and_help_do_not_claim_or_activate_live_desktop` | pure | moved | `ui:launch.rs::previews_and_help_do_not_claim_or_activate_live_desktop` |  |
| `instance_launch.rs` | `a_version_question_is_recognised_wherever_it_stands` | pure | moved | `ui:launch.rs::a_version_question_is_recognised_wherever_it_stands` |  |
| `instance_launch.rs` | `quiet_state_has_no_backend_ownership_profile_restore_or_inspection` | pure | moved | `ui:launch.rs::quiet_state_has_no_backend_ownership_profile_restore_or_inspection` |  |
| `inventory_information.rs` | `information_keeps_its_object_and_pool_and_refreshes_only_the_same_pool` | runtime | ported | `ui:areas/general/tests.rs::the_general_tab_of_a_vm_and_of_the_pool_list_the_rune_fields`; `ui:areas/general/tests.rs::the_general_tab_is_live` | the field lists and that the tab is live. Retired with the dialog: the panel keeping its target while the selection moves, Refresh from client, the Pool information button and Close |
| `inventory_information.rs` | `captured_host_hardware_and_pool_entry_point_are_reachable` | runtime | ported | `ui:areas/general/tests.rs::the_general_tab_of_a_server_lists_its_hardware` | the `pool-information` entry point is retired: the pool's General tab replaces it |
| `inventory_menu_tests.rs` | `right_click_any_column_or_padding_targets_the_visible_row_at_both_widths` | runtime | ported | `gpui:tests/shell.rs::a_grid_row_that_stands_for_an_object_has_its_context_menu_at_the_pointer`; `gpui:tests/renderer.rs::a_row_that_stands_for_an_object_asks_the_model_for_its_context_menu` | one press position and one window size; the three positions of the Rune test are not repeated |
| `inventory_menu_tests.rs` | `menu_key_and_shift_f10_preserve_keyboard_selection_and_open_current_row` | runtime | ported | `gpui:tests/renderer.rs::a_row_that_stands_for_an_object_asks_the_model_for_its_context_menu` | Menu and Shift+F10 on a grid row |
| `inventory_menu_tests.rs` | `header_and_blank_space_do_not_invent_a_row_target` | runtime | ported | `gpui:tests/shell.rs::a_tree_row_has_the_context_menu_of_its_object`; `gpui:tests/grid_menu.rs::header_and_blank_space_do_not_invent_a_row_target` | the tree's half (a heading has no menu) and the grid's half: a secondary press on the grid's header or below its rows opens no row menu |
| `inventory_menu_tests.rs` | `stale_menu_cannot_apply_to_a_removed_row_or_changed_pool` | runtime | ported | `ui:model_tests.rs::stale_menu_cannot_apply_to_a_removed_row_or_changed_pool` |  |
| `inventory_menu_tests.rs` | `keyboard_menu_follows_virtual_scrolling_and_changed_selection_cannot_start_a_batch` | runtime | ported | `ui:model_tests.rs::changed_selection_cannot_start_a_batch` | the rule about the selection; the menu key on a scrolled grid is `gpui:tests/renderer.rs::a_row_that_stands_for_an_object_asks_the_model_for_its_context_menu` (no virtual scrolling in it) |
| `inventory_report.rs` | `reports_keep_pool_scope_and_explicit_stale_incomplete_coverage` | pure | moved | `ui:areas/search/tests.rs::reports_keep_pool_scope_and_explicit_stale_incomplete_coverage` |  |
| `inventory_report.rs` | `report_limits_refuse_silent_truncation_or_invalid_filters` | pure | moved | `ui:areas/search/tests.rs::report_limits_refuse_silent_truncation_or_invalid_filters` |  |
| `inventory_report.rs` | `native_report_export_captures_results_and_ignores_cancelled_or_stale_dialogs` | runtime | ported | `ui:areas/search/tests.rs::native_report_export_captures_results_and_ignores_cancelled_or_stale_dialogs` |  |
| `local_conversion_view_tests.rs` | `offline_form_and_review_keep_heading_and_final_controls_reachable_without_host_access` | runtime | split | `ui:areas/transfers/conversion_tests.rs::offline_form_and_review_keep_heading_and_final_controls_reachable_without_host_access`; `gpui:tests/wanted.rs::transfers` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `local_conversion_view_tests.rs` | `native_file_dialog_cancellation_and_late_result_do_not_change_closed_form` | runtime | ported | `ui:areas/transfers/conversion_tests.rs::native_file_dialog_cancellation_and_late_result_do_not_change_closed_form` |  |
| `local_conversion_view_tests.rs` | `background_job_controls_do_not_retarget_and_disconnection_is_reported` | runtime | ported | `ui:areas/transfers/conversion_tests.rs::background_job_controls_do_not_retarget_and_disconnection_is_reported` |  |
| `local_conversion_view_tests.rs` | `successful_conversion_only_offers_import_and_never_creates_a_host_operation` | pure | moved | `ui:areas/transfers/conversion_tests.rs::successful_conversion_only_offers_import_and_never_creates_a_host_operation` |  |
| `main.rs` | `the_shared_registry_lists_every_app_scene` | pure | split | `ui:scene.rs::every_rune_scene_is_registered_or_listed_as_unported`; `gpui:tests/renderer.rs::every_scene_renders_with_the_keys_of_its_description` | the Rune test compared two registries; now the 129 names must be registered and every registered scene must render |
| `main.rs` | `sidebar_links_to_the_repository_without_a_demo_entry` | runtime | ported | `ui:areas/settings/tests.rs::the_commands_are_in_the_tools_and_help_menus` | the link is the command `help.website` and the button `help-website` of About; the sidebar header is gone |
| `main.rs` | `action_checks_follow_vm_availability_without_expiring_on_unrelated_events` | pure | moved | `ui:diagnostics.rs::action_checks_follow_vm_availability_without_expiring_on_unrelated_events` |  |
| `main.rs` | `action_checks_require_the_same_current_connection_and_vm_state` | pure | moved | `ui:diagnostics.rs::action_checks_require_the_same_current_connection_and_vm_state` |  |
| `main.rs` | `devices_keep_their_target_and_leave_compact_details` | runtime | ported | `ui:areas/vm_devices/tests.rs::reseeding_the_demo_closes_the_device_surfaces` | the part that still has a subject: seeding the demo closes the device surfaces. The compact details swap is gone |
| `main.rs` | `editing_the_host_resets_a_remembered_certificate_bypass` | runtime | ported | `ui:areas/connections/tests.rs::add_server_opens_the_empty_form_and_cancel_forgets_the_typed_password` | one test of the Add Server form |
| `main.rs` | `connection_form_exposes_one_opt_out_and_keeps_verified_options_available` | runtime | ported | `ui:areas/connections/tests.rs::add_server_opens_the_empty_form_and_cancel_forgets_the_typed_password`; `ui:areas/connections/tests.rs::the_six_connection_scenes_show_what_the_rune_scenes_showed` | the `storage_bytes` assertions of the Rune test are not repeated |
| `main.rs` | `search_and_host_filter_select_matching_objects` | pure | split | `ui:areas/search/tests.rs::the_tree_selection_presets_the_scope_of_the_query`; `gpui:tests/shell.rs::the_view_switcher_and_the_search_box_drive_the_model` | the Rune table's own text and host filter went with the table at the cutover, and with them the model test `a_host_filter_keeps_only_the_guests_on_that_host` of `ui:rows.rs` that first succeeded this one. A selected server scopes the Search tab's query to the guests on it (the model test named); the text filter of the navigation pane is the window test named |
| `main.rs` | `real_widgets_open_the_connection_dialog_and_filter_demo_inventory` | runtime | split | `ui:areas/connections/tests.rs::add_server_opens_the_empty_form_and_cancel_forgets_the_typed_password`; `gpui:tests/shell.rs::the_view_switcher_and_the_search_box_drive_the_model` |  |
| `main.rs` | `measure_large_inventory` | runtime | ported | `gpui:tests/large_inventory.rs::measure_large_inventory`; `gpui:tests/large_inventory.rs::the_work_of_a_frame_does_not_grow_with_the_inventory` | an `#[ignore]` measurement of the scene `large-inventory` (16 hosts, 2,000 guests) with the offscreen renderer: first frame, scroll, selection change, replaced inventory, as p50 and p95. Beside it `gpui:tests/large_inventory.rs::the_work_of_a_frame_does_not_grow_with_the_inventory` is a check that counts the rows built. One set of numbers was taken on a loaded machine (not recorded in the repository); the plan's record at G2 and G5 on an idle machine is still to be made |
| `main.rs` | `hosts_outside_the_qualified_release_are_named_in_a_notice` | runtime | ported | `ui:model_tests.rs::hosts_outside_the_qualified_release_are_named_in_a_notice` |  |
| `main.rs` | `dialogs_outside_the_supported_core_carry_the_experimental_notice` | runtime | split | `ui:areas/cross_pool/tests.rs::rune_experimental_notice_is_on_both_wizards`; `ui:areas/events/tests.rs::activity_and_its_dialogs_carry_no_experimental_notice`; `ui:areas/host_admin/tests.rs::only_changes_outside_the_supported_part_carry_the_experimental_notice`; `ui:areas/host_services/tests.rs::every_surface_of_the_area_carries_the_experimental_notice`; `ui:areas/migration/tests.rs::rune_experimental_notice_is_on_the_batch_and_not_on_the_single_move`; `ui:areas/mod.rs::the_experimental_notice_follows_the_surface_in_front`; `ui:areas/network/tests.rs::every_network_surface_carries_the_experimental_notice`; `ui:areas/network_services/tests.rs::every_network_services_surface_carries_the_experimental_notice`; `ui:areas/organization/tests.rs::both_dialogs_and_the_page_carry_the_experimental_notice`; `ui:areas/pool/tests.rs::every_surface_of_the_area_carries_the_experimental_notice`; `ui:areas/pool_security/tests.rs::every_dialog_carries_the_experimental_notice`; `ui:areas/power/tests.rs::rune_experimental_notice_follows_the_rune_status_of_each_surface`; `ui:areas/schedules_alarms/tests.rs::every_surface_of_the_area_carries_the_experimental_notice`; `ui:areas/snapshots/tests.rs::rune_main_dialogs_outside_the_supported_core_carry_the_experimental_notice`; `ui:areas/storage/tests.rs::every_storage_surface_carries_the_experimental_notice`; `ui:areas/vm_lifecycle/tests.rs::rune_experimental_notice_is_on_every_surface_of_the_area`; `ui:areas/vm_tools/tests.rs::every_scene_describes_its_experimental_dialog_with_unique_keys_and_a_quiet_first_poll`; `gpui:tests/wanted.rs::snapshots` | the front-most rule is the kernel's; each area asserts the flag of its own surfaces |
| `main.rs` | `interface_size_scales_the_layout_and_follows_ctrl_plus_minus_and_zero` | runtime | ported | `ui:prefs.rs::the_interface_scale_steps_through_the_offered_sizes_and_holds_at_the_ends` | the `Options` part and the three commands; the keys and the scale of the window are the shell's |
| `main.rs` | `column_and_sidebar_widths_reach_the_preferences_and_survive_a_table_rebuild` | pure | moved | `ui:model_tests.rs::widths_are_kept_in_the_preferences_and_saved_once_they_rest` |  |
| `main.rs` | `the_details_pane_width_reaches_the_preferences_and_the_next_start_uses_it` | pure | moved | `ui:prefs.rs::widths_a_file_holds_are_brought_inside_what_the_client_accepts` | what a preferences file may hold. The inspector half is retired: the new shell has no details pane |
| `main.rs` | `widths_are_saved_shortly_after_they_stop_changing_without_waiting_for_close` | runtime | ported | `ui:model_tests.rs::widths_are_kept_in_the_preferences_and_saved_once_they_rest` |  |
| `main.rs` | `search_clear_button_empties_the_search_and_restores_every_row` | runtime | ported | `gpui:tests/shell.rs::the_search_box_clear_button_empties_the_search_and_restores_every_row` | the pointer presses the box's own clear button (the Kit's, `Input::cleanable`): the text and the model's filter are empty and every row is back. Typing into the box is `gpui:tests/shell.rs::the_view_switcher_and_the_search_box_drive_the_model` |
| `main.rs` | `narrow_window_keeps_connection_and_vm_details_reachable` | runtime | retired | `gpui:tests/shell.rs::the_pane_gives_way_to_a_toggle_in_a_narrow_window` | the compact details swap is gone and the minimum window is 800x600 (plan D3); what a narrow window does now is the test named |
| `management_bond_ui.rs` | `management_guest_query_result_is_fenced_to_exact_form_and_pool` | pure | moved | `ui:areas/management_network/tests.rs::management_guest_query_result_is_fenced_to_exact_form_and_pool` |  |
| `management_bond_ui.rs` | `management_guest_impact_review_lists_moved_retained_and_no_eligible_guests` | runtime | split | `ui:areas/management_network/tests.rs::management_guest_impact_review_lists_moved_retained_and_no_eligible_guests`; `gpui:tests/wanted.rs::management_network` |  |
| `management_bond_ui.rs` | `management_guest_impact_becomes_stale_when_consumers_or_guest_placement_change` | pure | moved | `ui:areas/management_network/tests.rs::management_guest_impact_becomes_stale_when_consumers_or_guest_placement_change` |  |
| `management_bond_ui.rs` | `management_bond_review_has_exact_members_and_recovery_at_both_widths` | runtime | split | `ui:areas/management_network/tests.rs::management_bond_review_has_exact_members_and_recovery`; `gpui:tests/wanted.rs::management_network`; `gpui:tests/wanted_events.rs::management_bond_review_has_exact_members_and_recovery_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `management_bond_ui.rs` | `removal_review_captures_the_existing_primary_member_and_refuses_changed_bond` | pure | moved | `ui:areas/management_network/tests.rs::removal_review_captures_the_existing_primary_member_and_refuses_changed_bond` |  |
| `management_bond_ui.rs` | `management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths` | runtime | split | `ui:areas/management_network/tests.rs::management_vlan_bond_review_lists_recreated_ids_and_captures_parent`; `gpui:tests/wanted.rs::management_network`; `gpui:tests/wanted_events.rs::management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `management_bond_ui.rs` | `changed_member_or_added_consumer_invalidates_captured_bond_review` | pure | moved | `ui:areas/management_network/tests.rs::changed_member_or_added_consumer_invalidates_captured_bond_review` |  |
| `management_ipv6_bond_ui.rs` | `ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled` | runtime | split | `ui:areas/management_network/tests.rs::ipv6_stage_review_keeps_demo_apply_disabled`; `gpui:tests/wanted.rs::management_network`; `gpui:tests/wanted_events.rs::ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `management_ipv6_bond_ui.rs` | `ipv6_review_refuses_changed_temporary_or_member_identity` | pure | moved | `ui:areas/management_network/tests.rs::ipv6_review_refuses_changed_temporary_or_member_identity` |  |
| `management_ipv6_bond_ui.rs` | `ipv6_continuation_retains_original_review_and_targets_exact_predecessor` | pure | moved | `ui:areas/management_network/tests.rs::ipv6_continuation_retains_original_review_and_targets_exact_predecessor` |  |
| `management_network_ui.rs` | `vlan_over_bond_review_captures_full_topology_at_both_widths` | runtime | split | `ui:areas/management_network/tests.rs::vlan_over_bond_review_captures_full_topology`; `gpui:tests/wanted.rs::management_network`; `gpui:tests/wanted_events.rs::vlan_over_bond_review_captures_full_topology_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `management_network_ui.rs` | `topology_review_tracks_membership_protection_and_metadata_but_not_link_counters` | pure | moved | `ui:areas/management_network/tests.rs::topology_review_tracks_membership_protection_and_metadata_but_not_link_counters` |  |
| `management_network_ui.rs` | `bond_members_and_nested_vlan_targets_do_not_form_a_management_review` | pure | moved | `ui:areas/management_network/tests.rs::bond_members_and_nested_vlan_targets_do_not_form_a_management_review` |  |
| `management_network_ui.rs` | `automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo` | runtime | split | `ui:areas/management_network/tests.rs::automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo`; `gpui:tests/wanted.rs::management_network`; `gpui:tests/wanted_events.rs::automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo` |  |
| `management_network_ui.rs` | `automatic_review_rejects_wrong_family_hostnames_noop_and_changed_lease` | pure | moved | `ui:areas/management_network/tests.rs::automatic_review_rejects_wrong_family_hostnames_noop_and_changed_lease` |  |
| `management_network_ui.rs` | `ipv6_dhcp_and_autoconf_review_require_same_family_and_capture_stable_recovery` | pure | moved | `ui:areas/management_network/tests.rs::ipv6_dhcp_and_autoconf_review_require_same_family_and_capture_stable_recovery` |  |
| `management_network_ui.rs` | `management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch` | runtime | split | `ui:areas/management_network/tests.rs::management_review_captures_recovery_settings_without_demo_dispatch`; `gpui:tests/wanted.rs::management_network`; `gpui:tests/wanted_events.rs::management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `management_network_ui.rs` | `management_review_refuses_changed_source_pool_ha_and_nonstandard_mtu` | pure | moved | `ui:areas/management_network/tests.rs::management_review_refuses_changed_source_pool_ha_and_nonstandard_mtu` |  |
| `management_network_ui.rs` | `management_move_review_captures_the_destination_and_retains_old_address` | pure | moved | `ui:areas/management_network/tests.rs::management_move_review_captures_the_destination_and_retains_old_address` |  |
| `management_network_ui.rs` | `management_ipv6_review_keeps_installed_family_and_rejects_invalid_recovery_address` | pure | moved | `ui:areas/management_network/tests.rs::management_ipv6_review_keeps_installed_family_and_rejects_invalid_recovery_address` |  |
| `metric_presets.rs` | `save_apply_and_remove_presets_preserve_the_target_at_both_sizes` | runtime | split | `ui:areas/performance/tests.rs::save_apply_and_remove_presets_preserve_the_target`; `gpui:tests/wanted.rs::performance` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `metric_presets.rs` | `missing_sources_and_other_target_types_never_silently_change_the_graph` | pure | moved | `ui:areas/performance/tests.rs::missing_sources_and_other_target_types_never_silently_change_the_graph` |  |
| `metric_presets.rs` | `invalid_preset_does_not_replace_previous_definition` | pure | moved | `ui:areas/performance/presets.rs::invalid_preset_does_not_replace_previous_definition` |  |
| `metrics_view.rs` | `graph_preserves_missing_samples_time_gaps_zero_and_finite_extremes` | pure | moved | `ui:areas/performance/metrics.rs::graph_preserves_missing_samples_time_gaps_zero_and_finite_extremes` |  |
| `metrics_view.rs` | `timestamps_have_explicit_utc_calendar_dates_for_long_ranges` | pure | moved | `ui:areas/performance/metrics.rs::timestamps_have_explicit_utc_calendar_dates_for_long_ranges` |  |
| `metrics_view.rs` | `performance_controls_show_actual_coverage_and_change_ranges_without_host_writes` | runtime | split | `ui:areas/performance/tests.rs::performance_controls_show_actual_coverage_and_change_ranges_without_host_writes`; `gpui:tests/wanted.rs::performance` |  |
| `network_admin.rs` | `compact_and_wide_network_reviews_remain_visible_and_preview_cannot_submit` | runtime | split | `ui:areas/network/tests.rs::compact_and_wide_network_reviews_remain_visible_and_preview_cannot_submit`; `gpui:tests/wanted.rs::network` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_admin.rs` | `mtu_review_captures_original_value_instead_of_chasing_inventory` | pure | moved | `ui:areas/network/tests.rs::mtu_review_captures_original_value_instead_of_chasing_inventory` |  |
| `network_fabric_pool_ui.rs` | `exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply` | runtime | split | `ui:areas/network_services/tests.rs::exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply`; `gpui:tests/wanted.rs::network_services`; `gpui:tests/wanted_events.rs::exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_fabric_pool_ui.rs` | `pool_controller_and_input_changes_invalidate_captured_review` | pure | moved | `ui:areas/network_services/tests.rs::pool_controller_and_input_changes_invalidate_captured_review` |  |
| `network_fabric_pool_ui.rs` | `input_editor_requires_all_missing_hosts_and_preserves_exact_selection` | runtime | split | `ui:areas/network_services/tests.rs::input_editor_requires_all_missing_hosts_and_preserves_exact_selection`; `gpui:tests/wanted.rs::network_services` |  |
| `network_fabric_ui.rs` | `host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit` | runtime | split | `ui:areas/network_services/tests.rs::host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit`; `gpui:tests/wanted.rs::network_services`; `gpui:tests/wanted_events.rs::host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_fabric_ui.rs` | `review_fences_changed_controller_transport_and_pool` | pure | moved | `ui:areas/network_services/tests.rs::review_fences_changed_controller_transport_and_pool` |  |
| `network_fabric_ui.rs` | `sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes` | runtime | split | `ui:areas/network_services/tests.rs::sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes`; `gpui:tests/wanted.rs::network_services`; `gpui:tests/wanted_events.rs::sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_interfaces.rs` | `ipv6_review_is_visible_at_both_widths_and_keeps_captured_identity` | runtime | split | `ui:areas/network/tests.rs::ipv6_review_is_visible_at_both_widths_and_keeps_captured_identity`; `gpui:tests/wanted.rs::network` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_interfaces.rs` | `interface_review_refuses_changed_identity_configuration_and_pool` | pure | moved | `ui:areas/network/tests.rs::interface_review_refuses_changed_identity_configuration_and_pool` |  |
| `network_interfaces.rs` | `purpose_label_is_separate_from_unplug_protection_and_invalid_ipv6_is_refused` | pure | moved | `ui:areas/network/tests.rs::purpose_label_is_separate_from_unplug_protection_and_invalid_ipv6_is_refused` |  |
| `network_service_ui.rs` | `review_captures_plaintext_policy_and_interfaces_at_both_widths` | runtime | split | `ui:areas/network_services/tests.rs::review_captures_plaintext_policy_and_interfaces_at_both_widths`; `gpui:tests/wanted.rs::network_services`; `gpui:tests/wanted_events.rs::review_captures_plaintext_policy_and_interfaces_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_service_ui.rs` | `changed_interface_pool_or_service_policy_invalidates_captured_review` | pure | moved | `ui:areas/network_services/tests.rs::changed_interface_pool_or_service_policy_invalidates_captured_review` |  |
| `network_topology_ui.rs` | `pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch` | runtime | split | `ui:areas/network_services/tests.rs::pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch`; `gpui:tests/wanted.rs::network_services`; `gpui:tests/wanted_events.rs::pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `network_topology_ui.rs` | `topology_review_refuses_changed_pool_graph_and_keeps_captured_identities` | pure | moved | `ui:areas/network_services/tests.rs::topology_review_refuses_changed_pool_graph_and_keeps_captured_identities` |  |
| `network_topology_ui.rs` | `retag_review_refuses_protection_consumers_missing_inventory_and_unchanged_tag` | pure | moved | `ui:areas/network_services/tests.rs::retag_review_refuses_protection_consumers_missing_inventory_and_unchanged_tag` |  |
| `network_topology_ui.rs` | `vlan_zero_is_explicit_and_invalid_tags_cannot_reach_review` | pure | moved | `ui:areas/network_services/tests.rs::vlan_zero_is_explicit_and_invalid_tags_cannot_reach_review` |  |
| `network_topology_ui.rs` | `topology_export_contains_only_public_created_and_removed_ids` | pure | moved | `ui:areas/events/tests.rs::topology_export_contains_only_public_created_and_removed_ids` |  |
| `organization_view.rs` | `folder_rename_updates_descendants_and_registry_without_touching_other_paths` | pure | moved | `ui:areas/organization/tests.rs::folder_rename_updates_descendants_and_registry_without_touching_other_paths` |  |
| `organization_view.rs` | `custom_field_deletion_reviews_values_before_removing_definition` | pure | moved | `ui:areas/organization/tests.rs::custom_field_deletion_reviews_values_before_removing_definition` |  |
| `organization_view.rs` | `exact_tag_and_folder_filters_are_distinct_from_name_search` | pure | moved | `ui:areas/organization/tests.rs::exact_tag_and_folder_filters_are_distinct_from_name_search` |  |
| `organization_view.rs` | `compact_review_is_reachable_and_cannot_submit_demo_metadata` | runtime | split | `ui:areas/organization/tests.rs::compact_review_is_reachable_and_cannot_submit_demo_metadata`; `gpui:tests/wanted.rs::organization`; `gpui:tests/wanted_events.rs::compact_review_is_reachable_and_cannot_submit_demo_metadata` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `ovf_transfer_tests.rs` | `ovf_import_requires_explicit_network_mapping_and_reviews_captured_storage` | runtime | ported | `ui:areas/transfers/tests.rs::ovf_import_requires_explicit_network_mapping_and_reviews_captured_storage` |  |
| `ovf_transfer_tests.rs` | `ova_and_ovf_export_use_new_destinations_and_keep_captured_source` | runtime | ported | `ui:areas/transfers/tests.rs::ova_and_ovf_export_use_new_destinations_and_keep_captured_source` |  |
| `ovf_transfer_tests.rs` | `changing_package_format_discards_inspection_and_stale_picker_results` | runtime | ported | `ui:areas/transfers/tests.rs::changing_package_format_discards_inspection_and_stale_picker_results` |  |
| `pool_dissolve_view.rs` | `dissolve_review_keeps_exact_members_and_confirmation_reachable_at_both_widths` | runtime | split | `ui:areas/pool/tests.rs::dissolve_review_keeps_exact_members_and_confirmation_reachable`; `gpui:tests/wanted.rs::pool`; `gpui:tests/wanted_events.rs::the_pool_dissolve_review_submits_nothing_in_the_demo` | the window half is the row of `gpui:tests/wanted.rs` for the scene `pool-dissolve` ("the dissolve review"), matched to this test by its area, scene and subject, not by name |
| `pool_dissolve_view.rs` | `dissolve_capture_refuses_unknown_coordinator_ha_and_enabled_members_cannot_submit` | pure | moved | `ui:areas/pool/tests.rs::dissolve_capture_refuses_unknown_coordinator_ha_and_enabled_members_cannot_submit` |  |
| `pool_security_view.rs` | `certificate_files_are_regular_bounded_and_errors_do_not_include_contents` | pure | moved | `ui:areas/pool_security/tests.rs::certificate_files_are_regular_bounded_and_errors_do_not_include_contents` |  |
| `pool_security_view.rs` | `compact_review_keeps_footer_visible_and_captured_identity_immutable` | runtime | split | `ui:areas/pool_security/tests.rs::review_of_a_synthetic_pool_submits_nothing_and_keeps_the_captured_identity`; `gpui:tests/wanted.rs::pool_security`; `gpui:tests/wanted_events.rs::compact_review_keeps_footer_visible_and_captured_identity_immutable` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `pool_security_view.rs` | `native_certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection` | runtime | ported | `ui:areas/pool_security/tests.rs::certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection` |  |
| `pool_security_view.rs` | `late_certificate_validation_is_discarded_when_host_changes` | pure | moved | `ui:areas/pool_security/tests.rs::late_certificate_validation_is_discarded_when_host_changes` |  |
| `pool_view.rs` | `review_captures_target_and_compact_footer_never_submits_demo` | runtime | split | `ui:areas/pool/tests.rs::review_captures_target_and_never_submits_demo`; `gpui:tests/wanted.rs::pool`; `gpui:tests/wanted_events.rs::review_captures_target_and_compact_footer_never_submits_demo` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `pool_view.rs` | `ejection_requires_host_uuid_and_priority_unknown_is_not_disabled` | pure | moved | `ui:areas/pool/tests.rs::ejection_requires_host_uuid_and_priority_unknown_is_not_disabled` |  |
| `profile_pool_join.rs` | `compact_join_review_keeps_footer_reachable_and_never_dispatches_demo` | runtime | split | `ui:areas/pool/tests.rs::join_review_never_dispatches_demo`; `gpui:tests/wanted.rs::pool`; `gpui:tests/wanted_events.rs::the_pool_join_review_submits_nothing_in_the_demo` | the window half is the row of `gpui:tests/wanted.rs` for the scene `pool-join` ("the join review"), matched to this test by its area, scene and subject, not by name |
| `profile_pool_join.rs` | `destination_change_and_stale_connection_clear_transient_credentials` | pure | moved | `ui:areas/pool/tests.rs::destination_change_and_stale_connection_clear_transient_credentials` |  |
| `profile_reconnect.rs` | `reconnect_is_ephemeral_cancels_same_pool_aliases_and_retains_other_pool` | pure | moved | `ui:areas/connections/tests.rs::reconnect_is_ephemeral_cancels_same_pool_aliases_and_retains_other_pool` |  |
| `profile_reconnect.rs` | `changed_profile_refuses_reconnect_and_clears_entered_password` | pure | moved | `ui:areas/connections/tests.rs::changed_profile_refuses_reconnect_and_clears_entered_password` |  |
| `profile_reconnect.rs` | `compact_reconnect_review_cancel_never_changes_current_session` | runtime | split | `ui:areas/connections/tests.rs::compact_reconnect_review_cancel_never_changes_current_session`; `gpui:tests/areas.rs::the_heading_and_the_footer_of_the_reconnect_dialog_stay_reachable` |  |
| `profile_restore_tests.rs` | `startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring` | pure | moved | `ui:session.rs::startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring` |  |
| `profile_restore_tests.rs` | `bulk_loading_is_bounded_and_disconnect_all_invalidates_pending_credentials` | pure | moved | `ui:session.rs::bulk_loading_is_bounded_and_disconnect_all_invalidates_pending_credentials` |  |
| `profile_restore_tests.rs` | `edited_removed_and_cancelled_profiles_discard_late_results` | pure | moved | `ui:session.rs::edited_removed_and_cancelled_profiles_discard_late_results` |  |
| `profile_restore_tests.rs` | `retry_does_not_accept_an_old_attempt_and_editor_trust_edits_discard_loaded_passwords` | pure | moved | `ui:session.rs::retry_does_not_accept_an_old_attempt_and_editor_trust_edits_discard_loaded_passwords` |  |
| `profile_restore_tests.rs` | `concurrent_successes_connect_their_own_profiles_without_replacing_the_open_editor` | pure | moved | `ui:session.rs::concurrent_successes_connect_their_own_profiles_without_replacing_the_open_editor` |  |
| `profile_restore_tests.rs` | `rune_bulk_controls_show_profile_errors_and_cancel_pending_attempts` | runtime | ported | `ui:areas/connections/tests.rs::rune_bulk_controls_show_profile_errors_and_cancel_pending_attempts` |  |
| `profile_restore_tests.rs` | `forgetting_saved_credentials_also_disables_startup_restore` | pure | moved | `ui:session.rs::forgetting_saved_credentials_also_disables_startup_restore` |  |
| `profile_restore_tests.rs` | `previews_do_not_restore_profiles_and_keyring_mutations_cannot_race_profile_saves` | pure | moved | `ui:session.rs::previews_do_not_restore_profiles_and_keyring_mutations_cannot_race_profile_saves` |  |
| `profile_restore_tests.rs` | `headless_flags_do_not_restore_a_saved_session` | pure | moved | `ui:areas/connections/tests.rs::headless_flags_do_not_restore_a_saved_session`; `ui:launch.rs::headless_flags_are_previews_unless_native_is_asked_for` |  |
| `profile_restore_tests.rs` | `rune_startup_restore_checkbox_requires_explicit_keyring_choice` | runtime | ported | `ui:areas/connections/tests.rs::rune_startup_restore_checkbox_requires_explicit_keyring_choice` |  |
| `profile_root_password.rs` | `late_positive_evidence_never_saves_unknown_acknowledged_or_interrupted_completion` | pure | moved | `ui:areas/pool/tests.rs::late_positive_evidence_never_saves_unknown_acknowledged_or_interrupted_completion` |  |
| `profile_root_password.rs` | `review_is_pool_scoped_masked_and_footer_stays_reachable` | runtime | split | `ui:areas/pool/tests.rs::password_review_is_pool_scoped_and_masked`; `gpui:tests/wanted.rs::pool`; `gpui:tests/wanted_events.rs::the_root_password_review_submits_nothing_in_the_demo` | the window half is the row of `gpui:tests/wanted.rs` for the scene `root-password` ("the root password review"), matched to this test by its area, scene and subject, not by name |
| `profile_root_password.rs` | `mismatched_passwords_and_stale_profile_clear_without_submission` | pure | moved | `ui:areas/pool/tests.rs::mismatched_passwords_and_stale_profile_clear_without_submission` |  |
| `profile_root_password.rs` | `password_store_failure_preserves_host_result_and_never_retries_automatically` | pure | moved | `ui:areas/pool/tests.rs::password_store_failure_preserves_host_result_and_never_retries_automatically` |  |
| `profile_trust_tests.rs` | `compact_and_wide_editor_keep_header_and_connect_controls_visible` | runtime | split | `ui:areas/connections/tests.rs::compact_and_wide_editor_keep_header_and_connect_controls_visible`; `gpui:tests/areas.rs::the_heading_and_the_footer_of_the_connect_dialog_stay_reachable` |  |
| `profile_trust_tests.rs` | `reviewed_changed_pin_requires_exact_endpoint_and_explicit_fingerprint_check` | runtime | ported | `ui:areas/connections/tests.rs::reviewed_changed_pin_requires_exact_endpoint_and_explicit_fingerprint_check` |  |
| `profile_trust_tests.rs` | `delayed_inspection_and_review_cannot_follow_changed_route_pin_or_editor` | runtime | ported | `ui:areas/connections/tests.rs::delayed_inspection_and_review_cannot_follow_changed_route_pin_or_editor` |  |
| `profile_trust_tests.rs` | `proxy_credentials_are_session_only_and_bound_to_the_exact_proxy_configuration` | runtime | ported | `ui:areas/connections/tests.rs::proxy_credentials_are_session_only_and_bound_to_the_exact_proxy_configuration` |  |
| `profile_trust_tests.rs` | `synthetic_inspection_cannot_contact_an_endpoint_or_load_a_password` | runtime | ported | `ui:areas/connections/tests.rs::synthetic_inspection_cannot_contact_an_endpoint_or_load_a_password` |  |
| `profile_trust_tests.rs` | `endpoint_edit_drops_the_previous_certificate_pin` | pure | moved | `ui:areas/connections/tests.rs::endpoint_edit_drops_the_previous_certificate_pin` |  |
| `profile_trust_tests.rs` | `endpoint_edit_keeps_a_different_fingerprint_on_the_new_endpoint_only` | pure | moved | `ui:areas/connections/tests.rs::endpoint_edit_keeps_a_different_fingerprint_on_the_new_endpoint_only` |  |
| `profile_trust_tests.rs` | `unchanged_endpoint_keeps_its_certificate_pin` | pure | moved | `ui:areas/connections/tests.rs::unchanged_endpoint_keeps_its_certificate_pin` |  |
| `profile_trust_tests.rs` | `reviewing_failed_member_certificate_does_not_load_saved_host_credentials` | pure | moved | `ui:areas/connections/tests.rs::reviewing_failed_member_certificate_does_not_load_saved_host_credentials` |  |
| `profiles_view.rs` | `ca_picker_applies_to_its_editor_only_and_cancel_preserves_the_path` | runtime | ported | `ui:areas/connections/tests.rs::ca_picker_applies_to_its_editor_only_and_cancel_preserves_the_path` |  |
| `profiles_view.rs` | `saving_and_editing_a_profile_needs_no_connection_and_does_not_save_passwords` | pure | moved | `ui:session.rs::saving_and_editing_a_profile_needs_no_connection_and_does_not_save_passwords` |  |
| `profiles_view.rs` | `invalid_profile_settings_do_not_replace_saved_settings` | pure | moved | `ui:session.rs::invalid_profile_settings_do_not_replace_saved_settings` |  |
| `profiles_view.rs` | `second_window_cannot_write_profiles_or_migrate_legacy_settings` | pure | moved | `ui:session.rs::second_window_cannot_write_profiles_or_migrate_legacy_settings` |  |
| `profiles_view.rs` | `connections_dialog_exposes_saved_profiles_and_opens_an_editor` | runtime | ported | `ui:areas/connections/tests.rs::connections_dialog_exposes_saved_profiles_and_opens_an_editor` |  |
| `profiles_view.rs` | `saved_password_results_are_bound_to_the_open_profile_and_failures_allow_manual_input` | pure | moved | `ui:session.rs::saved_password_results_are_bound_to_the_open_profile_and_failures_allow_manual_input` |  |
| `resource_admin.rs` | `joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations` | runtime | split | `ui:areas/events/tests.rs::joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations`; `gpui:tests/wanted_events.rs::joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations` |  |
| `resource_admin.rs` | `network_review_captures_pool_name_mtu_and_never_submits_from_preview` | runtime | split | `ui:areas/network/tests.rs::network_review_captures_pool_name_mtu_and_never_submits_from_preview`; `gpui:tests/wanted.rs::network` |  |
| `resource_admin.rs` | `resource_review_keeps_original_selection_and_rejects_invalid_or_unchanged_values` | pure | moved | `ui:areas/network/tests.rs::resource_review_keeps_original_selection_and_rejects_invalid_or_unchanged_values` |  |
| `resource_admin.rs` | `used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens` | runtime | ported | `ui:areas/network/tests.rs::used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens`; `ui:areas/events/tests.rs::offline_pool_history_opens` | the history half is the `events` area's |
| `resource_admin.rs` | `compact_network_controls_remain_inside_the_window` | runtime | retired | `gpui:tests/wanted.rs::network` | it measured the buttons of the Rune page header (`manage-pool`, `create-network`, `add-connection`, `show-details`, `disconnect`) at two window sizes; that header does not exist any more. What took its place for a network, the tab's row of local actions, is held inside the window by the row of `gpui:tests/wanted.rs` named |
| `resource_details.rs` | `resource_details_show_relationships_and_unknown_attachment_state` | runtime | ported | `ui:areas/general/tests.rs::resource_tabs_show_relationships_and_unknown_attachment_state` |  |
| `search_files.rs` | `local_search_reader_is_bounded_and_refuses_symlinks_and_special_files` | pure | moved | `ui:areas/search/files.rs::local_search_reader_is_bounded_and_refuses_symlinks_and_special_files`; `ui:files.rs::the_reader_is_bounded_and_refuses_links_directories_and_devices` |  |
| `search_tests.rs` | `search_entry_point_is_reachable_at_both_window_sizes` | runtime | retired | `ui:areas/search/tests.rs::every_selection_has_a_search_tab_with_unique_keys` | the entry point is the Search tab, which every selection has; the test named covers that |
| `search_tests.rs` | `aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions` | pure | moved | `ui:areas/search/tests.rs::aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions`; `ui:model_tests.rs::two_saved_connections_to_one_pool_show_it_once_through_the_scope_session` |  |
| `search_tests.rs` | `navigation_refuses_removed_and_orphan_snapshots_instead_of_opening_another_vm` | pure | moved | `ui:areas/search/tests.rs::navigation_refuses_removed_and_orphan_snapshots_instead_of_opening_another_vm` |  |
| `search_tests.rs` | `cross_pool_results_keep_duplicate_vm_ids_distinct_and_synthetic_navigation_disabled` | runtime | ported | `ui:areas/search/tests.rs::cross_pool_results_keep_duplicate_vm_ids_distinct_and_synthetic_navigation_disabled` |  |
| `search_tests.rs` | `saving_and_loading_does_not_erase_imported_complex_filters` | runtime | ported | `ui:areas/search/tests.rs::saving_and_loading_does_not_erase_imported_complex_filters` |  |
| `search_tests.rs` | `imported_queries_require_review_and_conflicts_leave_existing_definitions_intact` | runtime | ported | `ui:areas/search/tests.rs::imported_queries_require_review_and_conflicts_leave_existing_definitions_intact` |  |
| `search_tests.rs` | `native_search_picker_cancellation_and_closed_workspace_callbacks_are_ignored` | runtime | ported | `ui:areas/search/tests.rs::native_search_picker_cancellation_and_closed_workspace_callbacks_are_ignored` |  |
| `session_permissions_view.rs` | `known_denial_disables_power_without_hiding_its_reason_at_both_widths` | runtime | split | `ui:areas/power/tests.rs::rune_known_denial_disables_power_with_its_reason_and_unknown_does_not`; `ui:permissions.rs::only_a_known_denial_gives_a_reason_to_disable`; `gpui:tests/wanted_events.rs::known_denial_disables_power_without_hiding_its_reason_at_both_widths` | the rule and the disabled commands with their reason; the window test reads the reason from the menu bar and the toolbar at one size, the two widths are not asserted |
| `session_permissions_view.rs` | `unknown_permissions_allow_review_but_grants_never_override_object_ineligibility` | runtime | ported | `ui:areas/power/tests.rs::rune_known_denial_disables_power_with_its_reason_and_unknown_does_not`; `ui:permissions.rs::only_a_known_denial_gives_a_reason_to_disable` | one test with the known-denial test |
| `session_permissions_view.rs` | `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard` | runtime | split | `ui:areas/bulk_edit/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/host_admin/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/host_services/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/network/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/organization/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/pool/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/schedules_alarms/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/storage/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/vm_lifecycle/tests.rs::review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/snapshots/tests.rs::rune_review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard`; `ui:areas/vm_tools/tests.rs::the_mount_review_explains_permission_and_the_demo_cannot_confirm`; `gpui:tests/wanted.rs::snapshots` | the Rune test ran over 13 scenes; each area that ported one of those reviews has its share |
| `settings.rs` | `options_cancel_and_apply_preserve_other_preferences_at_both_sizes` | runtime | ported | `ui:areas/settings/tests.rs::options_cancel_and_apply_preserve_other_preferences` | model half; by its name the Rune test promised reachability at its window sizes, and no window test names it |
| `settings.rs` | `legacy_preferences_default_safely_and_force_review_cannot_be_disabled` | pure | moved | `ui:prefs.rs::legacy_preferences_default_safely_and_force_review_cannot_be_disabled` |  |
| `settings.rs` | `help_topics_and_close_remain_reachable_on_compact_windows` | runtime | ported | `ui:areas/settings/tests.rs::help_topics_switch_and_close_stays_reachable` | model half; by its name the Rune test promised reachability at its window sizes, and no window test names it |
| `settings.rs` | `compact_console_details_returns_to_same_pinned_guest` | runtime | ported | `ui:areas/console/tests.rs::compact_console_details_returns_to_same_pinned_guest` | the compact swap between console and details pane is gone; that a console keeps its target is the console area's test of this name |
| `snapshot_capture.rs` | `cancelled_or_uncertain_snapshot_never_authorizes_preview_upload` | pure | moved | `ui:areas/snapshots/tests.rs::rune_cancelled_or_uncertain_snapshot_never_authorizes_preview_upload` |  |
| `snapshot_capture.rs` | `snapshot_capture_encoder_bounds_large_frames_and_rejects_invalid_data` | pure | moved | `ui:areas/snapshots/tests.rs::rune_snapshot_capture_encoder_bounds_large_frames_and_rejects_invalid_data` |  |
| `snapshot_capture.rs` | `snapshot_preview_opt_in_is_explicit_and_synthetic_confirm_cannot_dispatch` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_snapshot_preview_opt_in_is_explicit_and_synthetic_confirm_cannot_dispatch` |  |
| `snapshot_inspection.rs` | `snapshot_inspection_captures_identity_and_close_is_reachable_at_both_sizes` | runtime | split | `ui:areas/snapshots/tests.rs::rune_snapshot_inspection_captures_identity_and_close_is_reachable_at_both_sizes`; `gpui:tests/wanted.rs::snapshots` | the window half is the row of `gpui:tests/wanted.rs` for the scene `snapshot-information` ("the preview picture of the snapshot information"), matched to this test by its area, scene and subject, not by name; the inspection is a section of the Snapshots tab now and has no Close |
| `snapshot_inspection.rs` | `tree_and_list_keep_selection_and_show_current_branch` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_tree_and_list_keep_selection_and_show_current_branch` |  |
| `snapshot_inspection.rs` | `decoder_bounds_dimensions_and_supports_png_and_jpeg` | pure | moved | `ui:areas/snapshots/tests.rs::rune_decoder_bounds_dimensions_and_supports_png_and_jpeg` |  |
| `snapshot_inspection.rs` | `closing_or_replacing_the_panel_does_not_release_a_running_decoder_slot` | pure | moved | `ui:areas/snapshots/tests.rs::rune_closing_or_replacing_the_panel_does_not_release_a_running_decoder_slot` |  |
| `snapshot_inspection.rs` | `preview_request_refuses_a_changed_or_disconnected_workspace` | pure | moved | `ui:areas/snapshots/tests.rs::rune_preview_request_refuses_a_changed_or_disconnected_workspace` |  |
| `snapshot_policy_view.rs` | `schedule_review_captures_coordinator_and_pool_at_both_widths` | runtime | split | `ui:areas/schedules_alarms/tests.rs::schedule_review_captures_coordinator_and_pool`; `gpui:tests/wanted.rs::schedules_alarms` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `snapshot_policy_view.rs` | `invalid_minutes_retention_and_unknown_clock_block_review` | pure | moved | `ui:areas/schedules_alarms/tests.rs::invalid_minutes_retention_and_unknown_clock_block_review` |  |
| `snapshot_policy_view.rs` | `weekly_day_selection_and_checkpoint_are_reviewed` | runtime | ported | `ui:areas/schedules_alarms/tests.rs::weekly_day_selection_and_checkpoint_are_reviewed` |  |
| `storage_graph_view_tests.rs` | `reviewed_storage_graph_is_captured_reachable_and_never_submits_demo` | runtime | split | `ui:areas/storage_maintenance/tests.rs::reviewed_storage_graph_is_captured_and_never_submits_demo`; `gpui:tests/wanted.rs::storage_maintenance`; `gpui:tests/wanted_events.rs::reviewed_storage_graph_is_captured_reachable_and_never_submits_demo` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_graph_view_tests.rs` | `missing_repair_requires_selection_and_rejects_new_pair_before_review` | pure | moved | `ui:areas/storage_maintenance/tests.rs::missing_repair_requires_selection_and_rejects_new_pair_before_review` |  |
| `storage_graph_view_tests.rs` | `changed_multipath_attachment_and_pool_disable_captured_review` | pure | moved | `ui:areas/storage_maintenance/tests.rs::changed_multipath_attachment_and_pool_disable_captured_review` |  |
| `storage_graph_view_tests.rs` | `multipath_refuses_unknown_handler_and_nonmaintenance_host` | pure | moved | `ui:areas/storage_maintenance/tests.rs::multipath_refuses_unknown_handler_and_nonmaintenance_host` |  |
| `storage_graph_view_tests.rs` | `edit_host_selection_then_review_keeps_exact_uuid` | runtime | ported | `ui:areas/storage_maintenance/tests.rs::edit_host_selection_then_review_keeps_exact_uuid` |  |
| `storage_graph_view_tests.rs` | `partial_outcome_restore_is_a_new_review_and_unknown_cannot_offer_it` | runtime | ported | `ui:areas/storage_maintenance/tests.rs::partial_outcome_restore_is_a_new_review_and_unknown_cannot_offer_it` |  |
| `storage_graph_view_tests.rs` | `restore_cannot_turn_into_detach_after_state_changes` | pure | moved | `ui:areas/storage_maintenance/tests.rs::restore_cannot_turn_into_detach_after_state_changes` |  |
| `storage_local_view.rs` | `local_review_captures_operator_declaration_host_and_device_and_keeps_confirmation_reachable` | runtime | split | `ui:areas/storage/tests.rs::local_review_captures_operator_declaration_host_and_device_and_keeps_confirmation_reachable`; `gpui:tests/wanted.rs::storage` | the window half is the row of `gpui:tests/wanted.rs` for the scene `storage-local-review` ("local_review_captures_...: the device confirmation and the footer"), matched to this test by its area, scene and subject, not by name |
| `storage_local_view.rs` | `local_review_requires_current_dual_probe_and_explicit_operator_acknowledgement` | pure | moved | `ui:areas/storage/tests.rs::local_review_requires_current_dual_probe_and_explicit_operator_acknowledgement` |  |
| `storage_local_view.rs` | `local_known_repository_selection_chooses_nonformatting_introduction` | pure | moved | `ui:areas/storage/tests.rs::local_known_repository_selection_chooses_nonformatting_introduction` |  |
| `storage_local_view.rs` | `local_form_exposes_declaration_and_erase_acknowledgement_at_both_widths` | runtime | split | `ui:areas/storage/tests.rs::local_form_exposes_declaration_and_erase_acknowledgement_at_both_widths`; `gpui:tests/wanted.rs::storage` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_local_view.rs` | `local_path_focus_preserves_probe_but_text_edit_invalidates_it` | runtime | split | `ui:areas/storage/tests.rs::local_path_focus_preserves_probe_but_text_edit_invalidates_it`; `gpui:tests/wanted_events.rs::local_path_focus_preserves_probe_but_text_edit_invalidates_it` |  |
| `storage_maintenance_view_tests.rs` | `repair_review_captures_targets_and_remains_reachable_at_compact_and_wide_sizes` | runtime | split | `ui:areas/storage_maintenance/tests.rs::repair_review_captures_targets_and_stays_through_a_tab_change`; `gpui:tests/wanted.rs::storage_maintenance` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_maintenance_view_tests.rs` | `repair_requires_selection_and_rejects_changed_attachment_graph` | pure | moved | `ui:areas/storage_maintenance/tests.rs::repair_requires_selection_and_rejects_changed_attachment_graph` |  |
| `storage_maintenance_view_tests.rs` | `read_cache_review_explains_policy_and_preserves_target_at_both_widths` | runtime | split | `ui:areas/storage_maintenance/tests.rs::read_cache_review_explains_policy_and_preserves_target`; `gpui:tests/wanted.rs::storage_maintenance` | the window half is the row of `gpui:tests/wanted.rs` for the scene `storage-cache` ("the cache review: back to the form and review again"), matched to this test by its area, scene and subject, not by name |
| `storage_maintenance_view_tests.rs` | `cache_unknown_or_changed_current_policy_cannot_be_reviewed_or_submitted` | pure | moved | `ui:areas/storage_maintenance/tests.rs::cache_unknown_or_changed_current_policy_cannot_be_reviewed_or_submitted` |  |
| `storage_maintenance_view_tests.rs` | `selecting_a_host_attachment_reviews_only_that_exact_target_at_both_widths` | runtime | split | `ui:areas/storage_maintenance/tests.rs::selecting_a_host_attachment_reviews_only_that_exact_target`; `gpui:tests/wanted.rs::storage_maintenance` | the window half is the row of `gpui:tests/wanted.rs` for the scene `storage-maintenance` ("a connection is chosen for repair and reviewed"), matched to this test by its area, scene and subject, not by name |
| `storage_provisioning_view_tests.rs` | `hba_selection_requires_review_and_captures_host_device_and_allocation_at_both_widths` | runtime | split | `ui:areas/storage/tests.rs::hba_selection_requires_review_and_captures_host_device_and_allocation_at_both_widths`; `gpui:tests/wanted.rs::storage` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_provisioning_view_tests.rs` | `hba_review_refuses_unselected_device_changed_host_and_unadvertised_allocation` | pure | moved | `ui:areas/storage/tests.rs::hba_review_refuses_unselected_device_changed_host_and_unadvertised_allocation` |  |
| `storage_provisioning_view_tests.rs` | `hba_discovery_results_cannot_be_selected_after_host_or_connection_changes` | pure | moved | `ui:areas/storage/tests.rs::hba_discovery_results_cannot_be_selected_after_host_or_connection_changes` |  |
| `storage_provisioning_view_tests.rs` | `hba_review_explains_external_ownership_destructive_format_and_no_automatic_cleanup` | pure | moved | `ui:areas/storage/tests.rs::hba_review_explains_external_ownership_destructive_format_and_no_automatic_cleanup` |  |
| `storage_view.rs` | `authenticated_storage_review_is_captured_and_footer_remains_reachable` | runtime | split | `ui:areas/storage/tests.rs::authenticated_storage_review_is_captured_and_footer_remains_reachable`; `gpui:tests/wanted.rs::storage`; `gpui:tests/wanted_events.rs::authenticated_storage_review_is_captured_and_footer_remains_reachable` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_view.rs` | `storage_discovery_result_fence_includes_credential_changes` | pure | moved | `ui:areas/storage/tests.rs::storage_discovery_result_fence_includes_credential_changes` |  |
| `storage_view.rs` | `discovery_selection_is_reachable_bound_to_inputs_and_keeps_review_separate` | runtime | split | `ui:areas/storage/tests.rs::discovery_selection_is_reachable_bound_to_inputs_and_keeps_review_separate`; `gpui:tests/wanted.rs::storage` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_view.rs` | `chosen_lun_requires_separate_destructive_review` | pure | moved | `ui:areas/storage/tests.rs::chosen_lun_requires_separate_destructive_review` |  |
| `storage_view.rs` | `storage_review_preserves_inputs_and_compact_footer_never_submits_preview` | runtime | split | `ui:areas/storage/tests.rs::storage_review_preserves_inputs_and_compact_footer_never_submits_preview`; `gpui:tests/wanted.rs::storage`; `gpui:tests/wanted_events.rs::storage_review_preserves_inputs_and_compact_footer_never_submits_preview` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `storage_view.rs` | `storage_forget_and_destroy_have_distinct_review_and_exact_disk_set` | pure | moved | `ui:areas/storage/tests.rs::storage_forget_and_destroy_have_distinct_review_and_exact_disk_set` |  |
| `theme.rs` | `controls_are_distinguishable_from_the_surfaces_they_sit_on` | pure | moved | `gpui:tests/theme.rs::controls_are_distinguishable_from_the_surfaces_they_sit_on` | the same contrast ratios, asserted in both modes on the colours the GPUI front end draws with |
| `transfers.rs` | `native_save_captures_source_and_reviews_without_demo_writes` | runtime | ported | `ui:areas/transfers/tests.rs::native_save_captures_source_and_reviews_without_demo_writes` |  |
| `transfers.rs` | `cancelled_or_stale_file_dialog_does_not_replace_review` | runtime | ported | `ui:areas/transfers/tests.rs::cancelled_or_stale_file_dialog_does_not_replace_review` |  |
| `transfers.rs` | `offline_unknown_history_preserves_evidence_and_cannot_release_guard` | runtime | ported | `ui:areas/transfers/tests.rs::offline_unknown_history_preserves_evidence_and_cannot_release_guard` |  |
| `transfers.rs` | `switched_pool_invalidates_review_and_import_uses_open_dialog` | runtime | ported | `ui:areas/transfers/tests.rs::switched_pool_invalidates_review_and_import_uses_open_dialog` |  |
| `transfers.rs` | `native_raw_import_reviews_name_capacity_and_exact_storage_without_overwrite` | runtime | ported | `ui:areas/transfers/tests.rs::native_raw_import_reviews_name_capacity_and_exact_storage_without_overwrite` |  |
| `transfers.rs` | `disk_export_captures_identity_and_rejects_capacity_or_attachment_changes` | runtime | ported | `ui:areas/transfers/tests.rs::disk_export_captures_identity_and_rejects_capacity_or_attachment_changes` |  |
| `transfers.rs` | `format_change_invalidates_selected_file_and_blank_disk_name_cannot_apply` | runtime | ported | `ui:areas/transfers/tests.rs::format_change_invalidates_selected_file_and_blank_disk_name_cannot_apply` |  |
| `vm_creation_cloud.rs` | `optional_cloud_step_is_reachable_and_disabling_it_discards_the_draft` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::optional_cloud_step_is_reachable_and_disabling_it_discards_the_draft` | asserts the inputs of the Cloud-init step where Rune asserted a fourth step |
| `vm_creation_cloud.rs` | `creation_cloud_inputs_are_validated_and_review_omits_the_private_payload` | runtime | split | `ui:areas/vm_lifecycle/tests.rs::creation_cloud_inputs_are_validated_and_review_omits_the_private_payload`; `gpui:tests/wanted_events.rs::creation_cloud_inputs_are_validated_and_review_omits_the_private_payload` |  |
| `vm_creation_cloud.rs` | `creation_cloud_pending_requires_exact_success_and_never_uses_the_selection` | pure | moved | `ui:areas/vm_lifecycle/tests.rs::creation_cloud_pending_requires_exact_success_and_never_uses_the_selection` |  |
| `vm_creation_cloud.rs` | `creation_cloud_pending_controls_are_visible_and_synthetic_continue_never_prepares_media` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::creation_cloud_pending_controls_are_visible_and_synthetic_continue_never_prepares_media` | model half; by its name the Rune test promised reachability at its window sizes, and no window test names it |
| `vm_device_actions.rs` | `growth_input_preserves_exact_capacity_and_rejects_overflow` | pure | moved | `ui:areas/vm_devices/tests.rs::growth_input_preserves_exact_capacity_and_rejects_overflow` |  |
| `vm_device_lifecycle_tests.rs` | `disk_creation_reviews_selected_storage_size_and_captured_vm_at_both_widths` | runtime | split | `ui:areas/vm_devices/tests.rs::disk_creation_reviews_selected_storage_size_and_captured_vm`; `gpui:tests/wanted_events.rs::disk_creation_reviews_selected_storage_size_and_captured_vm_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `vm_device_lifecycle_tests.rs` | `attach_and_add_cd_review_exact_targets_without_deleting_existing_data` | runtime | ported | `ui:areas/vm_devices/tests.rs::attach_and_add_cd_review_exact_targets_without_deleting_existing_data` |  |
| `vm_device_lifecycle_tests.rs` | `detach_preserves_data_delete_reviews_consumers_and_ownership_changes_disable_it` | runtime | ported | `ui:areas/vm_devices/tests.rs::detach_preserves_data_delete_reviews_consumers_and_ownership_changes_disable_it` |  |
| `vm_device_lifecycle_tests.rs` | `nic_editor_reviews_generation_and_replacement_and_captures_original_identity` | runtime | ported | `ui:areas/vm_devices/tests.rs::nic_editor_reviews_generation_and_replacement_and_captures_original_identity` |  |
| `vm_device_lifecycle_tests.rs` | `invalid_resource_choices_and_user_values_cannot_reach_review` | pure | moved | `ui:areas/vm_devices/tests.rs::invalid_resource_choices_and_user_values_cannot_reach_review` |  |
| `vm_device_lifecycle_tests.rs` | `device_classes_are_read_using_canonical_inventory_keys` | pure | moved | `ui:areas/vm_devices/tests.rs::device_classes_are_read_using_canonical_inventory_keys` |  |
| `vm_device_lifecycle_tests.rs` | `reviewed_changes_cannot_follow_a_pool_switch_or_missing_live_observation` | runtime | ported | `ui:areas/vm_devices/tests.rs::reviewed_changes_cannot_follow_a_pool_switch_or_missing_live_observation` |  |
| `vm_device_lifecycle_tests.rs` | `shared_disk_consumers_are_described_and_deletion_stays_unavailable` | pure | moved | `ui:areas/vm_devices/tests.rs::shared_disk_consumers_are_described_and_deletion_stays_unavailable` |  |
| `vm_device_properties_tests.rs` | `priority_review_keeps_device_identity_at_both_widths_and_cannot_submit_synthetic_changes` | runtime | split | `ui:areas/vm_devices/tests.rs::priority_review_keeps_device_identity_and_cannot_submit_synthetic_changes`; `gpui:tests/wanted.rs::vm_devices` | the window half is the row of `gpui:tests/wanted.rs` for the scene `device-properties` ("the controls of the device properties"), matched to this test by its area, scene and subject, not by name |
| `vm_device_properties_tests.rs` | `nic_rate_review_has_correct_units_and_preserves_captured_qos` | runtime | ported | `ui:areas/vm_devices/tests.rs::nic_rate_review_has_correct_units_and_preserves_captured_qos` |  |
| `vm_device_properties_tests.rs` | `metadata_cbt_confirmation_and_invalid_values_require_explicit_valid_review` | pure | moved | `ui:areas/vm_devices/tests.rs::metadata_cbt_confirmation_and_invalid_values_require_explicit_valid_review` |  |
| `vm_devices.rs` | `absent_permission_and_empty_are_distinct` | pure | moved | `ui:areas/vm_devices/tests.rs::absent_permission_and_empty_are_distinct` |  |
| `vm_lifecycle_tests.rs` | `creation_wizard_reviews_typed_resources_and_keeps_its_source_at_both_widths` | runtime | split | `ui:areas/vm_lifecycle/tests.rs::creation_wizard_reviews_typed_resources_and_keeps_its_source`; `gpui:tests/wanted_events.rs::creation_wizard_reviews_typed_resources_and_keeps_its_source_at_both_widths` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `vm_lifecycle_tests.rs` | `cloning_distinguishes_fast_and_full_copy_and_exact_destination` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::cloning_distinguishes_fast_and_full_copy_and_exact_destination` |  |
| `vm_lifecycle_tests.rs` | `conversion_and_template_metadata_have_reviewed_captured_sources` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::conversion_opens_on_the_review_of_its_captured_source` | the conversion half. The metadata half is retired with the Rune kind `Metadata`: it is the General page of a template's Properties dialog (area `vm_properties`) |
| `vm_lifecycle_tests.rs` | `deletion_defaults_record_only_and_captures_exact_selected_data_and_snapshots` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::deletion_defaults_record_only_and_captures_exact_selected_data_and_snapshots` |  |
| `vm_lifecycle_tests.rs` | `invalid_resources_and_stale_or_changed_sources_cannot_submit` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::invalid_resources_and_stale_or_changed_sources_cannot_submit` |  |
| `vm_lifecycle_tests.rs` | `global_history_survives_deleted_source_and_is_reachable_offline_at_compact_width` | runtime | retired |  | retired with the "VM changes" panel: the Events tab lists these records, also offline. The 480 px position was layout |
| `vm_lifecycle_tests.rs` | `shared_history_retains_unresolved_records_and_bounds_terminal_records` | pure | moved | `ui:model_tests.rs::vm_operation_rows_merge_by_pool_and_id_and_keep_the_unresolved` | moved by the kernel with `merge_operation_rows` |
| `vm_lifecycle_tests.rs` | `installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_edited_or_deleted` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_deleted` | editing a template's metadata is the Properties dialog's now |
| `vm_lifecycle_tests.rs` | `copied_bios_choice_captures_host_and_affinity_in_the_creation_review` | runtime | ported | `ui:areas/vm_lifecycle/tests.rs::copied_bios_choice_captures_host_and_affinity_in_the_creation_review` |  |
| `vm_migration_tests.rs` | `compact_and_wide_review_capture_exact_targets_and_remain_preview_only` | runtime | split | `ui:areas/migration/tests.rs::rune_review_captures_exact_targets_and_remains_preview_only`; `gpui:tests/wanted.rs::migration`; `gpui:tests/wanted_events.rs::compact_and_wide_review_capture_exact_targets_and_remain_preview_only` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `vm_migration_tests.rs` | `destination_and_mapping_controls_produce_reviewed_typed_plan` | runtime | split | `ui:areas/migration/tests.rs::rune_destination_and_mapping_controls_produce_reviewed_typed_plan`; `gpui:tests/wanted_events.rs::destination_and_mapping_controls_produce_reviewed_typed_plan` |  |
| `vm_migration_tests.rs` | `stale_or_missing_vm_and_changed_devices_prevent_apply` | pure | moved | `ui:areas/migration/tests.rs::rune_stale_or_missing_vm_and_changed_devices_prevent_apply` |  |
| `vm_migration_tests.rs` | `halted_storage_move_requires_transfer_network_and_fresh_home_host` | pure | moved | `ui:areas/migration/tests.rs::rune_halted_storage_move_requires_transfer_network_and_fresh_home_host` |  |
| `vm_properties.rs` | `home_host_picker_reviews_original_identity_and_keeps_synthetic_apply_disabled` | runtime | ported | `ui:areas/vm_properties/tests.rs::home_host_picker_reviews_original_identity_and_keeps_synthetic_apply_disabled` |  |
| `vm_properties.rs` | `properties_review_is_reachable_and_demo_apply_is_disabled` | runtime | split | `ui:areas/vm_properties/tests.rs::properties_review_is_reachable_and_demo_apply_is_disabled`; `gpui:tests/wanted.rs::vm_properties` | the window half is the row of `gpui:tests/wanted.rs` for the scene `properties` ("the page list, the fields and the footer of the properties"), matched to this test by its area, scene and subject, not by name |
| `vm_properties.rs` | `memory_input_is_exact_and_bounded` | pure | moved | `ui:areas/vm_properties/tests.rs::memory_input_is_exact_and_bounded` |  |
| `vm_properties.rs` | `property_draft_keeps_the_original_target_and_values` | pure | moved | `ui:areas/vm_properties/tests.rs::property_draft_keeps_the_original_target_and_values` |  |
| `vm_properties.rs` | `running_guest_draft_uses_live_cpu_and_dynamic_memory_commands` | pure | moved | `ui:areas/vm_properties/tests.rs::running_guest_draft_uses_live_cpu_and_dynamic_memory_commands` |  |
| `vm_properties.rs` | `secure_boot_review_preserves_vm_identity_and_does_not_claim_enforcement` | runtime | ported | `ui:areas/vm_properties/tests.rs::secure_boot_review_preserves_vm_identity_and_does_not_claim_enforcement` |  |
| `vm_properties.rs` | `empty_custom_template_has_explicit_firmware_review_and_builtin_is_read_only` | runtime | ported | `ui:areas/vm_properties/tests.rs::empty_custom_template_has_explicit_firmware_review_and_builtin_is_read_only` |  |
| `vm_recovery_view_tests.rs` | `compact_and_wide_boot_review_keeps_target_and_visible_controls` | runtime | split | `ui:areas/power/tests.rs::rune_boot_review_keeps_target_and_refuses_in_demo_and_on_a_changed_connection`; `gpui:tests/areas.rs::the_controls_of_vm_recovery_are_inside_the_window_under_the_experimental_notice` |  |
| `vm_recovery_view_tests.rs` | `force_review_captures_exact_tasks_and_refuses_changed_tasks_or_admin_blocks` | runtime | ported | `ui:areas/power/tests.rs::rune_force_review_captures_exact_tasks_and_refuses_changed_tasks_or_admin_blocks` |  |
| `vm_recovery_view_tests.rs` | `a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it` | runtime | split | `ui:areas/power/tests.rs::rune_a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it`; `gpui:tests/wanted_events.rs::a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it` |  |
| `vm_recovery_view_tests.rs` | `unsupported_guest_and_missing_task_identity_do_not_offer_recovery_dispatch` | runtime | ported | `ui:areas/power/tests.rs::rune_unsupported_guest_and_missing_task_identity_do_not_offer_recovery_dispatch` |  |
| `vm_recovery_view_tests.rs` | `cancellation_review_is_bound_to_owned_operation_and_does_not_force` | runtime | ported | `ui:areas/power/tests.rs::rune_cancellation_review_is_bound_to_owned_operation_and_does_not_force` |  |
| `vm_snapshots.rs` | `snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes` | runtime | split | `ui:areas/snapshots/tests.rs::rune_snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes`; `gpui:tests/wanted.rs::snapshots`; `gpui:tests/wanted_events.rs::snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes` | the window half runs at the sizes of its row in `gpui:tests/wanted.rs`; 800x600 stands for the Rune compact size, which no longer exists |
| `vm_snapshots.rs` | `restore_requires_explicit_shutdown_and_start_choices_and_captures_identity` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_restore_requires_explicit_shutdown_and_start_choices_and_captures_identity` |  |
| `vm_snapshots.rs` | `snapshot_editor_keeps_reviewed_metadata_when_selection_changes` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_snapshot_editor_keeps_reviewed_metadata_when_selection_changes` |  |
| `vm_snapshots.rs` | `checkpoint_restore_offers_resume_and_retains_saved_memory` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_checkpoint_restore_offers_resume_and_retains_saved_memory` |  |
| `vm_snapshots.rs` | `cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo` | runtime | split | `ui:areas/snapshots/tests.rs::rune_cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo`; `gpui:tests/wanted_events.rs::cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo` |  |
| `vm_snapshots.rs` | `deletion_reviews_only_captured_owned_disks_and_lists_retained_disks` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_deletion_reviews_only_captured_owned_disks_and_lists_retained_disks` |  |
| `vm_snapshots.rs` | `deletion_does_not_offer_cleanup_without_complete_disk_inventory` | runtime | ported | `ui:areas/snapshots/tests.rs::rune_deletion_does_not_offer_cleanup_without_complete_disk_inventory` |  |
| `vm_snapshots.rs` | `explicit_checkpoint_block_overrides_advertised_memory_mode` | pure | moved | `ui:areas/snapshots/tests.rs::rune_explicit_checkpoint_block_overrides_advertised_memory_mode` |  |
| `vm_snapshots.rs` | `snapshot_review_requires_the_same_pool_parent_state_and_exact_capability` | pure | moved | `ui:areas/snapshots/tests.rs::rune_snapshot_review_requires_the_same_pool_parent_state_and_exact_capability` |  |
| `vm_snapshots.rs` | `snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes` | runtime | split | `ui:areas/snapshots/tests.rs::rune_snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes`; `gpui:tests/wanted_events.rs::snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes` |  |

## Mirror fixes

The plan meant this table for fixes made to the frozen Rune crate and mirrored into the port. There are
none: between the extraction commit `7446081` and the commit that removed the Rune crate (`71d9119`) no
commit changed it. The rows record the other direction, as the engineers reported it: behaviour of the
Rune client that the port judged a defect and did not copy. The Rune client had each of them until it
was removed, and the released versions up to 0.19.0 have them. "hazard n" is the number the defect has
in the engineers' packets. Each correction is implemented and, where a test is named in the area's
header, fixture-verified. Those in the supported areas were on the paths of the headless lab runs of
2026-11-19 only where the [report](reports/2026-11-19-gpui-live-headless.md) says so; none was aimed at
by a live scenario, and none was aimed at in the driven session in a virtual machine.

| area | Rune behaviour | what the port does | Rune commit |
| --- | --- | --- | --- |
| `events` | Acknowledge of a resource operation called `control_resource` on the scope connection, while its button was enabled by the connection picked for the control (hazard 47) | The call goes to the picked control connection as well | none (not fixed in Rune) |
| `events` | The unknown-outcome dialog closed whatever the answer to its acknowledgement was | The acknowledgement is checked again, a refusal stays in the dialog, and the confirmation is disabled in the demo | none |
| `events` | The reviews of a task cancellation and of a message dismissal did not assess the session's permissions | Both assess them; a known denial disables the confirmation with its reason | none |
| `events` | In a synthetic pool the confirmation was disabled under the caption of a stale review | It is disabled with the demo reason | none |
| `storage_maintenance`, `pool` | The submit handlers of repair, read caching and pool policy sent a change the session was known to be denied | They refuse a known permission denial | none |
| `pool_security` | The password or certificate was consumed before `submit_resource`; after a failed submit the review stayed staged, and a second Confirm of a directory operation would have sent an empty password (hazard 41) | A submit that fails after the secret was consumed returns to the draft | none |
| `vm_devices` | Applying a device change did not check again that the review was current and the device class available (hazard 1) | The apply checks staleness and availability again | none |
| `snapshots` | Submit trusted the state of the confirm button | Submit checks the button's whole condition again | none |
| `migration` | A refused submission or a known permission denial was written to the global error line, which the migration dialog never showed | It is shown in the wizard's own error line | none |
| `migration` | The batch move opened for a selection with a refused VM and for a pool that is not writable | The command is disabled with the reason (outside the demo for the pool) | none |
| `cross_pool` | For a suspended or paused VM, and in a synthetic pool, the button did nothing and said nothing | The command is disabled with a reason | none |
| `management_network` | "Remove management bond…" was shown when the management interface could not be captured, and then did nothing | The command is hidden there | none |
| `connections` | Reconnect As relied on its button to keep out a blank account | The handler refuses a blank account as well | none |
| kernel | The handles of VM operations were dropped when the shown pool changed, so a record of a pool the tree no longer showed stopped changing | Handles belong to their session; the one list holds the records of every connected session and keeps following them | none |
| kernel | Resource operations followed the scope session only, and their finished handles were never dropped, so the limit an area puts on unresolved resource operations counted every operation of the run | A resource operation belongs to the session that shows its pool; every connected session's history is merged; a finished handle is dropped once its record is merged, and a session's handles go with its connection | none |
| `connections`, `power`, `snapshots`, `performance`, `vm_tools`, `transfers`, `network`, `bulk_edit`, `bulk_export`, `host_services`, `pool`, `pool_security`, `cross_pool`, `vm_lifecycle` | Buttons were disabled, or hidden, without saying why | A disabled command or button carries a reason, shown as tooltip and in the tree dump. The texts are new, since Rune had none; each area lists its own in its header | none |

## Deviations

What differs from the plan ([GPUI-MIGRATION.md](GPUI-MIGRATION.md)) or from what the Rune client shows and
lets the user do. "where" is the plan's gate, the architecture, or the area. Rows marked "decision n" are
the decisions taken before the areas were ported; the others are from the engineers' results and the
headers of the areas. A row describes what is implemented; no person has seen any of it in a window, a
row says so where the driven session in a virtual machine of 2026-11-19 observed it
([report](reports/2026-11-19-gpui-vm-session.md)), and against a host only what the headless lab runs of
2026-11-19 passed through in the supported areas is shown
([first report](reports/2026-11-19-gpui-live-headless.md), [second](reports/2026-11-19-gpui-live-paths.md)). Differences that are corrections of a Rune defect
are under Mirror fixes.

| where | differs from | what differs | reason |
| --- | --- | --- | --- |
| architecture | plan | `xenadmin-ui` does not only hold the model: it describes every surface as plain data (`Surface`, `Block`, with handlers over the model), and the GPUI crate (`xenadmin-desktop` since the cutover) has one generic renderer for all of them. Hand-written GPUI code exists only for the shell, the tree, the console canvas, the charts and window management. The plan had each view ported as GPUI code by its package | An area port touches one crate, needs no knowledge of the Kit, and its interaction tests run without a window |
| architecture | plan | A Rune test that drove a `rune::Runtime` is in most cases a model test through the `Harness`, which presses the keys of the described surface; a headless window test exists only where a window is the subject. The plan rewrote every such test as a `#[gpui_kit::test]` | Follows from the described surfaces. What it costs is in "Tests by disposition": positions at a window size are asserted for few surfaces |
| architecture | plan | The port was done by area (29 directories under `ui:areas/`), not by the plan's packages P1 to P5 and W1 to W6; the ledger's Commands and Scenes tables have the registry's columns and the area, not "Rune source", "model status", "GPUI status" and "package"; the ledger was written once from the repository, not row by row with each merge | One generic renderer left nothing to port per package on the GPUI side; the columns that remain are the ones a script can check |
| architecture (decision 4) | plan | `xenadmin-ui` depends on `serde_json`, `zeroize` and `image` besides the plan's three crates. It depends on no UI toolkit. The model asks for a timed tick (`request_tick`) | Saved searches and reports, secret drafts, and console and snapshot pictures are model work |
| architecture (decision 9) | plan | The Properties dialogs are the kernel's (`server.properties`, `pool.properties`, `vm.properties`, `template.properties`, `storage.properties`, `network.properties`, `disk.properties`); an area hands in pages and registers no Properties command or surface. Network and disk Properties have no place in the menu bar; a Properties command without a page is disabled with a kernel text | One dialog per object, with the pages of several areas in the plan's order |
| architecture | plan | The scene tool knows scenes Rune did not have: the renderer's eight `gallery-*` scenes and the kernel's `host-properties` | To render what no Rune scene shows |
| G0 (decision 0006) | plan | Gate G0 did not pass as written; the migration continued on the operator's direction of 2026-11-17 with the limits below accepted. Route A (a second `wl_keyboard`) carries physical key codes; route B is not vendored; route C is not adopted | Decision 0006 |
| G0 (decision 0006) | Rune | Not shown on a real seat: GPUI delivered no pointer motion or button on the Hyprland probe | Accepted limit; open. In a virtual machine on sway 1.12 the client was driven by injected pointer presses and a guest recorded the button presses on its console (2026-11-19); pointer motion inside a guest was not examined |
| G0 (decision 0006) | Rune | `CursorStyle` has no hidden value: the local cursor stays visible over the console, and the guest cursor is not drawn (Rune hid the local one and drew the guest's). "Hold right button" is gone: a right-button drag is native | Accepted limit |
| G0 (decision 0006) | plan | Screencopy never completed: the pixels of fit and actual size were not read back from a real window | Accepted limit; open. Fit and actual size were looked at in pictures of a virtual machine's display (2026-11-19), not compared pixel by pixel |
| G0 (decision 0006) | Rune | Clipboard text was not offered because the probe window never became active; clipboard without focus stays unsupported | Accepted limit; open. With the window focused, text was copied out and pasted in on sway 1.12 in a virtual machine (2026-11-19); without focus it was not tried |
| G0 (decision 0006) | plan | Fullscreen was reported once and not again; leaving it was not observed. No guest recorded a key, a second host layout was not applied, Ctrl+Alt+F2 was not sent | Accepted limit; open for a real seat. In a virtual machine on sway 1.12 with injected keys (2026-11-19): fullscreen entered and left twice; a lab guest recorded every key, with the same codes under a German layout; Ctrl+Alt+F1 sent through the console's button arrived, no such chord was pressed at the keyboard |
| `console` | plan | Route A is in the client: the listener made from the first real window, its timer, the routing to the canvas that has the keyboard, and stopping GPUI's keystrokes while it is bound have no test, because the headless platform has no seat. Tests reach the canvas through the keystroke fallback. The listener ran once with a console, in a virtual machine on sway 1.12 with injected keys (2026-11-19, [report](reports/2026-11-19-gpui-vm-session.md)). The listener sees no repeated key, so the client repeats a held key itself, at the compositor's delay and rate (`Repeater` in `gpui:src/console/keys.rs`, with unit tests); the packages built before that fix send one press for a held key | No seat in a headless test |
| `console` | Rune | Without the Wayland listener (X11, which stays experimental) a key is sent with an evdev code from a US table or as a keysym: Num Lock and the right-hand modifiers cannot be reported. Without QEMU extended keys the keysym sent is the US layout's | The fallback has only GPUI's keystroke |
| all areas (decision 1) | Rune | A surface is experimental exactly when the Rune code showed the notice for it. Two corrections: a child panel of an experimental area that Rune left without the notice has it; nothing `docs/FEATURES.md` lists as supported gains one | One rule for the notice |
| `power` (decision 2) | plan | Pause and Unpause stay (VM menu, Start/Shut Down submenu) | The client has them (`PowerAction::ALL`) |
| `vm_devices` (decision 3) | plan | A template's Storage and Networking tabs show information only; every device command is hidden for a template | The client edits no template devices |
| kernel, `host_admin` (decision 5) | plan | Menu items of the plan that the client never implemented are left out, not stubbed: Import Server List, Export Server List, New Pool, View Log Files. Power On exists as `server.power-on` (experimental) | The plan says to leave them out; the Rune client has the reviewed power-on |
| `events` (decision 6) | Rune | The Notifications navigation view replaces the Activity dialog: Alerts (the Rune Messages) and Events (Tasks and History behind one choice) are its tabs. The Activity button, the dialog's Close and its scroll container are gone | A surface that only shows state is a tab |
| `power` (decision 7) | plan | Start on Server, Resume on Server and the task-aware force review keep the status the Rune code gives them | Decision 1 |
| `search` (decision 8) | Rune | The import of saved searches (`search-import-dialog`) and the inventory report (`inventory-report`) carry the experimental notice, which Rune did not show. Export Saved Searches opens no surface and so shows no notice | `docs/FEATURES.md` lists search files and reports outside the supported core |
| all areas (decision 10) | Rune | The one sweep of decision 10 was made: review and status texts that named Rune places ("Activity", "Pool administration", "pool history", "Open Connect / edit") now name the Events tab of the Notifications view, "this dialog" or the menu command ("Connect… in the Server menu"), and the tests that pinned the old words pin the new ones. Kept on purpose: the suggested file name of the exported list, `xenadmin-activity.json` | The places the old words named no longer exist; the file name is what earlier exports are called |
| `pool_security`, `connections` (decision 11) | Rune | Reconnect As cannot close the pool security editor; that area drops its own editor on a session change and when `pool.reconnect-as` is invoked from its surface | An area does not reach into another's state |
| `storage`, `storage_maintenance`, `network` (decision 12) | plan | `storage` owns Detach, Reattach, Forget, Destroy, Trim and the two attachment commands; `storage_maintenance` owns Repair; `network` owns Set as Default and Rescan, since their code was in the generic editor of `resource_admin.rs` | The commands went where their Rune code was |
| `vm_properties` (decision 13) | plan | The Properties dialog of a VM has a last page "Review Changes" outside the plan's page table: one draft spans the six pages and is reviewed and submitted once. All seven pages are experimental. A refused submit is shown on the page | `vm_properties.rs` reviewed and submitted one change set, and the port keeps that |
| `vm_properties` | plan | Changed-block tracking is not on Advanced. `properties-running` opens on CPU and Memory, `secure-boot` on Advanced, `home-host` on the review. New command `vm.memory-settings` on the Memory tab's row opens Properties at CPU and Memory | The client has that setting per virtual disk only (core gaps report, 12) |
| `pool` | plan | Pool Properties has a page "Advanced" for migration compression, outside the plan's page table. The plan put the setting on Network Options, which is the network area's page for IGMP snooping; of two pages with one id the kernel keeps the first | It is where the reference client has the setting |
| `network` | plan | The page General (rename) is provided for pool, server, storage and network, not for a network alone. The two Rune MTU changes are one field on Network Settings | The Rune rename served all four |
| `bulk_edit` | plan | Bulk properties is one page of the kernel's dialog for several VMs or several templates, with draft and review on that page; a selection it cannot edit (over 32 objects, several pools) is refused on the page. The Rune picker, a button and fourteen more, is one drop-down | The plan's "Several VMs: Bulk properties" |
| kernel | Rune | The connection manager dialog is the Home tab; a session whose inventory is stale (failed, reconnecting) stays a pool node with its status word instead of a bare connection node | A surface that only shows state is a tab |
| kernel | plan | View > Hidden Objects is left out. The other four View toggles exist and are not persisted | The inventory has no such flag; the preferences file format may not change (core gaps report, 11) |
| kernel | Rune | The details pane is gone, and with it the compact swap between table and details (`show-details`, `back-to-inventory`), the page header's buttons and the sidebar header with its repository link. Their successors are the tabs, the commands and Help > Project website | The plan's shell |
| kernel | Rune | The minimum window is 800x600 and the navigation pane gives way to a toggle in a narrow window (plan D3); the Rune compact size 513x652 cannot occur in the main window. The window tests of the areas use 800x600; the detached console window is tested at 513x652 and 480x640 | Plan D3 |
| kernel | Rune | The list of VM operations is one list for every connected session, with a limit of 64 unresolved operations per session. Resource operations belong to their session as well: the session that shows the record's pool, else the scope session; the rows merge every connected session's history; a session's handles are dropped with its connection (also by Reconnect As) after a last merge of their records; and a finished resource handle is dropped once its record is merged, so the limits of the areas (64; 128 in `vm_devices`, 32 in `performance`) count operations that are not finished, where Rune counted every operation of the run | Operation handles belong to their session |
| renderer | Rune | In a table with multi-selection, click, Ctrl+click, Shift+click, the arrow keys, Shift+Up and Shift+Down and select-all (the secondary modifier with A) work; Escape clears the selection | As the Rune table, on the Kit's table |
| renderer | Rune | Enter presses the focused button; with the focus elsewhere in a dialog it does nothing (no default action). The Kit never closes a dialog itself: Escape runs the surface's close handler. Dialogs open and close without animation | So that no widget is rebuilt for a closed surface that held a secret |
| renderer | Rune | The reason of a disabled radio option, segmented option or option of an open drop-down is its accessible description and a tooltip. A popup menu item cannot carry a description in Kit 0.9.1, so the reason of a disabled item of the menu bar, a context menu or a grid's row menu is part of the item's name; the tree dump has every reason. Semibold titles were drawn at regular weight on the build machine. None of this has been heard through a screen reader | The Kit exposes no description for a menu item; fonts of the machine |
| renderer | Rune | The Menu key and Shift+F10 open the menu of a grid row at the row, for a row that stands for an object of the tree and for one that does not | The Kit's table has no keyboard menu of its own |
| shell | Rune | A file prompt that fails is followed by the shell's own typed-path dialog; the request hears of the failure only when that dialog is closed without a path. Rune messages such as "Cannot open save dialog: ..." have no successor where an area sets no failure handler | A portal may be missing |
| `connections` | Rune | Card buttons that Rune showed by state are commands, hidden when the state does not apply. "View pool" is the activation of the row. `pool.disconnect` and `pool.reconnect-as` duplicate the server commands. The Home tab shows the shared error line | A command has one menu path |
| `general` | Rune | The information dialog is the General tab, which follows the selection and is always current: the panel that kept its target, "Refresh from client", the "Pool information" button and Close are gone. The Operations section of a VM takes only "Explain outcome…"; the other controls of a record are the power area's | A surface that only shows state is a tab |
| `search` | Rune | One query sheet, preset by the tree selection (pool, server, type of the Objects view, folder, tag, saved search, several objects, nothing). Saved searches are rows of their navigation view; the "Saved searches" toggle and the per-row Remove are the view, `search.save` and `search.remove`. The grid's row menu is the kernel's context menu. A new query keeps the selected rows that are still shown, as the Rune table did (`gpui:tests/wanted_events.rs::the_selected_row_a_filter_leaves_is_still_the_target`) | The plan's Search tab |
| `search` | Rune | A disk result selects the disk, not its repository. Synthetic and stale sources cannot be opened although the tree can select demo objects. A scope of several objects cannot be saved or reported | A disk is an object of the tree now; core gaps report, 5 |
| `host_admin` | Rune | The chooser dialog is gone: ten commands replace it, each opening its review. One item toggles Enter and Exit Maintenance Mode. Multipathing, Log Destination and Power On are pages of the server's Properties. A command is disabled, with the core's explanation, exactly when no input could make its review pass. No Back from an edit to the chooser | A dialog of buttons that open dialogs is what the plan rejects (R5) |
| `power` | Rune | The recovery panel's overview is dissolved: each review opens from a command or from a control under an operation record; there is no Back and no Refresh. The confirmation is disabled when stale or denied | No hub |
| `power` | plan | All seven toolbar slots are used. Force Shut Down and Force Reboot open the task-aware review when the ordinary action is refused and tasks are known, and always when a task is a migration. Commands read the pool of their target, not the scope session | The plan's toolbar; the Rune rule for forced actions (core gaps report, 10) |
| `events` | Rune | Commands act on the row selected in the tab. Every session's records are listed, not only those of pools once selected. The pool filter matches a record that applies to the pool. Two filters, both emptied when switching between Alerts and Events; their clear buttons are the input's own. The explanation of an outcome no longer returns the focus to its opener | The tab replaces three lists |
| `events` | Rune | The History list is the Rune Activity list for every connected session, without the limit of twelve records that the Rune pool history had. A batch shows as its parent, with its members as the parent's detail lines; the checkbox `activity-show-children` shows the members as rows of their own (their controls are refused with the batch reason). `resource-operation-notice` is replaced by the status bar's counters. The record of a transfer shows its stream and the objects it retained, and a finished cross-pool transfer offers its destination VM (`cross-destination-<id>`) | History is also the Activity list, which had no limit (header of `ui:areas/events/mod.rs`); a batch of 32 would otherwise bury the list |
| `console` | Rune | The console's controls are commands of the tab's local row, and buttons in a detached window. The console opens when its tab is shown and on a scope change while the tab stays active, releases input on leaving the tab and closes when another object is selected; `console.close` is new. The typing review stays an inline group. The canvas takes the keyboard by click only, not by Tab | A Tab stop on the canvas would trap the keyboard |
| `console` | Rune | The console windows manager dialog is gone: the Window menu lists up to eight windows and has Close All Consoles. There is no paste into the canvas (while it has the keyboard every key is the guest's); clipboard text is sent with the commands | The Window menu of the plan |
| `snapshots` | plan | Edit and Clone (New VM, New Template) stay dialogs beside the plan's Take, Revert and Delete | Rune had them |
| `snapshots` | Rune | "Inspect snapshot" is a section of the tab for the one selected snapshot; it reads the inventory as it is now, where the Rune dialog kept the inventory of the moment it was opened. "Load saved preview" stays a button of the section. Back to inventory and the Activity button are gone | A surface that only shows state is a tab |
| `performance` | plan | The source picker and the presets stay inside the tab, as in Rune, not dialogs; there is no command for the range (a segmented choice). The monitoring catalog is one table, not pages of buttons. Without a current connection the tab shows a waiting line. The stream exists only while the tab is shown | Rune's three bodies of one view |
| `migration` | plan | The wizard has the three Rune stages, not four steps; the batch has two. There is no separate id for the batch: `vm.migrate` and `vm.move` open it for several VMs. Lists of buttons became choices and drop-downs. Both surfaces close on a scope or session change. After an accepted batch the Events tab is shown where Rune opened the pool history | The Rune stages |
| `vm_lifecycle` | plan | The New VM wizard's steps are Name, Installation Media, CPU and Memory, Storage, Networking, Cloud-init, Review: no Template step and no Home Server step. `vm.new` and `templates.new` are disabled until one template is the target | Rune has no template picker; the BIOS identity depends on the firmware chosen under CPU and Memory |
| `vm_lifecycle` | Rune | "Edit template name and description" is the General page of the template's Properties. The "VM changes" history panel is the Events tab. Both surfaces close on a scope or session change | One place for each |
| `vm_devices` | Rune | The Devices page is the Storage and Networking tabs of a VM; the Virtual disks workspace is the Storage tab of a repository with a search field, without paging. The disk dialog has one mode per command (Resize, Copy, Delete Owned Disk). There is no Move and no Add on a repository. The General page of a disk exists only when a VM attaches it | Tabs for state, commands for actions; the Rune modules had no Move or Add; core gaps report, 7 |
| `vm_tools` | plan | `vm.install-guest-tools` is disabled with the dialog's empty-state text when the VM has no CD drive or the pool no tools media, and is also on the Storage tab's row. `vm.connect-external` is on the VM context menu and the Console tab's row, with no place in the menu bar. Saving tool declarations is refused in the demo. A failure to open "Installation guide" is no longer reported in the dialog | The open-URL intent has no failure reply |
| `storage` | Rune | The storage hub is dissolved: wizard New Storage (Type, Name, Location, Review), dialogs for an attachment, a change and the secret cleanup, and the Storage tab (repositories of a pool; one row per attachment on a server). Detach, Reattach and Remove act on one host attachment: the scope server's, else the repository's only one. New Storage is on the Storage tab's row of a pool and a server. No Properties page | No hub; these modules never edit an existing repository's name |
| `storage_maintenance` | plan | Read Caching is a page of the repository's Properties with its review on the page. `storage.multipath-workflow` has no menu place (the Multipathing page of a server runs it), stays enabled for any known server and shows a refusal inside its dialog, because the dialog also holds the history with Restore. The server workflow opens on its review, without Back | The history must stay reachable |
| `transfers` | plan | Import asks for the type first (the file filter depends on it), then the source. After "Begin transfer" the wizard stays open and shows only the transfer it started; the Rune list of all transfers is the Events tab. A scope change closes only an unsubmitted export; nothing closes a running conversion | The type decides the file filter |
| `network` | Rune | The host networking hub is dissolved: the Networking tab of a pool, server or network, the NICs tab of a server, a wizard for a new network and review dialogs. Interface commands act on the row selected in the interface grid; bond mode and hash are one command each with a choice. Set as Default acts on the selected repository (the Rune chooser is gone). The scene `pool-administration` shows the pool's Networking tab | No hub |
| `network_services` | plan | One command and one wizard per Rune fabric module (one host, the pool), with GRE, VXLAN and SR-IOV as the wizard's Type step: eight commands on the row instead of twelve. The Name step is read-only (these workflows never created the network). The Type step of New Network lists the two fabric wizards under their command labels and hands over to the one chosen; a refusal keeps the wizard open with the reason (no scene shows that step). The Networking tab of a network shows the NBD service state and the tunnel and SR-IOV coverage of the network | The Rune modules |
| `management_network` | Rune | A review handed to the shared `network-change` dialog has Back: the form is kept and restored (`network::review_back`). A bond removal has no form and so no Back. The commands are on the tabs' rows and the server's context menu, not in the row menu of the interface grid: a server as target means its management interface whatever row is selected, so the menu of another row would mislead. The caption of an unfinished IPv6 handover is shown on the NICs and Networking tabs as well as in the area's dialog | Rune could go back; the first port could not, and the gap was closed |
| `pool` | Rune | The pool hub is dissolved: the HA tab, thirteen commands, the Configure HA wizard, review dialogs, the pages Advanced and Security, the dissolve dialog, the join wizard and the password dialog. Every surface carries the experimental notice, including join and password. The limit of thirty VM buttons is gone (the grid lists every VM), and so is the jump to the pool history after a submission | No hub; join and password are not in `docs/FEATURES.md` |
| `pool` | plan | Join keeps the Rune direction: the selected standalone server joins another pool (`server.join-pool`, "Add to Pool", at Pool > Add Server). Start order and delays stay with the restart priority in the VM dialog of the HA tab, not on VM Properties. `pool.designate-coordinator` has no row in the plan and sits in the Server menu | The Rune client |
| `pool_security` | Rune | The "Pool security" dialog is the Users tab of a pool; each change is a dialog with the Rune review. A synthetic pool is never queried. After a submit the dialog closes and the overview is read again when the operation ends. Command labels are new. The certificate commands are in the Server menu only, not in a server's context menu | The plan's Users tab |
| `host_services` | Rune | The panel's mode row is three commands (`server.backup`, `server.restore-from-backup`, `tools.server-status-report`), and only the status report queries the server. Both reviews gained Back, which drops the chosen file. Dialog titles are the Rune mode names ("Stage recovery", not the menu's "Restore From Backup"). The archive commands are disabled without a connection or a current observation, where Rune opened the panel and showed the error | The action stages a backup for an installer-assisted recovery and restores nothing that runs |
| `schedules_alarms` | plan | The alarms hub is gone: thresholds are the page Alerts of a VM, server or repository, pool email the page Email Options. "Refresh service" has no row in the plan and is `tools.alarm-service-refresh` (Tools menu, pool context, a button on the Alerts page). The actions on a schedule are buttons of the schedules dialog, not commands | Properties pages for settings |
| `bulk_edit`, `bulk_export` | plan | Batch deletion and batch export have ids of their own (`vm.delete-several`, `templates.delete-several`, `vm.export-selection`, `templates.export-selection`) at the menu place of the single commands, each hidden when the other applies. After a submitted batch the Events tab is shown | A command id belongs to one area |
| `organization` | plan | Folder and tag editing are commands with one dialog, not part of the Custom Fields page. The Rune filter and check list are gone: the selection of the Search tab is the target, with the limit of 256 kept. Custom fields are not a heading of the Organization view. A heading shows only when it has rows | There is no object kind for a custom field |
| `cross_pool` | Rune | One command, `vm.move-to-pool`: one VM opens the single wizard (move or copy), several the batch; a batch of exactly one VM cannot be reached any more. The Rune transfer history is the Events tab. Button lists became choices, acknowledgements checkboxes. The wizard closes at submission and shows its record on the Events tab, which is where a finished transfer offers its destination VM. A submitted live move closes the guest's console, in the tab and in detached windows | One command for the selection |
| kernel | plan | Context menus nest. The power commands are under "Start/Shut Down", the organization commands under "Organize" and a server's management-interface commands under "Management Interface"; the top level has 19 entries for a VM, 20 for a server, 16 for a pool, 13 for a repository, 11 for a network, 9 for several VMs and 7 for a template. The keyboard reaches the submenus; a menu longer than the window still scrolls | A flat menu of every command of an object was longer than a small window |
| kernel | plan | A command on a tab's row of local actions names the kinds of scope it is offered for (`CommandSpec::local_scopes`; 108 placements, each pinned by a test per tab and scope). So New Storage is on the Storage row of a pool and a server and not of a VM, and the disk commands are on the Storage row of a repository and a VM and not of a pool or a server. On a repository's Storage tab the repository commands come first and the disk commands after them (orders 82 to 88) | The menu and the toolbar share a command's eligibility, which cannot hide it on one row only |
| `vm_lifecycle`, renderer | Rune | The four cloud-init fields for user data and network configuration are private text areas (`private_textarea`): the text an edit replaces is zeroized, in the model and in the copy the renderer hands to it, and the widget's buffer is emptied when the field is no longer drawn. The review omits the payload | The text may hold credentials (plan R11) |
| G2, criterion 5 | plan | The review of the criterion is the window test `gpui:tests/action_review.rs`: every control that is outside the menu bar, a menu, the toolbar, the navigation pane, the tab strip, a tab's row of local actions, the status bar and the dialogs is in its table, as a control of the view or as an action that stays with its reason. One action moved: "Export selection…" of the Snapshots tab was a button of the body and is the command `snapshot.export` on the tab's row (Snapshots.60, scope VM). The actions that stay are the rows below | The review covers the states the registered scenes show, the description of every tab of their scopes, the source picker, a VM with operation records (each record also alone, since a tab lists only its newest) and a finished transfer selected in the history of the Events tab; a control that only another state shows is not covered. A review of the test on 2026-11-20 found that its records hid four action rows and four rows of the view from it; they were added, and the reasons below that read as a rule of the plan were reworded as the choices they are |
| tab body ending in a short table | plan | The part above a short table (four rows or fewer) scrolls without the Kit's scroll area and so without a scrollbar, where the part above a long table keeps both | The Kit's scroll area keeps the wheel to itself while it has anything to scroll; over a part that has given way for the table the wheel would then never move the body, and the sections after the table could not be reached from there. Since 2026-11-20 the part above and a short table stand in a head that is laid out without regard to what follows, so that a selected row does not move the table (it moved it by more than a row where the row of local actions wraps); `gpui:tests/tab_layout.rs` holds this at 800x600, where the test platform's font wraps the Snapshots row |
| `console` (G2, criterion 5) | plan | Seven buttons of the detached console window are actions outside a row of commands: Ctrl+Alt+Delete, Alt+Tab, Ctrl+Alt+F1, Reconnect, Save screenshot…, Send clipboard to VM, Type clipboard as keys | The window has no menu bar, toolbar or row of commands, and a command acts on the selection of the main window, while these act on the console of their own window. In the main window the same controls are commands of the Console tab's row |
| `console` (G2, criterion 5) | plan | "Type it" of the typing review (`confirm-console-typing`) is a button of the Console tab's body | Not moved, by choice: it confirms an action, and the review it confirms is an inline group of the tab (opened by the command `console.type-clipboard`) where the project's rule would make it a dialog; the review was left as the old front end had it |
| `power`, `general`, `snapshots`, `vm_devices` (G2, criterion 5) | plan | The controls under an operation record of a VM are buttons of the tab's body. On the General tab: Stop remaining steps or cancel (`operation-cancel-*`), Recheck outcome, Review unknown outcome, Review cancellation request…, Explain outcome…. On the Snapshots tab, under a snapshot workflow: Stop remaining steps (`snapshot-operation-cancel-*`) and Recheck snapshot outcome (`snapshot-operation-recheck-*`). On a VM's Storage and Networking tabs, under a device change: Recheck outcome (`device-recheck-*`, one row of the review for each tab) | Not moved, by choice: their target is one operation record, shown as a line of the tab. The plan lets a command of a tab's row act on the selection of a grid; the records are lines without a selection and were not made a grid in this port |
| `performance` (G2, criterion 5) | plan | "Save current graph" and a preset's "Remove" are buttons of the presets body of the Performance tab | Not moved, by choice: they act on the preset named in the field beside them or on the preset of their row, inside the body the command `performance.presets` opens; they write only the client's preferences |
| `snapshots` (G2, criterion 5) | plan | "Load saved preview" is a button of the section "Snapshot information": it asks the host for the saved picture (a read) | Not moved, by choice: it acts on the selected snapshot, as the commands of the tab's row do, and could be one of them; it was left a button of the section it fills, where it reports a refusal |
| `pool`, `connections` (G2, criterion 5) | plan | A double click or Enter on a row acts without a drawn control in two grids: on a VM of the HA tab it invokes the command `vm.ha.restart-priority`; on a connection of the Home tab it shows the pool of a connected one and opens the settings dialog of one that is not connected | The first is the command of the tab's row with its eligibility; the second is the row's own default action (the old "View pool" and edit) |
| shell (start) | Rune | A second start still forwards the launcher's activation token to the running client, but the Kit has no call that activates a window with a foreign token: on Wayland the client asks the compositor itself, on the window's display, with the forwarded token (`gpui:src/wayland_activate.rs`). On sway 1.12 in a virtual machine that marks the first window urgent, and focuses it only where the compositor's policy allows; other compositors were not tried | The Kit puts a press serial on its own activation request, which wlroots does not honour without focus ([report](reports/2026-11-19-gpui-vm-session.md), finding 1) |
| shell (start) | Rune | A start that would open a window and names no display (neither `WAYLAND_DISPLAY` nor `DISPLAY`, the two the toolkit chooses its backend by; `WAYLAND_SOCKET` alone is none) ends with one line and status 1 before anything is restored. A second start from such a shell is forwarded to the running client as before (the check came before the forwarding until 2026-11-20). A start the toolkit gives up (a display it cannot reach, no seat, no usable font, no Vulkan driver) ends with one line and status 1 instead of a panic or a process that stays; any setting of `RUST_BACKTRACE` restores the toolkit's own report. Only the starting thread's panic is taken for a failed start, and one that is none of the known failures keeps the usual report with its place; the sessions are restored only once the window is open | The clean-install run found the packaged client silent or panicking in these cases ([report](reports/2026-11-19-gpui-clean-install.md)). The messages were seen once, in a debug build of 2026-11-19 in the Arch machine; the changes of 2026-11-20 to this code (the order with the forwarding, the thread and the known failures, the sessions after the window) have unit tests and were not run in a machine |
| shell (start) | Rune | The command line has `--version`, `--help`, `--demo`, `--scene NAME` with `--snap` and `--tree`, `--theme` and `--size`. The other options of the Rune toolkit (`--html`, `--video`, `--frames`, `--script`, `--native`, `--list-scenes` and the like) are refused with the usage and status 2, and so are `--theme` and `--size` on an ordinary start, which Rune accepted | The options were the toolkit's. The model's launch code knows the same four preview options as the parser |
| shell (options) | Rune | "Reduce animation" is applied by the application's window to the Kit (`set_reduce_motion`), not by a scene or a test, whose harness keeps reduced motion on for captures. The desktop's own reduced-motion setting is not followed (Rune followed it as well as the option). From the cutover until 2026-11-19 the option was stored and had no effect | The Kit reads no such setting of the system. Window test `the_applications_window_applies_the_option_reduce_animation` |
| `prefs` | Rune | A `preferences.json` that does not parse is still read as the defaults, but the first save moves it to `preferences.json.unreadable` instead of writing over it. Nothing is said in the window | The saved searches, tools and presets of such a file were lost for good at the first close, in Rune as well (review of the cutover) |

## Appendix: Rune controls without a key in the new crates

Every element key of the Rune crate (`crates/xenadmin-desktop/src` at commit `89d700c`) that is no string literal of the new crates
(136 keys; `python3 scripts/gpui_ledger_tables.py keys` lists them when `XENADMIN_RUNE_SOURCES` names the sources of the deleted crate). A key is
how a test or the accessibility driver finds a control, so a missing key is a control that was renamed,
replaced by a command, or removed. "dissolved" means the control has no successor because what it
belonged to is gone: a hub (a dialog of buttons that opened other dialogs), the Close or Back button of a
page that became a tab, a heading that became a title, or a scroll container. A formatted key is written
with its placeholder. The row that said "none" in the first version of this ledger, `cross-destination-{id}` of
`cross_pool_vm_view.rs` (show the destination VM of a finished transfer), has a successor since: the
record of a finished transfer on the Events tab carries `cross-destination-<id>`, so the key is no longer
absent and its row is gone.

| Rune file | key | what it was | successor |
| --- | --- | --- | --- |
| `activity.rs` | `activity-close` | Close button of the Activity dialog | dissolved: the dialog is the Alerts and Events tabs of the Notifications view |
| `activity.rs` | `activity-hide` | button "Hide completed entry" / "Restore entry" | `events.hide` |
| `activity.rs` | `activity-recheck` | button "Recheck outcome" | `events.recheck` |
| `activity.rs` | `activity-review` | button "Review cancellation…" | `events.cancel-task` |
| `activity.rs` | `activity-scroll` | scroll container of the dialog | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `alarms_view.rs` | `alarm-resource-{index}` | button "Configure alarms" for one object of the alarms picker | the page Alerts of the object's Properties (`vm.properties`, `server.properties`, `storage.properties`); the picker is gone |
| `bulk_cross_pool.rs` | `batch-cross-pool-body` | scroll container of the batch dialog | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `bulk_cross_pool.rs` | `batch-cross-pool-open` | button "Move selection to another pool…" (selection bar, row menu) | `vm.move-to-pool` on several VMs |
| `bulk_delete.rs` | `batch-delete-open` | button "Delete selection…" (selection bar, Templates page) | `vm.delete-several`, `templates.delete-several` |
| `bulk_export.rs` | `batch-export-open` | button "Export selection…" (selection bar) | `vm.export-selection`, `templates.export-selection` |
| `bulk_move.rs` | `batch-move-body` | scroll container of the batch dialog | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `bulk_move.rs` | `batch-move-open` | button "Move selection…" (selection bar, row menu) | `vm.migrate` and `vm.move` on several VMs |
| `bulk_power.rs` | `batch-power-open` | button "Power actions for selection…" | `vm.power-selection` |
| `bulk_properties.rs` | `batch-properties-open` | button "Edit selection…" | `vm.properties` / `template.properties` on several objects (page Bulk properties) |
| `bulk_properties.rs` | `batch-property-picker` | button "Property: …" that opened fourteen buttons | the choice `batch-property`; its options keep the Rune keys `batch-property-<Property>` |
| `cloud_init_view.rs` | `open-cloud-init` | button "Cloud-init drive…" | `vm.cloud-init-drive` |
| `console_windows.rs` | `console-windows` | button "Console windows (n)" that opened the windows manager | the Window menu, which lists the windows (`window.console-1` to `window.console-8`) |
| `console_windows.rs` | `console-windows-close` | button "Done" of the windows manager | dissolved: the manager dialog is the Window menu |
| `console_windows.rs` | `console-windows-close-all` | button "Close all consoles" | `window.console.close-all` |
| `console_windows.rs` | `close-console-window-{index}` | button of the manager that closed one window | dissolved with the manager: a window is closed from itself |
| `console_windows.rs` | `show-console-window-{index}` | button of the manager that raised one window | `window.console-N` |
| `cross_pool_vm.rs` | `cross-pool-open` | button "Move or copy to another pool…" of the details pane | `vm.move-to-pool` |
| `cross_pool_vm_view.rs` | `cross-recheck-{id}` | button "Recheck" of a transfer in the transfer history | `events.recheck` on the record in the History list of the Events tab |
| `cross_pool_vm_view.rs` | `cross-stop-{id}` | button to stop a transfer in the transfer history | `events.request-cancel` on the record in the History list |
| `disk_workspace.rs` | `disk-heading` | heading of the Virtual disks workspace | dissolved: the title of the dialog `virtual-disk` |
| `disk_workspace.rs` | `disk-list-back` | button back from a disk to the list | dissolved: the list is the Storage tab of a repository |
| `disk_workspace.rs` | `disks-open` | button "Virtual disks…" | the Storage tab of a repository, with the search field `disk-search` |
| `disk_workspace.rs` | `disk-manage-{id}` | button of a disk row that opened its workspace | `disk.grow`, `disk.copy`, `disk.cleanup` on the disk (row menu, context menu, Storage tab row) |
| `disk_workspace.rs` | `disk-mode-{label}` | mode buttons of the workspace (Grow, Copy, Clean up) | the three commands `disk.grow`, `disk.copy`, `disk.cleanup`: one mode of the dialog each |
| `disk_workspace.rs` | `disk-select-{id}` | button that selected a disk of the list | a row of the disk grid on the Storage tab of a repository |
| `host_admin.rs` | `host-admin-open` | button "Host administration…" that opened the chooser | dissolved: a hub. The Server menu and a server's context menu list the changes (`server.reboot`, `server.shutdown`, `server.restart-toolstack`, `server.maintenance`, `server.evacuate`, `server.power-on`, `server.control-domain-memory`, `server.destroy`, `pool.vm-autostart`, `server.properties`) |
| `host_admin.rs` | `host-monitoring-open` | button "Monitoring recording…" of the chooser | `server.monitoring-recording` |
| `host_console.rs` | `open-host-console` | button that opened the host text console | `server.console`; the Console tab of a server |
| `host_monitoring_view.rs` | `monitoring-heading` | heading of the monitoring dialog | dissolved: the title of the dialog `host-monitoring` |
| `host_monitoring_view.rs` | `monitoring-source-{}` | one button per data source, twelve a page | a row of the table `monitoring-sources` |
| `host_support_view.rs` | `host-support-open` | button "Host backup and support…" | `server.backup`, `server.restore-from-backup`, `tools.server-status-report` (the panel's three modes) |
| `inventory_information.rs` | `information-close` | Close button of the information dialog | dissolved: the dialog is the General tab |
| `inventory_information.rs` | `information-dialog` | the information dialog | the General tab of the selected object |
| `inventory_information.rs` | `information-pool` | button "Pool information" | the General tab of the pool |
| `inventory_information.rs` | `information-refresh` | button "Refresh from client" | dissolved: a tab shows the current inventory; there is nothing captured to refresh |
| `local_conversion_view.rs` | `conversion-heading` | heading of the conversion dialog | dissolved: the title of the wizard `local-conversion` |
| `main.rs` | `activity-open` | Activity button of the sidebar | the Notifications navigation view; the status bar's counters open it |
| `main.rs` | `add-connection` | button "Add connection" of the page header | `server.add` |
| `main.rs` | `alarms-open` | button that opened the alarms hub | dissolved: a hub. The pages Alerts and Email Options of the Properties dialogs, and `tools.alarm-service-refresh` |
| `main.rs` | `archives-open` | button "Archives" (import and export) | `file.import`; export is `vm.export`, `templates.export`, `disk.export-image` |
| `main.rs` | `back-to-inventory` | button back from the compact details to the table | dissolved: the compact swap between table and details pane is gone with the details pane |
| `main.rs` | `certificate-review-scroll` | scroll container of the certificate review | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `main.rs` | `connect-fields-scroll` | scroll container of the connect form | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `main.rs` | `connect-heading` | heading of the connect dialog | dissolved: the title of the dialog `connect-dialog` |
| `main.rs` | `conversion-open` | button "Convert local disk…" | `tools.convert-disk-image` |
| `main.rs` | `create-network` | button "New network" | `network.new` |
| `main.rs` | `create-storage` | button "New storage" | `storage.new` |
| `main.rs` | `cross-pool-history` | button "Cross-pool transfers" | the History list of the Events tab |
| `main.rs` | `disconnect` | button "Disconnect" of the page header | `server.disconnect`, `pool.disconnect` |
| `main.rs` | `help-open` | button "Help" | `help.help` |
| `main.rs` | `information-open` | button that opened the information dialog | the General tab |
| `main.rs` | `interface-zoom` | the element that scaled the whole interface | dissolved: the scale is the window's (`view.zoom-in`, `view.zoom-out`, `view.zoom-reset`) |
| `main.rs` | `inventory-table` | the inventory table | the grid `search-results` of the Search tab, and the navigation tree |
| `main.rs` | `manage-pool` | button "Pool administration" of the page header | dissolved: a hub. The tabs of the pool (Storage, Networking, HA, Users), the Pool menu and the Events tab for the history |
| `main.rs` | `open-console` | button "Console" of the details pane | the Console tab; `vm.console` |
| `main.rs` | `open-guest-tools` | button "Mount guest tools media" | `vm.install-guest-tools` |
| `main.rs` | `open-properties` | button "Properties" of the details pane | `vm.properties` |
| `main.rs` | `open-snapshots` | button "Snapshots" of the details pane | the Snapshots tab of a VM |
| `main.rs` | `open-template-properties` | button "Template properties" of the details pane | `template.properties` |
| `main.rs` | `organization-open` | button "Organize" | dissolved: a hub. The Organization navigation view and the `org.*` commands |
| `main.rs` | `repository-link` | link to the project repository in the sidebar header | `help.website`; the button `help-website` of About |
| `main.rs` | `search-pools-open` | button "Search pools" | the Search tab, which every selection has |
| `main.rs` | `settings-open` | button "Settings" | `tools.options` |
| `main.rs` | `show-details` | button that showed the details in a compact window | dissolved: the compact swap is gone; the tabs show the selected object at every size |
| `main.rs` | `snapshot-schedules-open` | button "Snapshot schedules" | `pool.snapshot-schedules` |
| `management_bond_ui.rs` | `ipv6-management-bond` | button "Staged IPv6 management bond…" | `server.management.bond.ipv6` |
| `management_bond_ui.rs` | `management-bond-create` | button "Create management bond…" | `server.management.bond.create` |
| `management_bond_ui.rs` | `management-bond-remove` | button "Remove management bond…" | `server.management.bond.remove` |
| `management_network_ui.rs` | `management-ip` | button "Change management IP…" | `server.management.configure` |
| `management_network_ui.rs` | `management-move` | button "Move management…" | `server.management.move` |
| `metrics_view.rs` | `close-performance` | button "Back to inventory" | dissolved: the page became a tab, which is left by choosing another tab or object; leaving drops the view and its subscription |
| `metrics_view.rs` | `metric-presets-open` | button "Graph presets…" | `performance.presets` |
| `metrics_view.rs` | `metrics-sources` | button "Data sources…" | `performance.data-sources` |
| `metrics_view.rs` | `open-performance` | button "Performance" of the details pane | the Performance tab of a server or VM; choosing it starts the stream |
| `network_admin.rs` | `network-administration-open` | button "Host networking…" that opened the hub | dissolved: a hub. The Networking tab of a pool, server or network and the NICs tab of a server |
| `network_admin.rs` | `network-review-line-{index}` | one line of a network review | a line of the review block `network-change-review` |
| `network_topology_ui.rs` | `topology-create` | button "Create pool-wide VLANs…" | `network.vlan.create-pool` |
| `network_topology_ui.rs` | `topology-retag` | button "Retag pool-wide VLANs…" | `network.vlan.retag-pool` |
| `pool_view.rs` | `pool-dissolve-open` | button that opened the dissolution of the pool | `pool.make-standalone` |
| `pool_view.rs` | `pool-join-open` | button that opened the join of a server | `server.join-pool` |
| `pool_view.rs` | `pool-root-password` | button that opened the password change | `pool.change-server-password`, `server.change-password` |
| `pool_view.rs` | `pool-security-open` | button "Authentication, access and certificates…" | the Users tab of a pool and its commands (`pool.directory.*`, `pool.subject.*`, `server.certificate.*`) |
| `profiles_view.rs` | `close-connections` | Close button of the connection manager | dissolved: the manager is the Home tab |
| `profiles_view.rs` | `connect-all` | button "Connect all" | `server.connect-all` |
| `profiles_view.rs` | `disconnect-all` | button "Disconnect all" | `server.disconnect-all` |
| `profiles_view.rs` | `inventory-table` | the inventory table, named for the keyboard focus | the grid `search-results` of the Search tab |
| `profiles_view.rs` | `manage-connections` | button that opened the connection manager | the Home tab (nothing selected) |
| `profiles_view.rs` | `new-profile` | button for a new saved connection | `server.add` |
| `profiles_view.rs` | `profiles-dialog` | the connection manager dialog | the Home tab |
| `profiles_view.rs` | `cancel-profile-{id}` | button that cancelled a connection attempt | `server.cancel-connect` |
| `profiles_view.rs` | `disconnect-profile-{id}` | button "Disconnect" of a connection card | `server.disconnect` |
| `profiles_view.rs` | `edit-profile-{id}` | button "Connect / edit" of a connection card | `server.connect` |
| `profiles_view.rs` | `inspect-failed-{id}` | button that reviewed the certificate of a failed connection | `server.review-certificate` |
| `profiles_view.rs` | `reconnect-profile-{id}` | button "Reconnect as another account…" | `server.reconnect-as`, `pool.reconnect-as` |
| `profiles_view.rs` | `remove-profile-{id}` | button that removed a saved connection | `server.remove-connection` |
| `profiles_view.rs` | `view-profile-{id}` | button "View pool" | activating the row of the Home tab's grid |
| `resource_admin.rs` | `pool-autostart` | button "Pool VM autostart…" | `pool.vm-autostart` |
| `resource_admin.rs` | `pool-information` | button "Pool information" | the General tab of the pool |
| `resource_admin.rs` | `resource-operation-notice` | notice "… · View operation history" | the status bar's counters of operations and unknown outcomes, which open the Events tab |
| `search_view.rs` | `search-close` | Close button of the search page | dissolved: the page became a tab, which is left by choosing another tab or object |
| `search_view.rs` | `search-export` | button that exported the saved searches | `search.export` |
| `search_view.rs` | `search-import` | button "Import…" | `file.import-search` |
| `search_view.rs` | `search-open-result` | button that opened the selected result in its pool | `search.open-result`; activating the row |
| `search_view.rs` | `search-saved` | toggle "Saved searches" | the Saved Searches navigation view; `search.save`, `search.remove` |
| `snapshot_inspection.rs` | `snapshot-inspection-dialog` | the "Inspect snapshot" dialog | the section `snapshot-information` of the Snapshots tab |
| `storage_view.rs` | `storage-heading` | heading of the storage dialogs | dissolved: the title of each surface |
| `storage_view.rs` | `storage-manage` | button that opened the storage hub | dissolved: a hub. The Storage tab and the `storage.*` commands |
| `vm_device_lifecycle.rs` | `device-lifecycle-body` | scroll container of the dialog | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `vm_device_lifecycle.rs` | `device-lifecycle-dialog` | the dialog for a new or removed device | the dialog `device-lifecycle` |
| `vm_device_lifecycle.rs` | `{prefix}-{id}` | one button of a list of repositories, networks or images (the prefix is a variable) | an option of the choice that replaced the list; option keys keep the Rune pattern (for example `lifecycle-sr-<id>`). The script cannot match a key whose prefix is a variable |
| `vm_device_properties.rs` | `nic-properties-{id}` | button of an interface row that opened its properties | `vm.nic.rate-limit` (and `vm.nic.properties`) on the row of the Networking tab |
| `vm_devices.rs` | `close-vm-devices` | button "Back to inventory" of the Devices page | dissolved: the page became a tab, which is left by choosing another tab or object |
| `vm_devices.rs` | `open-guest-tools` | button "Mount guest tools media" | `vm.install-guest-tools` |
| `vm_lifecycle.rs` | `close-vm-lifecycle-history` | Close button of the "VM changes" panel | dissolved: the panel is the Events tab |
| `vm_lifecycle.rs` | `vm-lifecycle-body` | scroll container of the dialog | dissolved: a scroll container; the body of a tab, dialog or wizard scrolls by itself |
| `vm_lifecycle.rs` | `vm-lifecycle-history` | button "VM changes" | the Events tab of the Notifications view |
| `vm_lifecycle.rs` | `{prefix}-choices` | the container of one list of choice buttons (the prefix is a variable) | dissolved: the list is a choice block of the wizard `vm-lifecycle-dialog` |
| `vm_lifecycle.rs` | `{prefix}-{id}` | one button of such a list | an option of that choice. The script cannot match a key whose prefix is a variable |
| `vm_migration.rs` | `open-vm-migration` | button "Migrate / move VM" of the details pane | `vm.migrate`, `vm.move` |
| `vm_properties.rs` | `properties-dialog` | the VM properties dialog | the kernel's dialogs `vm-properties` and `template-properties` |
| `vm_recovery_view.rs` | `vm-recovery-open` | button "Recovery controls…" that opened the recovery overview | dissolved: a hub. `vm.recovery-start`, `vm.recovery-restore-boot` and the controls under an operation record |
| `vm_snapshots.rs` | `back-from-snapshots` | button "Back to inventory" of the Snapshots page | dissolved: the page became a tab, which is left by choosing another tab or object |
| `vm_snapshots.rs` | `clone-snapshot-template` | button "Create template" | `snapshot.new-template` |
| `vm_snapshots.rs` | `clone-snapshot-vm` | button "Create VM" | `snapshot.new-vm` |
| `vm_snapshots.rs` | `create-snapshot` | button that took a snapshot | `vm.snapshot` |
| `vm_snapshots.rs` | `delete-snapshot` | button "Delete" | `snapshot.delete` |
| `vm_snapshots.rs` | `edit-snapshot` | button "Edit properties" | `snapshot.properties` |
| `vm_snapshots.rs` | `inspect-snapshot` | button "Inspect snapshot" | the section `snapshot-information` of the tab, shown for the one selected snapshot |
| `vm_snapshots.rs` | `restore-snapshot` | button "Restore snapshot" | `snapshot.revert` |
| `vm_snapshots.rs` | `snapshot-activity` | button "Activity" | the Events tab of the Notifications view |
| `vm_snapshots.rs` | `snapshot-dialog` | the one dialog of the snapshot actions | the dialogs `snapshot-create`, `snapshot-restore`, `snapshot-edit`, `snapshot-clone`, `snapshot-delete` |
