# 0.12.0 development checkpoint

The local Linux packages freeze source at `a2a43bd`. Implementation of the retained parity scope continues. This checkpoint does not establish complete parity or live-host qualification; the prepared disposable lab remains offline at the user's request.

## Added since 0.11

- Automatic management addressing, existing bond/VLAN configuration, and reviewed static IPv4 management bond creation/removal with VLAN recreation and certificate-bound recovery.
- Host recovery archive staging, retained crashdump deletion and performance alarm configuration.
- Disk growth, copying and provenance-checked cleanup; selected multi-host storage attachment repair and read-cache policy.
- Optional private snapshot preview capture, upload and content verification.
- Up to eight independent native console windows, with isolated input and recovery state.
- Single-instance desktop activation through private same-user IPC, with ownership held until process exit.

The individual feature reports and [parity ledger](../PARITY-IMPLEMENTATION.md) record supported topologies, failure behavior and outstanding workflows. Guest SSH/RDP launch, HBA provisioning and expanded export validation are subsequent work, outside this source freeze.

## Verification

The combined workspace passed 979 tests: 442 core unit, 266 direct-protocol, 258 Rune, 5 profile, 4 TLS and 4 probe. Strict workspace/all-target Clippy, formatting and diff checks passed. The locked release packaging build succeeded; all 14 packaging tests passed.

Both artifact hashes were checked. The portable archive and extracted AppImage installed successively into an isolated temporary prefix containing a space; both installed probe launchers worked, and both releases remained available. Uninstall removed owned files and preserved a state sentinel. The packaged console manager at 513×652, management VLAN/bond review at 1440×900 and independent console at 513×652 rendered successfully and were visually inspected using synthetic inventory.

No user installation, private screenshot or live host was changed or published. These checks do not establish real Wayland/X11 focus, guest input, accessibility, multi-monitor behavior, clean-distribution compatibility or infrastructure data integrity.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.12.0-linux-x86_64.tar.gz`: SHA-256 `af5eede9b740e45c390309adeb61848d8a02bea60ca93b6b15131fdcd2985746`.
- `artifacts/xenadmin-rs-0.12.0-linux-x86_64.AppImage`: SHA-256 `cdf3d4d615f9aaf9128e0a6657115987eff542575740befe474309cefd040485`.

These artifacts remain local. No GitHub release, tag or binary upload was created.
