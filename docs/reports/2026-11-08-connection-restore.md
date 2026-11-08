# Bulk connections and protected startup restoration

This increment implements the remaining bulk controls and startup restoration in retained
rows P01-08 and P01-10. Native Secret Service and live-pool qualification remain open.

## Behavior

- Connect all queues saved-credential lookups for disconnected profiles, with at most four
  in flight. Existing active connections are skipped. Session-only profiles retain manual
  password entry and show a per-profile explanation.
- Each profile can explicitly opt into startup connection when Secret Service storage is
  selected. Startup attempts opted-in profiles once. Missing, locked, denied or unavailable
  keyrings produce per-profile recovery messages without automatic retry.
- Disconnect all cancels queued credential requests and disconnects active pool handles.
  Individual pending requests can also be cancelled. Late results cannot reconnect them.
- Credential requests capture the saved profile and a unique attempt identity. Saved settings,
  including endpoint, username, trust and access mode, must still match at completion. Editor
  loads also verify the current form. Concurrent background results cannot borrow another
  open editor's destination or credentials.
- Keyring writes/removals and profile saving are serialized per profile. Forgetting credentials
  also clears startup connection. No password is serialized into application files.
- Profile document version 1 remains readable with startup restoration off. The next write uses
  version 2, which older clients reject instead of overwriting.
- Demo and headless scene/render invocations suppress startup restoration.

## Verification

The full Cargo workspace test suite passed during implementation. After the final preview
guard and keyring-write serialization changes, all 14 profile controller/Rune interaction
tests passed, including nine new cases, and strict workspace Clippy passed. The core profile
tests cover the new opt-in constraint and version-1 compatibility.

New fixtures cover bounded bulk loading, startup opt-in, locked/missing keyring results,
no automatic retry, settings changes, removal, cancellation, stale attempt IDs, concurrent
completion while another editor is open, credential-forget behavior, and preview isolation.
Rune interaction tests click bulk controls and the startup/keyring checkboxes. All desktop
credential tasks use a fake adapter in tests. The connection fixture uses a loopback listener.

Two synthetic 513×652 GPU renders were inspected: Connections and Saved connection options.
Bulk controls and fixed dialog footers remain reachable, and the longer settings form scrolls.
No user screenshots, real inventories, passwords, actual desktop keyring entries, or production
hosts were used. The renders do not establish native accessibility or real password-manager
compatibility.
