# Verification requirements

## Evidence status

This document specifies acceptance requirements. The [alpha report](reports/2026-11-07-alpha.md)
records local fixture, rendering, and build results. No agent-run live XCP-ng test or lab
mutation has run. The [first user connection failure and its fix](reports/2026-11-07-connection-fixes.md)
are recorded separately. Fixture success does not establish host compatibility.

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
3. Rune headless scenes cover realistic inventory, loading/stale/empty states, action
   availability, errors, operation history, and keyboard navigation. Use stable element keys
   and inspect tree semantics as well as pixels.
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


## Expanded parity qualification

The retained workflow checklist is [PARITY-IMPLEMENTATION.md](PARITY-IMPLEMENTATION.md).
The [disposable nested KVM harness](../lab/README.md) prepares isolated resources and has
an offline ownership/XML test suite. Running that suite does not create an XCP-ng lab or
qualify a host workflow. Real tests require a designated disposable environment and record
exact host versions and permitted actions. Never substitute a production pool for this gate.

The ordinary check script also runs the vendored RFB wire tests and isolated fake Secret
Service fixtures. It renders only synthetic scenes. User-provided screenshots and private
inventories must not appear in artifacts, fixtures, reports or releases.
