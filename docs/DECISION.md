# Choose one executable with a separate application core

## Problem

The application must manage XCP-ng directly when XO is unavailable. Rune introduces a UI
thread and GPU lifecycle, while XAPI connections, event continuity, and server operations
need explicit ownership. A desktop crash can interrupt local observation after a host has
already accepted an operation.

## Usage and shape

The desktop connects a profile, watches its inventory, submits typed intentions, and opens
console sessions through one facade. [Architecture](ARCHITECTURE.md#callers-view) owns the
usage and signatures. The core hides XAPI records, sessions, tokens, reconnects, and task
reconciliation. It exposes observable state and operation results. Rune stays in the desktop.

## Alternatives considered

Two independent sketches compared the whole deployment shape:

- Candidate A uses one executable, a background runtime, and one state owner per pool.
- Candidate B uses a separately supervised systemd user service with private Unix-socket IPC.
  The service can continue tracking host tasks when the desktop closes or crashes.

The service hides host protocol behavior but adds process discovery, IPC authorization,
subscription resumption, version compatibility, and service/keyring lifecycle. Those concerns
do not advance the first direct-access recovery workflow. One executable hides the same
XAPI complexity behind a smaller application boundary.

Scores use 1 for weakest and 5 for strongest. These are design judgments, not measurements.

| Criterion | Candidate A | Candidate B |
| --- | --- | --- |
| Reconnect and uncertain-write correctness | 5 | 5 |
| Application API hides protocol complexity | 5 | 4 |
| Rune responsiveness and integration | 5 | 4 |
| Independence from XO and other infrastructure | 5 | 5 |
| Implementation size for the first usable release | 5 | 2 |

## Synthesis decision

Use candidate A. Adapt candidate B's explicit duplicate-operation-ID semantics and
write-before-dispatch journal marker. Both candidates require fresh-state admission,
task reconciliation, generation isolation, and conditional compensation.

Keep the journal in the in-process core. Server tasks can continue after the application
exits; the journal lets the next session reconcile them. Independent local polling after
desktop exit is a deferred requirement. A remote service would add an availability dependency
that conflicts with the intended recovery use.

Review checked both candidates against shallow forwarding layers, leaked wire types,
split state ownership, hand-maintained duplicate invariants, and importable internals.
The selected design keeps the ordinary call path within desktop, pool owner, and XAPI adapter.

The configured architecture runners were GPT-6 Astra and GPT-6.1 Sol, both at medium effort.
The parent compared the complete candidates against the rubric. The pstack cross-family
judge step could not run because the configured pool and available session models belong
to the GPT family. No independent cross-family verdict is claimed.

## Tradeoffs accepted

- A desktop crash stops local polling. The next session reconciles unfinished server work.
- Original implementation size includes a durable operation journal because outage recovery
  must distinguish refused work from work whose result is unknown.
- The initial implementation covers a narrow command set with qualified behavior. Broad
  feature parity follows the usable direct-access workflow.

## Open questions and risks

M0 resolves these questions without changing the product goal:

- Which RFB library passes XCP-ng's authenticated tunnel and keyboard negotiation checks?
- Which machine-readable API export matches the chosen host lineage and supports reproducible Rust bindings?
- Which Rune changes, if any, are needed for Wayland dialogs, physical console input, and Orca?
- Which release format works on the target systems, including software-rendered workstations?

## Next implementation step

Build the M0 Rune shell and direct-host qualification tools, then record the actual compatibility results.
