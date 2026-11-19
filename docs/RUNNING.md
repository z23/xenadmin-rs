# Run XenAdmin RS

This is a development alpha. Local protocol fixtures exercise the connection, console,
and power-operation paths, and a disposable nested XCP-ng 8.3.0 lab has exercised them
live; the project's live qualification report records exactly what was run. Other host
versions, real desktop seats and most failure paths remain unqualified.
Use a disposable lab before relying on it for infrastructure recovery.

## Build from source

Install the toolchain pinned in `rust-toolchain.toml`. Rustup downloads it automatically
when you run Cargo from this checkout. The interface is built with GPUI Kit. Linux builds
need a C compiler, pkg-config, and the development files of xcb, xkbcommon, xkbcommon-x11
and fontconfig (which brings freetype's). The client is linked against `libxcb`,
`libxkbcommon` and `libxkbcommon-x11` and does not start without them, in a Wayland
session either. At run time it also needs `libwayland-client` in a Wayland session, a
Vulkan loader and a Vulkan driver, and a font of one of the families it looks for by name
(DejaVu Sans and Noto Sans among them); it does not load fontconfig. The
[installation guide](INSTALLATION.md#runtime-requirements) has the run-time packages for
Ubuntu 24.04 and Arch as a clean-system check confirmed them.

On Debian or Ubuntu the build packages are the ones CI and the release workflow install:
`build-essential`, `pkg-config`, `libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libxcb1-dev`,
`libfontconfig1-dev`, `libvulkan1` and `mesa-vulkan-drivers` (and `desktop-file-utils` for
packaging). That list was derived from the link line and the build scripts on the
development workstation (Arch); CI's check and its packaging pass with it on Ubuntu 24.04.
A real display session is required for a window. Headless
render checks still require a hardware or software Vulkan implementation (Mesa's lavapipe
is enough). A release build of the client took about three minutes on the development
workstation, and the binary is about 110 MB.

Run these commands from the source checkout:

```sh
cargo build --locked --workspace
./target/debug/xenadmin-rs
```

To explore synthetic inventory without connecting to a host:

```sh
./target/debug/xenadmin-rs --demo
```

The demo is explicitly labeled and cannot issue host actions.

`xenadmin-rs --version` prints the version and exits without opening a window or
touching the state directory.

The window is at least 800 by 600. Below a width of 1100 the navigation pane is behind
its toggle.

Status of the window itself: nobody has used the GPUI Kit front end in a window yet. It
passes its tests on the toolkit's headless test platform, every scene renders off screen,
and it has been driven headless against the lab
([report](https://github.com/z23/xenadmin-rs/blob/main/docs/reports/2026-11-19-gpui-live-headless.md)). The packaged client has been
installed on untouched Ubuntu 24.04 and Arch systems in virtual machines and has opened its
window there with software rendering: in a Wayland compositor that had no seat, and, with
scripted input, in an X session without Wayland
([clean-install report](https://github.com/z23/xenadmin-rs/blob/main/docs/reports/2026-11-19-gpui-clean-install.md)). A Wayland session with
a person at it, a hardware GPU and GPU selection on a hybrid machine remain to be shown
for it. What earlier versions of this guide
recorded about them (native Wayland presentation working, an X11 smoke test through
Xwayland that needed an explicit adapter) was observed with the Rune interface of the
releases up to 0.19.0 and does not carry over.

Every action is a command in the menu bar, the toolbar, an object's context menu or the row
of actions at the top of a tab, and what shows state is a tab of the selected object. The
sections below name each control by its label.

## Connect to a host

1. Choose **Add Server…** (toolbar, Server menu or the Home tab).
2. Enter a host management URL, username, and password.
3. Leave **Disable certificate validation** unchecked for normal CA verification. Under
   **Certificate options**, you can enter a certificate SHA-256 fingerprint verified through
   a separate trusted channel. Certificate pins apply to exact endpoints, including ports.
4. To bypass certificate validation, check **Disable certificate validation** once. This
   applies to the initial member, coordinator redirects, and consoles for that connection.
   HTTPS remains enabled, but certificate identity and validity checks are skipped. The
   choice is saved with that profile and shown in the connection status.
5. With validation enabled, if other pool hosts use self-signed certificates, add their verified pins as
   `https://host=SHA256` entries, one per line. Coordinator redirects and console tunnels
   verify their destination independently.
6. Click **Connect** and wait for current inventory. The tree shows the pool, its servers,
   guests and repositories.
7. Select a VM. Use the power commands of the toolbar or the VM menu, or open its
   **Console** tab.

Passwords stay in the running session by default. **Remember password in desktop keyring**
opts into Secret Service storage. Locked/unavailable keyrings leave a working manual-password
path. The **Home** tab lists the saved connections and switches among concurrent pools; custom CA,
read-only mode and timeout live under connection options. See [connection details](CONNECTIONS.md).

**Options** in the Tools menu holds the appearance, the interface size and the console
defaults. **Larger Interface**, **Smaller Interface** and **Reset Interface Size** in the
View menu change the size of the whole interface, and the choice is kept between runs. A
console opened in a window of its own is drawn at the same size.

Only one desktop can write a state directory. Close older application versions before
starting this build. Host actions use advertised capabilities and fresh checks; XAPI enforces
authorization again. **Check action availability**, in the row of actions of a VM's
**General** tab, asks the current host for detailed reasons. **Start on Server** and
**Resume on Server** in the VM menu review a chosen destination and recheck compatibility.

Force operations require an explicit review. Closing an operation's observer does not
cancel the host task. An operation is listed with its controls under **Operations** on the
VM's **General** tab and on the **Events** tab of the **Notifications** view, whose row of
actions acts on the selected entry. **Request cancellation** records the request durably,
asks the host when supported, and waits for its reported outcome. Cancellation remains
attached to the operation after reconnect. The client retries transient task-read failures
without repeating the original mutation.

An interrupted request can remain **OutcomeUnknown** and block another mutation on that VM.
Use **Recheck outcome** when a saved task or property-change intent is available. **Review
unknown outcome** (**Acknowledge unknown outcome…** on the Events tab) allows
an explicit acknowledgement after you inspect the host and stop other instances handling
the operation. This releases the local block; the result remains unverified and any host
task may still continue. Close older application versions before using the updated journal.

## Manage VM snapshots

Select a virtual machine and open its **Snapshots** tab. The tab shows the VM's snapshots
from the same direct XAPI inventory, as a tree or (**Show list**) as a list, and
**Snapshot information** for the one selected. The commands are in the row of actions at
the top of the tab and act on the selected snapshot; **Take a Snapshot** is also in the VM
menu and the VM's context menu.

**Take a Snapshot** opens **Create snapshot**, which captures disks/configuration.
**Include guest memory** selects XAPI's
checkpoint operation when advertised by the host. Neither mode adds application quiescing;
quiesced/VSS snapshots were removed in XCP-ng 8.1. Shut down the guest or quiesce its
applications first if the workload requires application-consistent data. The host controls
availability, and confirmation checks the selected mode.

To restore, shut down the VM, select a snapshot and choose **Revert**. The dialog
**Restore this snapshot?** shows both UUIDs; review them before confirming with **Restore
snapshot**: restoration replaces the configuration and disk contents,
and newer changes are lost. The backend rechecks the halted state, snapshot identity and
parent relationship immediately before dispatch. The review offers explicit clean shutdown before restore and optional start or resume
after verified restore. Leaving those options off keeps the corresponding power state
unchanged; no recovery step is replayed after interruption.

Snapshot actions share the operation history, cancellation and unknown-outcome handling
above. A successful create is reported only after verifying the returned snapshot identity.
A lost reply never causes automatic recreation. The demo previews both dialogs but cannot
confirm host changes. Optional shutdown/restore/start or resume steps have separate
durable results and stop on failure.
Snapshot writes are fixture-verified; live qualification requires
an explicitly designated disposable lab.

**Create VM** and **Create template** clone the selected snapshot after reviewing its UUID
and the new name. A created VM remains halted. The operation records the new object's UUID
and offers navigation to it (**Show created VM** or **Show created template** under the
operation on the tab) after identity verification. If conversion from template to VM
cannot be confirmed, the record distinguishes the existing clone from the incomplete
conversion; rechecking does not repeat either write.

**Delete** opens **Delete this snapshot?**, which defaults to deleting the snapshot record.
The review separately lists
the eligible owned snapshot disks and the disks that will be retained. Enabling disk
cleanup (**Also delete … exclusively owned snapshot disk(s)**) binds the request to the
exact reviewed UUIDs; a changed ownership or attachment
check stops deletion. Shared, unowned and unselected disks are retained. Deleting a memory
checkpoint also discards its saved memory, which is shown in the review. Completed steps
stay completed if a later step fails, and interrupted outcomes remain visible on the
**Events** tab of the **Notifications** view.

## Use the read-only host probe

```sh
./target/debug/host-probe https://xcp-host.example --username root --json
```

The probe prompts for a password in the terminal. `--password-stdin` reads one line from
standard input for an existing secret-manager command. Do not put passwords in arguments.
Use `--ca-cert PATH` for a private CA, `--cert-sha256 HEX` for the initial endpoint, and
repeat `--host-cert https://member=HEX` for other verified pool endpoints. `--watch SECONDS`
observes inventory after the first synchronization.
`--insecure` disables certificate validation for the entire connection, including coordinator
redirects. It overrides certificate pins and custom CA settings.

To investigate unavailable power actions on one VM:

```sh
./target/debug/host-probe https://xcp-host.example --diagnose-vm VM_UUID
```

Add `--insecure` when deliberately using the certificate-validation opt-out, or `--json`
for structured output. This mode reads the selected VM's advertised allowed, current and
blocked operations, then checks each power action without submitting it. Administrative
block messages are displayed as text, including when the host supplies no reason. It
does not dump other VMs or the entire pool. Incomplete checks exit nonzero; a successfully
diagnosed block is not a probe failure. The output can still contain private VM names,
UUIDs and administrator messages, so keep it local while investigating.

## Local state

State is stored under `$XDG_STATE_HOME/xenadmin-rs`, or `~/.local/state/xenadmin-rs`.
`XENADMIN_STATE_DIR` overrides the directory for isolated testing. New state directories
use mode 0700, and new preference and journal files use mode 0600.

`preferences.json` stores UI choices and endpoint metadata. `operations.sqlite3` stores
operation intentions, task references, and observed outcomes. Journal entries contain no
passwords or API sessions. Preserve the journal when investigating an interrupted action.

## Check and package

```sh
./scripts/check.sh
./scripts/package.sh
```

The check script uses isolated temporary state and local protocol fixtures and opens no
window. It renders every scene under `artifacts/ui/`. Packaging writes a release archive
under the same ignored directory `artifacts/`.
Archives also include declared dependency licenses and supplied notices under
`third-party/`. The application is under the MIT licence; its XAPI bindings
(`xenadmin-xapi`) are under the LGPL 2.1 with a linking exception, with that licence and
a notice of their source in `third-party/xenadmin-xapi-VERSION/`. The complete source of
every release is at <https://github.com/z23/xenadmin-rs> under the release's tag. `./scripts/package.sh --appimage` also downloads the pinned
AppImage inputs into `target/package-tools`, verifies each against its recorded hash, and
retries a transient mirror error a few times before refusing the build; CI caches that
directory keyed on the input list.

To run an extracted archive, execute `bin/xenadmin-rs` inside it. Optional local
installation, upgrades, retained versions, removal and AppImage use are described in
[the installation guide](INSTALLATION.md). No host-side installation is required.


## Edit VM properties and devices

**Properties** (VM menu, the VM's context menu, or the row of actions of its **General**
tab) opens the VM's Properties dialog. Its page **General** edits name and description and
**Start Options** the autostart; for a halted VM, **CPU and Memory** and **Boot Options**
also expose available CPU, memory-limit and boot-order settings. Memory fields accept whole
byte counts or KiB/MiB/GiB. **Review changes** shows the old and new values on the page
**Review Changes**; review them before **Apply changes**. Changes execute in order, verify
their saved values, and stop on failure. The operation history distinguishes verified,
failed, pending and unknown steps. Rechecking uncertain properties reads the host without
repeating a setter. Pool-level autostart (**VM Autostart** in the Pool menu) must also be
enabled for a VM's autostart setting to take effect.

A VM's **Storage** tab shows its disk and CD/DVD attachments with their VDIs and SRs, and
its **Networking** tab its virtual interfaces with their networks; guest tools and
guest-reported addresses are on the **General** tab. The commands of the row of actions at
the top of each tab act on the selected row. On **Storage**: **New Virtual Disk**, **Attach
Virtual Disk**, **Add CD/DVD Drive**, **Detach**, **Delete**, **Insert ISO**, **Eject
Media**, **Activate**, **Deactivate**, **Resize** (eligible disk growth) and **Properties**.
On **Networking**: **Add**, **Remove**, **Activate**, **Deactivate**, **Properties** and
**Rate Limit**.
Growing a virtual disk does not resize guest partitions or filesystems. Every action reviews
the captured VM/device and rechecks ownership and current host restrictions before dispatch.
Backing disk deletion (**Delete Owned Disk**) is separate from attachment removal; the
review shows retained data.

## Console controls

The row of actions at the top of the **Console** tab provides special keys (**Send
Ctrl+Alt+Del**, **Ctrl+Alt+F1**, **Alt+Tab**, **Ctrl+C**, **Ctrl+D**), explicit clipboard
send/copy (**Send Clipboard to VM**, **Copy VM Clipboard**), **Type Clipboard as Keys…**
with its review, **Actual Size** (the console is fitted otherwise), **Fullscreen**, **Open
in Separate Window**, **Save Screenshot…**, **Release Input**, **Reconnect** and **Close
Console**. A separate console window has its controls as buttons of its own.
Clipboard transfers are bounded and use
negotiated UTF-8 when available; legacy servers accept only representable Latin-1 text.
Middle-button and wheel input are supported, and the right button's press and release go
to the guest as they happen (there is no control to hold the right button).
Click the console to type; while it has the keyboard every key is the guest's.
Focus loss releases held input. Reconnect keeps the original VM target even if the
selection changes. Up to eight separate console windows keep their original targets; the
**Window** menu lists them and has **Close All Consoles**. The
local pointer stays visible over the console and the guest's own cursor is not drawn.

In a Wayland session the console's keys are read by key code through a second keyboard
listener on the display ([decision 0006](https://github.com/z23/xenadmin-rs/blob/main/docs/decisions/0006-console-key-codes.md)); elsewhere
(X11) the toolkit's keystrokes are forwarded with codes looked up on the US layout, which
is experimental. Neither path has been used with a console in a real window yet: the
headless lab runs sent keys through the toolkit's test events. Native keyboard, layout,
clipboard, fullscreen and compositor qualification remain open.

## Host networking

The **Networking** tab of a pool, a server or a network and the **NICs** tab of a server
show the networks and the interfaces. The commands are in the row of actions at the top of
the tab, and those of an interface also in the menu of its row. Select an existing network
to add a VLAN interface (**Add VLAN Interface**); **Create Bond** on a server's **NICs** tab
bonds unused NICs. The tabs also offer physical NIC scans (**Rescan** on **NICs**), interface
attachment (**Plug Interface** in an interface's row menu) and IPv4 settings (**Configure
IPv4**). MTU and automatic new-VM attachment are on the page **Network Settings** of a
network's **Properties**, and pool IGMP snooping on the page **Network Options** of the
pool's **Properties**. Changes
show a separate review and use the pool operation history (**Operation history** under the
grids shows it). Network changes can interrupt
traffic. Management changes (**Configure Management IP**, **Move Management Interface**) use
a separate captured reconnection plan; ordinary bond
changes require reviewed VLAN/IP/VM dependencies. GRE/VXLAN and SR-IOV workflows (**Add
Tunnel or SR-IOV Interface**, **Complete Pool Coverage**) expose
host coverage and reviewed pool-wide changes. Controller state, actual traffic and
reboot outcomes remain separate from configuration verification.

## Convert and export disks locally

**Convert Disk Image** in the Tools menu opens **Local disk conversion**. Offline disk
conversion accepts self-contained raw, VHD, QCOW2, VMDK and VHDX images
and produces raw or dynamic VHD. Choose the input format and trusted qemu-img executable,
then review the inspected image (**Inspect and review**) before conversion (**Convert
reviewed image**). A successful output can continue to a
separate disk-import review (**Review disk import…**); conversion alone does not contact a
host or create a guest.

This optional workflow requires Linux x86-64, QEMU 8 or newer with the requested formats,
Bubblewrap, util-linux prlimit, working user namespaces and a kernel supporting
close_range(CLOEXEC). These tools are not bundled or installed by the application.
Unsupported backing files, encryption and internal snapshot selection are refused.
Every inspection/conversion/comparison runs in the same restricted sandbox. **Cancel local
work** stops the process; interrupted history offers cleanup of owned temporary files
(**Review temporary file cleanup**), without resuming conversion or deleting published
output.

Select multiple eligible halted VMs or custom templates and choose **Export** in the VM
menu (**Export to File** in the Templates menu) for a reviewed serial XVA or
OVF/OVA export. Each source has its own destination; existing outputs are never replaced.
**Stop remaining steps** finishes the current transfer and skips the remainder. **Recheck
outcome** inspects existing
evidence and never publishes a new file. Snapshot and checkpoint exports have their own
reviews; full XVA does not include snapshot ancestry.

Several snapshots of one VM can go out in one such batch. On the VM's **Snapshots** tab,
Shift-click or Ctrl-click selects 1 to 32 snapshots and **Export selection…** in the row
of actions opens the
review. Each snapshot becomes an archive of its own at its own new destination. A memory
checkpoint is exported as XVA only; its archive holds the guest's memory only where
**Include guest memory** is chosen for that checkpoint, and such an archive may contain
credentials and imports as a suspended guest. This batch is fixture-verified and has not
been run against a host.

## Dissolve a pool

**Make into Standalone Server** in the Pool menu (and a pool's context menu) opens the
review **Dissolve pool**. Review the retained coordinator and exact
member order, then enter the pool UUID under **Confirmation pool UUID**; **Dissolve
reviewed pool** submits. Members must already be disabled, evacuated and
free of local guest-disk data, with HA disabled. Ejection reboots each selected member;
the coordinator remains and its pool name/description are cleared last. Stop and recheck
never submit remaining steps automatically. Verify each ejected host independently after
reboot. Qualification of these host changes remains deferred to a disposable lab.
