# Pinned console protocol dependency

This is vnc-rs 0.6.0 from the local pinned Cargo source, including its existing
bounded queue, decode-limit, and desktop-resize fixes. Upstream licenses are
preserved. No dependency version is upgraded.

XenAdmin additions implement the RFB protocol's Latin-1 baseline clipboard,
negotiated Extended Clipboard UTF-8 messages with bounded decompression, and
QEMU Extended Key Events. Negotiation and explicit clipboard consent are handled
by the XenAdmin console worker. Clipboard payloads are excluded from tracing.

The client's event receiver is held outside its control mutex and closure is
tracked atomically, so `recv_event` can wait for the server while `input`,
`resize_desktop` and `close` proceed. `VncClient::framebuffer_updates()` counts
the `FramebufferUpdate` messages whose rectangles have all been queued, which
lets the XenAdmin console worker ask for the next update at a frame boundary
instead of on a timer. `VncEvent` is unchanged.

Protocol reference: https://github.com/rfbproto/rfbproto/blob/master/rfbproto.rst

Example-only dev dependencies are omitted and example doctests disabled so the
pinned library wire fixtures run offline. The example remains in upstream docs.
