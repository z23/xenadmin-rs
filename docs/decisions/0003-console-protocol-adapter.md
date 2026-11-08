# Console protocol adapter

XenAdmin keeps the Rune UI independent of the RFB wire protocol. The Rust console
worker owns the authenticated host tunnel, capability negotiation, input release,
framebuffer, cursor and in-app clipboard state. A console's pool and VM identity
remain fixed while it is open. Opening or reconnecting assigns a fresh UI epoch
so an asynchronous local clipboard read cannot deliver text to a replacement
console session.

## Pinned protocol library

The workspace patches `vnc-rs` 0.6.0 to `vendor/vnc-rs`. It preserves the upstream
MIT and Apache-2.0 licenses and the existing bounded queue, decoder-limit and
extended desktop-size fixes in the pinned source. Dependency versions are not
upgraded. The additional adapter changes are:

- Baseline RFB clipboard bytes use ISO-8859-1 in both directions. Text outside
  that character set is refused instead of being replaced or sent as invalid
  UTF-8 bytes.
- Extended Clipboard supports capability, request, peek, notify and provide
  messages. UTF-8 is enabled only after server capability negotiation. Compressed
  payloads have bounded wire and decoded sizes; the app accepts text up to
  256 KiB. OS clipboard reads and writes require explicit user actions. Clipboard
  contents are excluded from protocol trace messages.
- QEMU Extended Key Events are advertised and used only after the server's
  matching pseudo-rectangle. Physical keys use the protocol's XT make-code
  encoding, including its Pause and Print Screen exceptions. Servers that do
  not negotiate this extension continue receiving ordinary X11 keysyms.
- Cursor updates preserve their alpha masks and hotspots. Legacy and extended
  desktop-size updates rebuild the framebuffer only when dimensions change.

The protocol reference is the [community RFB specification](https://github.com/rfbproto/rfbproto/blob/master/rfbproto.rst).
Fixtures cover baseline Latin-1 bytes, UTF-8 round trips, invalid and oversized
clipboard payloads, QEMU packet bytes, and rejection of unadvertised extensions.
The vendored library tests run independently with
`cargo test --manifest-path vendor/vnc-rs/Cargo.toml --lib --offline`.

## Rune behavior and remaining gaps

The pinned Rune revision exposes fullscreen, local wheel coordinates, physical
keys and middle-button dragging. Its secondary-button callback reports a complete
right click on release; it does not expose a matching down/up drag pair. The UI
therefore offers a visible **Hold right button** control for right-drag gestures,
while normal right clicks remain one-shot. Focus loss and **Release input** clear
held keys and buttons. Native secondary-button dragging requires a future Rune
adapter change.

Fit and actual-size views, native fullscreen, explicit clipboard controls,
reconnect and special-key buttons are implemented in the current window. Separate
console windows and persistent multiple-console tabs remain feature gaps. These
fixtures do not qualify behavior against a production host or guest OS.
