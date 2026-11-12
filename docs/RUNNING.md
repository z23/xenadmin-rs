# Run XenAdmin RS

This is a development alpha. Local protocol fixtures exercise the connection, console,
and power-operation paths, and a disposable nested XCP-ng 8.3.0 lab has exercised them
live; the project's live qualification report records exactly what was run. Other host
versions, real desktop seats and most failure paths remain unqualified.
Use a disposable lab before relying on it for infrastructure recovery.

## Build from source

Install the toolchain pinned in `rust-toolchain.toml`. Rustup downloads it automatically
when you run Cargo from this checkout. Linux builds need a C compiler, pkg-config,
Wayland development files, and xkbcommon development files. Runtime needs include
`libxkbcommon`, `libxkbcommon-x11`, `libwayland-client`, fontconfig, and a Vulkan driver.

On Debian or Ubuntu, the relevant packages include `build-essential`, `pkg-config`,
`libwayland-dev`, `libxkbcommon-dev`, `libxkbcommon-x11-0`, `libfontconfig1-dev`,
`libvulkan1`, and `mesa-vulkan-drivers`. A real display session is required for a window.
Headless render checks still require a hardware or software Vulkan implementation.

Run these commands from the source checkout:

```sh
cargo build --locked --workspace
./target/debug/xenadmin-rs
```

To explore synthetic inventory without connecting to a host:

```sh
./target/debug/xenadmin-rs -- --demo
```

The demo is explicitly labeled and cannot issue host actions.

`xenadmin-rs --version` prints the version and exits without opening a window or
touching the state directory.

On the development workstation, native Wayland presentation works. The X11 path through
Xwayland needs explicit GPU selection on its hybrid Intel/NVIDIA setup; automatic selection
failed during surface configuration. This command passed the local X11 smoke test:

```sh
WGPU_ADAPTER_NAME=NVIDIA ./target/debug/xenadmin-rs
```

`WGPU_ADAPTER_NAME` matches part of an adapter's name. This is a workstation-specific
workaround, not a claim that all X11 systems need NVIDIA hardware. Standalone Xorg,
other GPUs, software rendering, and clean-distribution installation remain to be qualified.

## Connect to a host

1. Click **Add connection**.
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
6. Click **Connect** and wait for current inventory.
7. Select a VM. Use the available power controls or click **Open console**.

Passwords stay in the running session by default. **Remember password in desktop keyring**
opts into Secret Service storage. Locked/unavailable keyrings leave a working manual-password
path. **Connections** manages saved profiles and switches among concurrent pools; custom CA,
read-only mode and timeout live under connection options. See [connection details](CONNECTIONS.md).

**Options** in the toolbar holds the appearance, the interface size and the console
defaults. Ctrl with plus, minus or 0 makes the whole interface larger, smaller or 100
percent again from anywhere outside a console, and the choice is kept between runs. A
console opened in a window of its own is drawn at the same size. Buttons
follow one scheme: outlined buttons open a view or dismiss a dialog, filled buttons act on
the selected object, red labels mark destructive or forced actions, and the accent colour
marks the one action a view is for.

Only one desktop can write a state directory. Close older application versions before
starting this build. Host actions use advertised capabilities and fresh checks; XAPI enforces
authorization again. **Check action availability** asks the current host for detailed reasons.
**Start on host** and **Resume on host** review a chosen destination and recheck compatibility.

Force operations require an explicit review. Closing an operation's observer does not
cancel the host task. **Request cancellation** records the request durably, asks the host
when supported, and waits for its reported outcome. Cancellation remains attached to the
operation after reconnect. The client retries transient task-read failures without
repeating the original mutation.

An interrupted request can remain **OutcomeUnknown** and block another mutation on that VM.
Use **Recheck outcome** when a saved task or property-change intent is available. **Review unknown outcome** allows
an explicit acknowledgement after you inspect the host and stop other instances handling
the operation. This releases the local block; the result remains unverified and any host
task may still continue. Close older application versions before using the updated journal.

## Manage VM snapshots

Select a virtual machine and choose **Snapshots** in Details. The view lists snapshots
from the same direct XAPI inventory, with their names, creation times and UUIDs.
**Create snapshot** captures disks/configuration. **Include guest memory** selects XAPI's
checkpoint operation when advertised by the host. Neither mode adds application quiescing;
quiesced/VSS snapshots were removed in XCP-ng 8.1. Shut down the guest or quiesce its
applications first if the workload requires application-consistent data. The host controls
availability, and confirmation checks the selected mode.

To restore, shut down the VM, select a snapshot and choose **Restore snapshot**. Review
both UUIDs before confirming: restoration replaces the configuration and disk contents,
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
and offers navigation to it after identity verification. If conversion from template to VM
cannot be confirmed, the record distinguishes the existing clone from the incomplete
conversion; rechecking does not repeat either write.

**Delete snapshot** defaults to deleting the snapshot record. The review separately lists
the eligible owned snapshot disks and the disks that will be retained. Enabling disk
cleanup binds the request to the exact reviewed UUIDs; a changed ownership or attachment
check stops deletion. Shared, unowned and unselected disks are retained. Deleting a memory
checkpoint also discards its saved memory, which is shown in the review. Completed steps
stay completed if a later step fails, and interrupted outcomes remain visible in history.

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

The check script uses isolated temporary state and local protocol fixtures. It renders
the demo and connection dialog under `artifacts/`. Packaging writes a release archive to
the same ignored directory.
Archives also include declared dependency licenses, supplied notices, and embedded-font
notices under `third-party/`. The application is under the MIT licence; its XAPI bindings
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

**Properties** edits name, description and autostart; a halted VM also exposes available CPU,
memory-limit and boot-order settings. Memory fields accept whole byte counts or KiB/MiB/GiB.
Review old/new values before applying. Changes execute in order, verify their saved values,
and stop on failure. The operation history distinguishes verified, failed, pending and
unknown steps. Rechecking uncertain properties reads the host without repeating a setter.
Pool-level autostart must also be enabled for a VM's autostart setting to take effect.

**Disks and network interfaces** shows attachments, VDIs/SRs, ISO drives, NICs/uplinks and
guest tools/addresses. Reviewed controls support disk/CD/NIC creation and removal, ISO
insert/eject, activation/deactivation, eligible disk growth and device-property changes.
Growing a virtual disk does not resize guest partitions or filesystems. Every action reviews
the captured VM/device and rechecks ownership and current host restrictions before dispatch.
Backing disk deletion is separate from attachment removal; the review shows retained data.

## Console controls

The toolbar provides special keys, explicit clipboard send/copy, fit/actual size, fullscreen,
release input and reconnect. Clipboard transfers are bounded and use negotiated UTF-8 when
available; legacy servers accept only representable Latin-1 text. Middle-button and wheel
input are supported. Rune's current secondary-click API lacks a release callback, so use
**Hold right button** and **Release input** for right-button dragging. Focus loss releases
held input. Reconnect keeps the original VM target even if table selection changes.
Up to eight separate console windows keep their original targets. Broader native
keyboard/layout and compositor qualification remain open.

## Host networking

Choose **Host networking…** in host or network Details. Select an existing network to add a
VLAN interface or unused-NIC bond. The dialog also offers physical NIC scans, interface
attachment, IPv4 settings, MTU, automatic new-VM attachment and pool IGMP snooping. Changes
show a separate review and use the pool operation history. Network changes can interrupt
traffic. Management changes use a separate captured reconnection plan; ordinary bond
changes require reviewed VLAN/IP/VM dependencies. GRE/VXLAN and SR-IOV workflows expose
host coverage and reviewed pool-wide changes. Controller state, actual traffic and
reboot outcomes remain separate from configuration verification.

## Convert and export disks locally

Offline disk conversion accepts self-contained raw, VHD, QCOW2, VMDK and VHDX images
and produces raw or dynamic VHD. Choose the input format and trusted qemu-img executable,
then review the inspected image before conversion. A successful output can continue to a
separate disk-import review; conversion alone does not contact a host or create a guest.

This optional workflow requires Linux x86-64, QEMU 8 or newer with the requested formats,
Bubblewrap, util-linux prlimit, working user namespaces and a kernel supporting
close_range(CLOEXEC). These tools are not bundled or installed by the application.
Unsupported backing files, encryption and internal snapshot selection are refused.
Every inspection/conversion/comparison runs in the same restricted sandbox. Cancel local
work stops the process; interrupted history offers cleanup of owned temporary files,
without resuming conversion or deleting published output.

Select multiple eligible halted VMs or custom templates for a reviewed serial XVA or
OVF/OVA export. Each source has its own destination; existing outputs are never replaced.
Stop finishes the current transfer and skips the remainder. Recheck inspects existing
evidence and never publishes a new file. Snapshot and checkpoint exports have their own
reviews; full XVA does not include snapshot ancestry.

## Dissolve a pool

Pool administration offers **Dissolve pool…**. Review the retained coordinator and exact
member order, then enter the pool UUID. Members must already be disabled, evacuated and
free of local guest-disk data, with HA disabled. Ejection reboots each selected member;
the coordinator remains and its pool name/description are cleared last. Stop and recheck
never submit remaining steps automatically. Verify each ejected host independently after
reboot. Qualification of these host changes remains deferred to a disposable lab.
