# Verification requirements

## Evidence status

This document specifies acceptance requirements. The [alpha report](reports/2026-11-07-alpha.md)
records the first local fixture, rendering, and build results. Live results against the
disposable nested lab are recorded in the
[lab qualification report](reports/2026-11-09-lab-qualification.md) and per acceptance row in
[lab/ACCEPTANCE.md](../lab/ACCEPTANCE.md). Fixture success does not establish host
compatibility, and a passed scenario does not establish its failure or interruption paths.

The front end was moved from the Rune toolkit to GPUI Kit
([migration plan](GPUI-MIGRATION.md)). Every desktop result recorded before 2026-11-19 (the
driven desktop sessions, the packaged AppImage runs, the operator's sessions on a real
seat, the clean-install report) was obtained with the Rune interface. For the GPUI Kit
front end the recorded evidence is: its model's tests, its window tests on the Kit's
headless test platform, a render of every scene, the headless runs against the lab of
2026-11-19 ([successful paths](reports/2026-11-19-gpui-live-headless.md),
[refusals, interruptions and recoveries](reports/2026-11-19-gpui-live-paths.md)), and two
runs of the packages CI built at commit `e808a2f`, made the same day in virtual machines
with software rendering and nobody at them: a clean installation on Ubuntu 24.04 and Arch
([report](reports/2026-11-19-gpui-clean-install.md)) and a driven Wayland session with
input injected at the machine's devices
([report](reports/2026-11-19-gpui-vm-session.md)). On 2026-11-20 the scenarios of the first
headless report were run again on the merged tree, and the AppImage CI built of that tree
(commit `3493640`) was driven in the session's machine through its start and the ways it
fails, a second start, a guest's console with a held key, the Snapshots tab and its close
([report](reports/2026-11-20-gpui-merged-tree.md)). On 2026-11-21 the operator ran a local
build and reported that it looks good; the run names no compositor or renderer. It
has not drawn on a hardware GPU. Screen readers will not be supported (decision D11). The gate
reports of 2026-11-19 (`reports/2026-11-19-gpui-gate-g2.md` to `-g5.md`) give each
criterion's state.

The [verification traceability report](reports/2026-11-11-verification-traceability.md)
maps each core scenario below, the console qualification list and the outage acceptance run
to the fixture tests and live scenarios that cover them, and names what is still uncovered.

Record results under `docs/reports/`. Each report names the
application commit, dependency lockfile, host/API versions, desktop protocol, distribution,
hardware or software renderer, scenario, actual result, and remaining limitations.
Keep private host identifiers and raw captures outside Git. Use sanitized fixture names
in committed reports.

## Test layers

1. Pure core tests cover inventory reducers, relationship indexes, command eligibility,
   connection-generation handling, operation transitions, and conditional cleanup plans.
2. A controllable protocol server exercises request encoding, decoding, timeouts, redirects,
   event continuation, response loss, task polling, and malformed data. Assert behavior at
   the application facade rather than private helper structure.
3. The interface has three headless layers. Model tests (`cargo test -p xenadmin-ui`)
   cover selection, tabs, command eligibility with its reasons, editors, reviews and
   refusals without any window. Window tests (`cargo test -p xenadmin-desktop --features
   scenes`) drive the GPUI Kit front end with real pointer, key and text events on the
   Kit's headless test platform. Scene renders (the scene tool `xenadmin-scenes`, run for
   every scene by `scripts/check.sh`) cover realistic inventory, loading/stale/empty
   states, action availability, errors and operation history as a PNG and an element tree.
   Use stable element keys and inspect tree semantics as well as pixels. The test platform
   has no compositor, no seat, no clipboard of the system and no portal.
4. Real desktop sessions verify Wayland and X11 windows, input, scale, clipboard, file
   dialogs, AT-SPI/Orca, and the software renderer. Headless output cannot establish these.
5. A designated disposable XCP-ng lab verifies direct connections, actual task semantics,
   console negotiation, recovery, and infrastructure-changing workflows.

CI initially runs only offline tests. Live jobs need an explicit lab target and allowed
operation scope. They must never discover production targets from a developer's saved profiles.

## Core scenarios

| ID | Scenario | Required observation |
| --- | --- | --- |
| V01 | Cold startup with XO and internet unreachable | Saved direct endpoints and a session-only login remain usable |
| V02 | Login succeeds but initial inventory is delayed | UI shows Synchronizing and admits no action requiring ready inventory |
| V03 | Object changes during initial inventory | Applied state and continuation token preserve the change without an unobserved gap |
| V04 | Required non-event class is missing or denied | Readiness or affected capability has an explicit reason |
| V05 | Session expires or pool coordinator changes | Client rediscovers and authenticates the verified destination, then resynchronizes |
| V06 | Old event/read/frame completes after reconnect | Current inventory and console ignore the old generation |
| V07 | Old mutation response arrives after reconnect | Journal retains the evidence and reconciles the operation |
| V08 | Event batch is malformed or exceeds limits | No partial apply or token advance. Client reports the fault and resynchronizes |
| V09 | UI observer is slow or closed | Current snapshots remain available and accepted operation results stay queryable |
| V10 | Command queue or journal is full | Submission is refused before dispatch, with a visible error |
| V11 | Server accepts a mutation but its reply is lost | Outcome remains unknown or reconciling. No automatic duplicate mutation |
| V12 | Process exits after dispatch but before task reference is saved | Restart reconciles journal evidence without replaying the intention |
| V13 | Same operation ID is submitted twice | Same intent returns the existing operation. A changed intent is rejected |
| V14 | Task events are absent or task disappears | Polling supplies progress where possible. Missing task alone never establishes success |
| V15 | Cancellation races with completion | UI distinguishes cancellation requested, cancelled, and already completed |
| V16 | Certificate changes on API or console endpoint | Credentials are withheld pending the required trust decision |
| V17 | Advertised console or coordinator redirects elsewhere | Destination identity is checked before forwarding authentication |
| V18 | Keyring is locked or absent | Session-only credentials work and persistence failure is explicit |
| V19 | VM reboot or migration replaces its console | Stale console frames/input stop and the current endpoint is reopened |
| V20 | Metrics fails while XAPI works | Charts show unavailable data while management remains usable |
| V21 | Two connections resolve to the same pool | One inventory owner remains and operations retain the right pool identity |
| V22 | Composite creation fails after creating some resources | Exact owned resources and partial outcomes are visible. Shared or externally changed resources survive |

Inject failures at the request's important boundaries: before admission, after durable
admission, before dispatch, after dispatch, after host acceptance, and after response receipt.
Assert the observed server mutation count and journal result. A frontend error toast is
not sufficient evidence that the host did nothing.

## Console qualification

Test the authenticated CONNECT tunnel before evaluating graphics alone. Verify that the
selected RFB library accepts that stream and negotiates the actual host's supported features.
Use a disposable guest with a visible input test application.

- Resize the guest display and the client window. Verify pointer coordinates at each scale.
- Enter text with the target US and non-US layouts. Exercise modifier combinations,
  QEMU extended keys where supported, and a deliberate Ctrl+Alt+Delete action.
- Hold a key or mouse button, change focus, close the tab, and break the connection.
  Confirm that input does not remain held in the guest.
- Exercise negotiated framebuffer, cursor, and resize encodings. Use a compatible baseline
  encoding if an extension is unavailable and record the result.
- Verify clipboard opt-in and refusal without logging clipboard text or session cookies.
- Open multiple consoles while issuing read-only API calls. Console traffic must leave
  operation state observable and management controls responsive.

## Outage acceptance run

1. Record the disposable target pool, test account, VM UUID, and allowed power operations.
2. Make XO unavailable to the workstation while retaining direct management access to the hosts.
3. Cold-start the installed application. Use a saved member endpoint or enter one manually.
4. Authenticate directly, wait for current inventory, and locate the test XO VM by its UUID.
5. Start the VM if it is stopped and follow the server operation result.
6. Open the embedded console and demonstrate actual keyboard and pointer input.
7. Restore XO availability. Confirm that the client did not use it at any stage.
8. Save the sanitized outcome and restore the disposable lab state.

Use network observations or a controlled endpoint allowlist to prove the connection path.
The fact that the UI still opens is not proof of independent operation.

## Release checks

Install on a clean Arch development target and Ubuntu 24.04 x86-64 baseline. Record the
Wayland compositor and X11 window manager actually used. Check hardware rendering and one
software Vulkan setup, theme contrast, keyboard-only operation, HiDPI, file chooser, and Orca.
Document packaging dependencies before choosing an AppImage, Flatpak, or distribution package.

For performance claims, record dataset shape, pool count, visible rows, update rate, console
resolution and frame rate, hardware, renderer, build profile, and measurement method. Distinguish
host response latency, decoding, core processing, and UI presentation. Upstream benchmark
numbers are not project results. Establish budgets only after the first representative run.

Check formatting, Clippy, unit/protocol tests, deterministic UI fixtures, and release builds
once the workspace exists. Release reports link the actual CI run and lab evidence.
Do not add empty CI jobs or placeholder tests that claim these checks already pass.

A release starts from a tag. Pushing `vX.Y.Z` runs the release workflow, which requires
the tag to name the workspace version and the [changelog](../CHANGELOG.md) to have an
entry for it that is no longer marked in development, repeats the check script on the
tagged commit, builds the archive and AppImage from it and leaves a draft release holding
them and their checksums. A person publishes the draft. The workflow has not run yet: no
tag has been pushed.


## Expanded parity qualification

The retained workflow checklist is [PARITY-IMPLEMENTATION.md](PARITY-IMPLEMENTATION.md).
The [disposable nested KVM harness](../lab/README.md) prepares isolated resources and has
an offline ownership/XML test suite. Running that suite does not create an XCP-ng lab or
qualify a host workflow. Real tests require a designated disposable environment and record
exact host versions and permitted actions. Never substitute a production pool for this gate.

The ordinary check script also runs the vendored RFB wire tests and isolated fake Secret
Service fixtures. It renders only synthetic scenes, into `artifacts/ui/`, which CI keeps
as the artifact `ui`. User-provided screenshots and private
inventories must not appear in artifacts, fixtures, reports or releases.

## Live lab scenarios

`crates/xenadmin-core/tests/lab_live.rs` holds ignored integration tests that drive the
core facade against the disposable lab. They take every target from the harness manifest
and refuse to run without it, read the lab-only root password from a file, and write
sanitized JSON evidence under the manifest directory, never into the repository:

```sh
XENADMIN_LAB_MANIFEST=/var/lib/libvirt/images/xenadmin-rs-lab/lab.json \
XENADMIN_LAB_PASSWORD_FILE=/path/to/lab-root-password \
XENADMIN_LAB_GUEST_IMAGE=/path/to/alpine-virt-x86_64.iso \
cargo test -p xenadmin-core --test lab_live -- --ignored --test-threads=1
```

`XENADMIN_DEV_RPC_TRACE=1` prints failed XAPI method names and sanitized error codes to
stderr during these runs; it never prints parameters, tokens or payloads. It also prints,
for each batch of events a session receives, how many events of each class and kind it
held and over how many objects, and a line whenever the session's check of its inventory
against the hosts finds it out of step and reads it again; never a record, a value or a
reference. Scenario results
and host findings are recorded in the dated qualification reports under `reports/`.

The native desktop can be driven in the same lab without a physical seat: a nested
compositor with a headless output, AT-SPI for discovery and activation, the compositor's
physical key injection for typing, and a client of the compositor's virtual-pointer protocol
for motion, buttons and the wheel. Virtual keyboard tools with their own keymap do not
produce valid QEMU scancodes, and the compositor's own button dispatcher delivers a click at
a window's corner rather than at the pointer. Guest-side proof comes from the guest: have it
capture its own input devices and report the bytes to a listener on the isolated bridge.
Headless output of the GPUI Kit front end (scene renders, tests on the Kit's test platform,
and the headless runs of the shell against the lab) never substitutes for these runs, and a
nested compositor still does not establish real seat, clipboard or Orca behaviour. The
driven sessions recorded before 2026-11-19 were of the Rune interface; the harness's
element lookups were written for that interface's accessibility tree and have not been
run against the GPUI Kit front end. Its driven sessions of 2026-11-19 and 2026-11-20 used a driver of
their own, outside the repository: a compositor on the display of a virtual machine, keys and
pointer injected at the machine's devices, the accessibility bus read to find controls,
and a lab guest reporting its own input devices
([first report](reports/2026-11-19-gpui-vm-session.md),
[second](reports/2026-11-20-gpui-merged-tree.md)). That is a driven session in the sense
of this section and, like a nested compositor, establishes nothing for a real seat with a
person (gate G3 of the [migration plan](GPUI-MIGRATION.md)).

`crates/xenadmin-desktop/tests/lab_live.rs` holds ignored tests that run the front end
itself against the lab without a window: the application as `app::start` wires it (the
real core, store, shell and console windows) on the Kit's headless platform, acting
through presses, typing and commands on the elements a user has, with a PNG and an element
tree of every step written beside the lab's other evidence, outside the repository. Run
one at a time, on an empty temporary state directory:

```sh
XENADMIN_LAB_MANIFEST=/var/lib/libvirt/images/xenadmin-rs-lab/lab.json \
XENADMIN_LAB_PASSWORD_FILE=/path/to/lab-root-password \
XENADMIN_STATE_DIR="$(mktemp -d)" \
cargo test -p xenadmin-desktop --features scenes --locked --test lab_live -- \
  --ignored --exact lab_g2_add_server_and_follow_the_host --nocapture --test-threads=1
```

This is live-host evidence of the model, the shell and the core working together, and of
what the renderer drew. It is not evidence of a window: no compositor, seat, clipboard,
portal or screen reader takes part, and the released binary is not what runs.

A private D-Bus session bus isolates a throwaway Secret Service provider from the operator's
own keyring, but it shares the user's runtime directory: a keyring daemon and an
accessibility launcher started on it bind the same socket paths as the session's own and
displace them. Give such daemons their own runtime directory, and do not use `--replace`.
