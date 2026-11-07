# Run XenAdmin RS

This is a development alpha. Local protocol fixtures exercise the connection, console,
and power-operation paths. A real XCP-ng host has not yet qualified this build.
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
   choice is remembered with the last endpoint and shown in the connection status.
5. With validation enabled, if other pool hosts use self-signed certificates, add their verified pins as
   `https://host=SHA256` entries, one per line. Coordinator redirects and console tunnels
   verify their destination independently.
6. Click **Connect** and wait for current inventory.
7. Select a VM. Use the available power controls or click **Open console**.

The password stays in the running session. The current build remembers the last endpoint,
username, accepted pins, sidebar width, and column widths. It does not save passwords or
provide keyring integration yet. Host actions are enabled from the host's advertised
capabilities, and XAPI checks authorization again.

Force operations require an explicit review. Closing an operation's observer does not
cancel the host task. **Request cancellation** records the request durably, asks the host
when supported, and waits for its reported outcome. Cancellation remains attached to the
operation after reconnect. The client retries transient task-read failures without
repeating the original mutation.

An interrupted request can remain **OutcomeUnknown** and block another mutation on that VM.
Use **Recheck host task** when a saved task is available. **Review unknown outcome** allows
an explicit acknowledgement after you inspect the host and stop other instances handling
the operation. This releases the local block; the result remains unverified and any host
task may still continue. Close older application versions before using the updated journal.

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
notices under `third-party/`.

To run an extracted archive, execute `bin/xenadmin-rs` inside it. For optional desktop
integration, copy the binaries into a directory on your PATH, the `.desktop` file into
`~/.local/share/applications`, and the SVG into
`~/.local/share/icons/hicolor/scalable/apps`. No host-side installation is required.
