# Operations availability UX — 0.2.1

Previously, the VM Details panel omitted every power action absent from the received
allowed-action list. An empty list therefore left an unexplained Operations heading.

The panel now renders all nine power actions, keeping unavailable actions disabled.
Empty availability has an explanation, and stale/disconnected inventory has separate
status text. Eligibility and command admission are unchanged. No force override or
host-side task cancellation was added. The message does not claim to know why the host
did not advertise a power action. Exact per-action diagnostics remain a parity gap.

## Verification

- Rust 1.99.1; Rune remains pinned to `f3d04f3f2069d177d04c963b0ba9f6b2065e46eb`.
- `scripts/check.sh`: formatting, Clippy with warnings denied, all 52 tests, workspace
  build and nine synthetic Rune renders passed.
- The initial sandboxed run could not bind loopback fixture listeners. Re-running the
  same checks with permission to bind local sockets passed; no live host was contacted.
- Inspected the synthetic `unavailable-actions` image at 1440×1200 and its accessibility
  tree. Both status explanations and all nine disabled power buttons were present.
- Release build and packaging passed. Archive checksum and both packaged executables
  were checked against the release outputs.

Artifact: `artifacts/xenadmin-rs-0.2.1-linux-x86_64.tar.gz`.
SHA-256: `f7ac5e83427f863b74ba677ede4f14c2062c60a4f97f8746bfbc6947021add8c`.

These are local fixture/build/render results. No new native-display or live-host
qualification was performed. No operator screenshots or private inventory were copied
into the repository or package. The [parity audit](../PARITY.md) records the full
remaining workflow and acceptance gaps.
