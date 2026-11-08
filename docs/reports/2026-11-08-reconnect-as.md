# Session-only reconnect as another account

The Connections manager now offers **Reconnect as another account** for a connected
pool. Security inspection also links to this flow when the session is not a local
superuser. The review captures the saved profile, pool UUID and current connection
generation; a changed profile or connection requires reopening it.

The entered account and password apply only to the new session. The saved username,
keyring mode/entry, auto-connect choice, endpoint and TLS policy remain unchanged.
An optional read-only session checkbox is explicit. The sidebar and Connections
manager identify the session account separately from the saved account.

Reconnection cancels pending same-pool credential loads, removes their transient
pending saves and closes the same-pool aliases before opening the replacement
connection. It retains other pool connections. The active view switches and its
console closes. Existing mutation journal recovery still owns interrupted outcomes;
no infrastructure command is issued by this flow. Failed authentication does not
silently reconnect as the saved account.

Three new synthetic loopback/Rune tests cover account delivery and read-only mode,
unchanged saved settings and absent keyring writes, same-pool alias closure with
other pools retained, stale-profile rejection/password clearing, and compact review
footer behavior. No real host, account or desktop keyring was accessed. General
per-method RBAC eligibility remains open; this flow changes authentication rather
than temporarily bypassing the server's permission checks.

Validation: all 107 desktop tests passed, including three new reconnect-as tests.
Strict workspace/all-target Clippy, formatting and diff whitespace checks passed.
The tests exercise Rune's compact layout and interaction engine; no additional
native compositor or live authentication qualification is claimed.
