# A snapshot in flight while the coordinator's toolstack restarts

2026-11-21. The ignored test `lab_p3_inflight_toolstack_restart` in
`crates/xenadmin-desktop/tests/lab_live.rs` ran the real shell and core, with no
window, on the Kit's headless platform. The lab hosts were already running.
Evidence is `evidence/gpui/1795228900-p3-inflight-toolstack-restart` beside the
lab manifest, outside this repository. The summary says passed, 509.7 seconds,
application `0.20.0`.

The connection was current again at 34.2 seconds. The rest of the 509.7 seconds
is the lab's quiet period after a coordinator toolstack restart, then a recheck
of the unknown snapshot and a Start the same session refused.

What the run did:

- Connected to `xenadmin-lab-9e8e4f49-h1` with pinned certificates. The guest
  was `lab-guest-1794316526`. It was halted, so the test started it and waited
  until the host allowed a snapshot.
- Take a Snapshot, with the guest's memory, was confirmed for
  `gpui-inflight-1795228913`. At 16.9 seconds the operation was Tracking.
  Its detail was "Waiting for host task; stopping the workflow prevents later
  steps only".
- At 17.0 seconds another administrator restarted the coordinator's toolstack.
  The restart was accepted. The snapshot was still Tracking.
- At 18.9 seconds the window showed "Create memory snapshot • OutcomeUnknown"
  and "The connection was closed; later steps were not submitted. No automatic
  rollback or restart is attempted."
- At 34.2 seconds the connection was current again. The same operation was
  OutcomeUnknown, now "Interrupted snapshot workflow. Recheck saved step
  evidence and inspect the host; remaining steps will not be submitted
  automatically." The snapshot was not in the Snapshots tab. The guest was
  running.
- The scenario then waited until two minutes after the restart. Recheck was
  offered. At 509.7 seconds the snapshot was still OutcomeUnknown. Its detail
  was "Read-only reconciliation finished; no writes were repeated and pending
  shutdown, restore, metadata or startup steps were not submitted". Start in
  that session was Rejected: "Another operation on this VM is still running
  or unresolved. Inspect operation history before submitting another action".
  The guest was halted.
- The summary's `away_after_seconds` and `current_again_after_seconds` were
  both written at the 34.2 second step. The step times above are the ones
  that separate the away frame from the return. The test now records each
  interval when that wait returns.

A second session, `lab_p3_acknowledge_interrupted_snapshot`, used a new state
directory. Evidence is
`evidence/gpui/1795229581-p3-acknowledge-interrupted-snapshot`. It passed in
14.3 seconds. The guest was halted and the operation list was empty. The
unknown snapshot had stayed in the first session. Start was accepted, and the
inventory showed the guest running while that Start was still Tracking.

Not this run: a window, a hardware GPU, and a keyring.
