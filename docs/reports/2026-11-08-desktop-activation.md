# Desktop instance activation

Retained P18-11 now has a local activation handoff for repeated launches using the same
state directory. Actual compositor focus behavior remains desktop qualification.

## Behavior

The launcher acquires desktop ownership before loading profiles, restoring credentials,
starting developer inspection or creating a backend. If another process holds ownership,
the new launcher asks that process to activate its existing main window and exits. It
forwards a bounded `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID` when present; Rune's pinned
`AppHandle::activate` applies the platform token. Without a token it requests `open`, which
also restores a minimized window. The compositor decides whether focus is granted.

An activation failure is reported on stderr and returns a failed launch status. It never
starts a competing backend, removes an active owner, changes profiles, or retries host work.
Different explicitly selected state directories remain independent client workspaces.

Preview/render/demo/help invocations bypass live ownership and activation. Their state
constructor does not start a backend, load saved profiles/credentials, or start developer
inspection. This also keeps synthetic render tooling from interfering with a running client.
Secondary console windows use the same quiet construction boundary.

## Local protocol and lifetime

The listener is a separate `desktop-activate.sock` under the owned 0700 state directory,
with mode 0600 and same-user peer checks on both ends. Its versioned JSON protocol accepts
only activation plus an optional token. It cannot query inventory, submit host commands,
execute programs, accept forwarded command-line arguments or change settings. Tokens are
not printed or persisted. Requests are limited to 16 KiB, tokens to 4096 bytes, four
concurrent handlers and 500 ms per exchange. The second launcher waits at most two seconds
for a primary process still creating its listener; it does not retry after sending a request.
Acknowledgement means the focus request was queued, not that a compositor granted it.

The lock rejects symlink/hardlink/non-regular substitutions. Only an owned private stale
socket with a refused connection is reclaimed under the exclusive lock. Shutdown removes
only the same socket inode it created; replacements are preserved.

Ordinary `DesktopInstance` ownership releases on drop. The actual desktop explicitly pins
one cloned close-on-exec lock descriptor for its process lifetime. This is deliberate:
`Backend` uses asynchronous runtime teardown, so dropping window state alone is insufficient
to prove every worker has stopped. The listener and its socket still close normally, while
the OS retains ownership until the process exits. Child programs do not inherit the pinned
descriptor. No backend shutdown behavior or uncertain-write recovery rule is weakened.

The local activation API is independent of the opt-in developer inspection protocol, whose
read-only command set and privacy boundaries remain unchanged.

## Verification

The full workspace passed **952 tests**: 430 core unit, 262 direct protocol, 247 Rune/desktop,
five profile, four TLS and four probe cases. Final focused ownership/protocol tests passed
all nine cases after strengthening oversized-frame refusal. Strict workspace/all-target
Clippy, formatting, diff checks and workspace build passed. Two startup tests verify that
preview/help selection cannot claim live ownership and quiet state has no backend or active
profiles/inspection.

A native Vulkan synthetic preview and `--help` both completed while a separate process held
the same isolated state-directory lock and inspection was requested through the environment.
The state directory retained only its unchanged lock sentinel: no backend, profile or
inspection files were created. The compact preview was visually inspected. Independent
source review identified the runtime-teardown lifetime issue; the process-held descriptor
and gated child-process regression address it.

Local fixtures cover cross-process ownership and activation,
late listener startup, invalid/oversized frames, unsupported commands and versions, false
acknowledgement, a slow client, stale paths, replacement preservation, independent state
directories and process-lifetime lock retention after the UI/listener guard drops.
Real Wayland/X11 activation tokens, minimization and compositor policy remain qualification;
no user's live desktop or host connection is used for these fixture checks.
