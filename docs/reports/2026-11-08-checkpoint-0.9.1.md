# Version 0.9.1 development checkpoint

This package adds reviewed [cross-pool movement/copy](2026-11-08-cross-pool-transfers.md),
[authenticated storage](2026-11-08-authenticated-storage.md),
[NoCloud configuration drives](2026-11-08-cloud-init.md),
[certificate review and proxy routing](2026-11-08-connection-trust.md),
[secondary IPv6 and interface purposes](2026-11-08-network-interfaces.md),
[desktop options and offline help](2026-11-08-desktop-options.md), and
[graph presets and captured resource reports](2026-11-08-graph-presets-reports.md).
It includes the preceding firmware, bulk-power, host-console and console-recovery increments.
Unknown operations retain monotonic interruption fences while explicit rechecks show progress;
private cloud-init plans are bound to the exact public operation and artifact identities.

The [Linux installer and AppImage](2026-11-08-linux-packaging.md) provide verified payloads,
retained releases and ownership-checked removal. Artifact testing caught a custom-prefix
management launcher defaulting to the normal user prefix; 0.9.1 fixes it and tests the
actual installed CLI. The local 0.9.0 artifacts were retained unchanged and superseded.
The notice collector now retains an owned provenance file for crates with no bundled
notice/README and does not inherit an unrelated registry-parent license.

## Verification

- The integrated application passed **713 workspace tests**, formatting, strict
  workspace/all-target Clippy and a native build. The final version-only bump additionally
  passed workspace/all-target checking and release compilation.
- The preceding full check passed **46 vendored console tests**, **8 lab helper tests**
  and 83 synthetic renders. After report/trust integration, eight changed/new wide/compact
  scenes were rendered; their source increments also verified all newly registered scenes.
  These are synthetic checks, not physical desktop or host qualification.
- All **14 packaging tests** passed, including actual custom-prefix launcher status/removal
  and provenance for a notice-less crate.
- Both final artifact checksums passed. The archive installed into a temporary prefix with
  a space in its name; desktop metadata validation, bundled probe startup and compact
  native connection-screen render passed. The AppImage ran through extract-and-run without
  FUSE, rendered the wide connection screen, and installed its payload into the same prefix.
  Both retained payloads were removed by the installed launcher; isolated test state survived.
- The current-artifact development and inspection scripts were exercised against a real
  empty native client in an isolated private state directory. Inspection reported application
  version 0.9.1 and zero connections through its Unix socket. The owned test process was then
  closed. No saved user profiles or host sessions were loaded.

## Local artifacts

| Artifact | SHA-256 |
| --- | --- |
| `artifacts/xenadmin-rs-0.9.1-linux-x86_64.tar.gz` | `6d43bda6aa5e93d7cc3cc3434bf76482dd5e6a076501b8265a1762f8848296bf` |
| `artifacts/xenadmin-rs-0.9.1-linux-x86_64.AppImage` | `381121dfdd9b91fe3da131918240fdd2645994de6e90a29908cde9c48f965a32` |

This remains a development checkpoint with open workflows in the
[retained parity ledger](../PARITY-IMPLEMENTATION.md). It does not include later host BMC,
general-information or network-topology increments. Packages were built on the development
workstation; AppImage wrapping does not establish clean Ubuntu compatibility or lower glibc
requirements. CI artifact generation is configured but this report does not claim a remote
CI result or a published GitHub release.

No production mutations or private screenshot publication occurred. The prepared disposable
lab awaits virtualization; the user has deferred destructive live testing. Real failover,
storage/guest integrity, XO-down recovery, portals, input, accessibility and distribution
qualification remain open.
