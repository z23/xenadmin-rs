# Console lifecycle recovery

The open console remains pinned to its pool and VM UUID. Current inventory supplies the
resident host, connection generation and optional VM start marker. A changed host, boot
marker or generation closes the old stream, releases held input and schedules a reconnect
through the existing authenticated connection. A VM returning from a non-running state is
also eligible. Missing boot metadata is not itself a reboot.

Stale inventory, missing VM/host, a foreign pool, a template/control domain or read-only
connection cannot reopen the guest console. The last image may remain visible while waiting,
but input is gated. Clipboard and screenshot callbacks are fenced by a new console epoch
when a stream closes or reopens. Ordinary table selection never changes the console target.

Stream failure and lifecycle changes use five retries with 1/2/4/8/16-second delays. A brief
successful connection does not replenish the budget; 15 seconds of stable connection or a
new authoritative host/boot/session does. A late connected frame on a closed stream cannot
enable input or restart exhausted retries. Reconnect admission failures also consume the
budget. An automatic-reconnect toggle and manual Reconnect remain available. Existing
certificate policy applies to every new tunnel; this workflow changes no trust settings.

The previous journal-only local-migration follower is replaced by current inventory and
stream state. This covers operations issued by another client and reboot/session recovery
without interpreting an uncertain task result as successful. Cross-pool routing remains a
separate explicitly selected destination workflow.

## Verification

Seven focused model/Rune fixtures cover external host moves, changed boot/session identity,
missing/stale/halted/read-only targets, same-UUID foreign pools, missing optional metrics,
paused guests, bounded/flapping failure, synchronous admission refusal, late closed-stream
frames and the reconnect toggle at wide/compact sizes. All 148 desktop tests and strict
workspace/all-target Clippy passed. Wide and compact native synthetic renders were inspected;
the compact reconnect label was shortened to fit. Existing authenticated RFB protocol fixtures remain
unchanged. Native real-guest reboot/migration/input checks await the prepared lab.
