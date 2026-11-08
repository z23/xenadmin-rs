# Independent console windows

An embedded VM or host console can move into an independent native window. Up to eight
can remain open beside the main inventory. The Console windows manager lists their captured
names, pool UUIDs and VM UUIDs, with Show, Close and Close all controls. It remains reachable
at the compact size. Each window retains fullscreen, scaling, clipboard, screenshot, input
release and bounded reconnect controls. Closing a secondary window drops its display stream;
closing the main application closes every console.

The implementation uses the existing pinned Rune `App::window` API, without changing the
framework version. Eight lazy, reusable window slots share one event loop. A secondary window
never initializes a backend, connection profile store, keyring lookup or session restoration.
Detaching transfers the existing console handle and frame state and clones its original
connection handle. Browsing or reconnecting another profile cannot replace that handle.
Recovery remains bound to the captured pool/VM and its current host, boot marker and session
generation. Disconnect/stale-state handling releases input and closes the stream.

Each window owns its frame, pointer/key state, clipboard callbacks, screenshot state and
reconnect budget. Detaching releases held input and changes the console epoch; a pending
file dialog in the old window cannot save a frame afterward. An already-authorized background
PNG writer may finish. Unique slot leases reject stale Show/Close/adoption requests. A slot
cannot be reused until the UI-thread close acknowledgement has released the old stream and
queued the native close; Rune drains that close synchronously before another window handler.

## Verification

Five focused Rune fixtures cover independent target/frame/input state, selection changes,
closing one window, stale callbacks and close-before-reuse, the eight-window limit, pending
screenshot-dialog invalidation and manager navigation at 513×652 and wide sizes. The full
workspace passed 926 tests at default concurrency (412 core units, 256 protocol, 245 desktop,
5 profile, 4 TLS and 4 probe). Strict workspace/all-target Clippy, formatting, diff checks and
the release desktop compile passed.

Synthetic Vulkan renders were inspected for the compact secondary window, the manager and
a desktop composite with two distinct frame buffers. A bounded native Wayland smoke run,
using isolated application state and temporary instrumentation, reported creation and first-frame
readiness for both `console-1` and `console-2`. The instrumentation was removed. No host
connections, guest input, user screenshots or production changes were used.

Live guest input across multiple displays, real clipboard/file portals, mixed-DPI/X11 and
long-lived migration/reconnect behavior still require the prepared lab. Window geometry is
not persisted, and returning a detached stream to the embedded view is not implemented;
close it and open the console again from the main window. Guest SSH/RDP/external tooling
remains a separate retained gap.
