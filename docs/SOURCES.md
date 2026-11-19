# Planning sources

Checked on 2026-11-07. Source behavior and design choices are distinguished below.
Local build, rendering, and protocol-fixture results are in the
[alpha report](reports/2026-11-07-alpha.md). Live-host results exist for XCP-ng 8.3.0 only,
in the [live qualification report](reports/2026-11-09-lab-qualification.md); every other
host version remains unverified.

## Reference application

The reference revision is `35dc091c5b62ab3b22449bb7a89cfcf84a6600d4`. Planning used a local
read-only checkout at `~/src/xenadmin`; that checkout is not present on the current
workstation, so the links below are the authority.

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
  governs any reuse of its source. The current Rust implementation is original work.

These paths establish behavior to preserve. They do not establish that old TLS defaults,
fixed reconnect delays, or UI-thread cache mutation are appropriate for the new client.

## GPUI Kit and GPUI

The front end is built with GPUI Kit. The versions below are what `Cargo.lock` holds and
what `cargo tree -p xenadmin-desktop` links on Linux; all come from the crates.io registry,
and the lock file records each one's checksum.

`gpui-kit` is pinned at exactly `0.9.1` (`gpui-kit = "=0.9.1"` in the workspace manifest;
published 2026-11-09). Its default features are `component` and `assets`. `gpui-fast`,
`tree-sitter`, `syntect`, `speech` and `decimal` stay off; the lock file lists the
`gpui-fast` crates (0.1.15) because the Kit names them as an optional dependency, and
nothing links them. The Kit depends on the GPUI crates at exactly `0.3.12`.

| Crate | Version | What it is |
| --- | --- | --- |
| [`gpui-kit`](https://crates.io/crates/gpui-kit/0.9.1) | 0.9.1 | The facade. Application code reaches GPUI as `gpui_kit::*` ([repository](https://github.com/longbridge/gpui-kit)) |
| [`gpui-component`](https://crates.io/crates/gpui-component/0.9.1), `gpui-component-macros` | 0.9.1 | The Kit's component library and its macros |
| [`gpui-base`](https://crates.io/crates/gpui-base/0.9.1) | 0.9.1 | A crate of the Kit beneath the facade |
| [`gpui-kit-assets`](https://crates.io/crates/gpui-kit-assets/0.9.1) | 0.9.1 | The Kit's bundled assets (icons) |
| [`gpui-pre`](https://crates.io/crates/gpui-pre/0.3.12) | 0.3.12 | The GPUI snapshot. `Keystroke` carries modifiers, key and key_char, and no physical key code |
| [`gpui-pre-linux`](https://crates.io/crates/gpui-pre-linux/0.3.12) | 0.3.12 | The Wayland and X11 backend. Its public crate root exports `current_platform` and `linux_platform`, and no `current_headless_renderer` (the scene tool takes the headless renderer from the Kit's `platform` module) |
| [`gpui-pre-wgpu`](https://crates.io/crates/gpui-pre-wgpu/0.3.12) | 0.3.12 | The renderer, on `wgpu` 30.0.1 |
| `gpui-pre-platform`, `-macros`, `-collections`, `-refineable`, `-derive-refineable`, `-scheduler`, `-shared-string`, `-sum-tree`, `-util`, `-http-client`, `-ztracing`, `-ztracing-macro` | 0.3.12 | The rest of the GPUI snapshot that the three above bring |

Beside them the front end links `accesskit` 0.24.1 with `accesskit_unix` 0.22.1 and
`accesskit_atspi_common` 0.19.1 (the accessibility tree and its AT-SPI adapter),
`ashpd` 0.13.13 (desktop portals), `cosmic-text` 0.19.0 (text), `wayland-client` 0.31.15
and `xkbcommon` 0.8.0.

What was reviewed and what was not. The Kit's and GPUI's published sources at these
versions were read for the [migration plan](GPUI-MIGRATION.md), and the spike reports
(`reports/2026-11-14-gpui-spike-s0.md` to `-s5.md`) record what the pin did and did not
show. Since then the pin has built the client, passed the front end's tests on the Kit's
headless test platform, rendered every scene off screen, and carried the headless runs
against the lab of 2026-11-19 ([report](reports/2026-11-19-gpui-live-headless.md)). It has
not presented the client's window on a compositor. The one piece of this pin shown on a
real seat is the spike's second Wayland keyboard listener, on Hyprland 0.56.2
([decision 0006](decisions/0006-console-key-codes.md)), where GPUI also delivered no
pointer events to the probe and fullscreen and the clipboard were not shown. No notice
review of the GPUI crates beyond the collector's run (`scripts/collect-notices.py`, which
copies each package's licence files and supplies a standard text where a package ships
none) has been done.

A Kit upgrade is a reviewed change of its own: update the pin, this table and the lock
file together, and re-run the checks.

### Rune (history)

Every release up to 0.19.0 was built with Rune at commit
`f3d04f3f2069d177d04c963b0ba9f6b2065e46eb`
([core documentation](https://github.com/anomalyco/rune/blob/f3d04f3f2069d177d04c963b0ba9f6b2065e46eb/crates/rune/README.md),
[component documentation](https://github.com/anomalyco/rune/blob/f3d04f3f2069d177d04c963b0ba9f6b2065e46eb/crates/rune-kit/README.md)).
The crate and the pin were removed at the cutover and nothing in the workspace depends on
Rune any more (`scripts/check.sh` fails if something does). The live-lab finding about its
accessibility tree (children of a scroll viewport that stay listed but unregistered on the
AT-SPI bus after a resize or scroll, finding 6 of the
[live qualification report](reports/2026-11-09-lab-qualification.md), with
`accesskit_unix` 0.24.1 of that build) belongs to that front end; whether the GPUI Kit
front end's tree behaves on the bus was not examined, because no AT-SPI client and no
screen reader has been run against it.

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

The workspace pins Rust 1.99.1 and the Kit version above (the alpha pinned the Rune
revision named above), and uses the vendored `vnc-rs` 0.6.0
through a custom stream-based CONNECT adapter
([console adapter decision](decisions/0003-console-protocol-adapter.md)). On the XCP-ng
8.3.0 lab the adapter delivered guest and control-domain frames through the authenticated
tunnel and carried keyboard input, as keysyms from the harness and as physical keys sent
as QEMU extended key events from the desktop (the Rune desktop of that time; the GPUI Kit
front end's keys have reached a lab guest only through the Kit's test key events, headless),
and pointer position and buttons that a guest read back exactly; JSON-RPC login, coordinator redirects and the `event.from` bootstrap run
against those hosts in every live scenario. Non-US layouts, guest framebuffer resize, cursor
shapes, the wheel's direction and clipboard negotiation against a real guest remain
unqualified. The decoder is still handwritten, on top of generated record views. The
API description is what upstream's generator writes for the data model at the lab's
toolstack tag, produced by running that generator (the copy upstream keeps in its
repository is two years older), and bindings are generated from it in
`crates/xenadmin-xapi`, the one crate under upstream's LGPL terms
([decision](decisions/0004-generated-bindings.md)). A read-only lab tool compares the
description with the fields of live records.
No XML-RPC transport exists for hosts without JSON-RPC.

Later reports must distinguish successful tests from untested requirements. Production
access, host versions, credentials, and a lab were not supplied during planning; the
disposable lab was added on 2026-11-09.
