# Planning sources

Checked on 2026-11-07. Source behavior and design choices are distinguished below.
No runtime compatibility check has run for this project.

## Reference application

The local read-only checkout is `~/src/xenadmin` at
`35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`.

- [Connection worker](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/Network/XenConnection.cs#L1135)
  separates event and command transports, warms the cache, and handles reconnection.
- [Object downloader](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/XenObjectDownloader.cs#L69)
  fetches roles separately and bootstraps event-covered inventory with an empty `event.from` token.
- [Session duplication](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/XenAPI-Extensions/Session.cs#L51)
  distinguishes additional transport objects from additional authentication sessions.
- [Hosted console](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenAdmin/ConsoleView/XSVNCScreen.cs#L1066)
  resolves the advertised console and opens an authenticated CONNECT stream.
- [CONNECT implementation](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenModel/XenAPI/HTTP.cs#L698)
  sends the console path and a session cookie before RFB negotiation.
- [RFB implementation](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/XenAdmin/VNC/VNCStream.cs#L115)
  contains protocol and encoding behavior to qualify in a replacement library.
- [Reference license](https://github.com/xcp-ng/xenadmin/blob/35dc091c5b62ab3b22449bb7a89cfcf84a6600d4/LICENSE)
  governs any later reuse of its source. This planning package contains original prose.

These paths establish behavior to preserve. They do not establish that old TLS defaults,
fixed reconnect delays, or UI-thread cache mutation are appropriate for the new client.

## Rune

Reviewed commit: `f3d04f3f2069d177d04c963b0ba9f6b2065e46eb`.
It is a candidate dependency until M0 builds and exercises it.

- [Core documentation](https://github.com/anomalyco/rune/blob/f3d04f3f2069d177d04c963b0ba9f6b2065e46eb/crates/rune/README.md)
  describes frame updates, background requests, Linux dependencies, native dialogs,
  accessibility, and deterministic headless tools.
- [Component documentation](https://github.com/anomalyco/rune/blob/f3d04f3f2069d177d04c963b0ba9f6b2065e46eb/crates/rune-kit/README.md)
  describes virtualized trees and tables and the desktop shell.
- [Core manifest](https://github.com/anomalyco/rune/blob/f3d04f3f2069d177d04c963b0ba9f6b2065e46eb/crates/rune/Cargo.toml)
  enables desktop backends and the AccessKit Unix adapter.

The core README's old "Differences from PLAN.md" paragraph contradicts its newer
accessibility and scrolling sections. The manifest and current modules support the newer
description. Verify behavior in M0 and M3 rather than relying on either paragraph alone.
The documented incoming Wayland file-drag gap requires a working file picker for imports.

## Host APIs

- [XCP-ng release history](https://docs.xcp-ng.org/releases/)
  lists XCP-ng 8.3 as the current LTS. Selecting it as the initial target is a project decision.
- [XCP-ng API overview](https://docs.xcp-ng.org/management/manage-locally/api/)
  identifies XAPI as the host management API. Its general recommendation to use XO is
  intentionally inapplicable to this project's explicit direct-access outage requirement.
- [XenAPI transport, sessions, and coordinator behavior](https://docs.xenserver.com/en-us/xenserver/developer/sdk-guide/using)
  describes JSON-RPC, session lifecycle, coordinator redirects, and object queries.
  Newer XenServer documentation does not by itself establish an XCP-ng version's support.
- [Pinned upstream API description](https://github.com/xapi-project/xen-api/blob/7b2387956900b8a0e1c6982d9e4b2aa515f18fc4/ocaml/idl/datamodel.ml)
  defines `event.from` parameters and returned token/batch behavior. This upstream revision
  is a research reference, not the selected 8.3 binding schema.
- [SDK generator](https://github.com/xapi-project/xen-api/tree/7b2387956900b8a0e1c6982d9e4b2aa515f18fc4/ocaml/sdk-gen)
  is the starting point for selecting a matching machine-readable binding input in M0.
- [RRD metrics](https://xapi-project.github.io/xen-api/metrics.html)
  documents the separate metrics path and format negotiation.

## Decisions requiring runtime evidence

M0 must select the final Rune revision, Rust toolchain, API export/version input, RFB
library, and any custom CONNECT transport. It must verify JSON-RPC and event bootstrap
against the designated XCP-ng lab. No existing Rust SDK is assumed complete or compatible.

Later reports must distinguish successful tests from untested requirements. Production
access, host versions, credentials, and a lab were not supplied during planning.
