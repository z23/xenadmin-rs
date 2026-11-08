# 0.11.0 development checkpoint

This local Linux build freezes source at `b7a3645`. Retained parity implementation continues; this is not a complete-parity or live-host qualification claim. The disposable lab is prepared offline and live mutation checks remain deferred until virtualization is available.

## Added since 0.10

- Reviewed guest-tools ISO mounting with exact VM, CD-drive and media identities.
- Static management-interface handover and address configuration with separate route verification and uncertain-outcome recovery.
- Private host filesystem backup and selected support-log downloads.
- Optional private cloud-init drafts in VM creation, followed by an explicit configuration-drive review for the exact newly created VM.
- Guarded VM task recovery and temporary recovery boot, with independently tracked boot-setting restoration.

Each feature report records its supported topology, failure behavior and remaining limits. Dynamic management addressing, staged host restore, disk administration, alarms and the other open ledger workflows are not included in this checkpoint.

## Verification

The integrated workspace passed 851 tests: 380 core unit, 236 direct-protocol, 222 Rune, 5 profile, 4 TLS and 4 probe. Strict workspace/all-target Clippy, workspace/all-target compilation, formatting and diff checks passed. The release packaging build succeeded.

All 14 packaging tests passed. Both artifact checksums were verified, and the archive and extracted AppImage were installed successively into an isolated temporary prefix containing a space. The installed probe help command succeeded; both verified releases were retained. Uninstall removed owned application files and preserved a test-state sentinel. Two smoke-script path assumptions were corrected to the actual installed probe launcher and AppImage `usr` directory before completing the checks.

The packaged portable connection dialog at 513×652 and AppImage-installed guest-tools review at 1440×900 were rendered and visually inspected using synthetic inventory. No user installation, live host or private screenshot was changed or published. These checks do not establish actual Wayland/X11 input, accessibility, clean-distribution compatibility or host mutation/data-integrity qualification.

## Immutable local artifacts

- `artifacts/xenadmin-rs-0.11.0-linux-x86_64.tar.gz`: SHA-256 `82288ffa56d26f89ee0fd0be6a45c0826ae9e89e49788790d5ae9fabd5bcc358`.
- `artifacts/xenadmin-rs-0.11.0-linux-x86_64.AppImage`: SHA-256 `b90834adbeec504ef4a501d5bb0f0041426aaa5198b445fe242ce8d89afecaff`.

These are local artifacts. No GitHub release, tag or binary upload was created.
