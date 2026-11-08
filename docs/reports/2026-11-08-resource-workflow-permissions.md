# Resource workflow permission reviews

This increment extends P13-11 to dedicated resource workflows. The immutable reviewed operation supplies its known write requirements to both the Rune confirmation and the resource worker's guard before any workflow step runs. An explicit missing permission disables confirmation and rejects programmatic submission before the first write. Unavailable permission discovery remains unknown, and the server still checks every request.

## Coverage

- vApp creation, deletion, group power, membership, names/descriptions and per-VM order/delays. Permission review and execution share the same changed-field and method selectors.
- Snapshot schedule creation, configuration, membership, enable/disable, deletion and run-now. Unchanged fields require no extra setters; the VMSS API remains responsible for its host-side workflow.
- Organization metadata across all selected object classes. Every reviewed edit contributes its actual setter, including later objects in a multi-object patch.
- Host power-on configuration and retained-Secret cleanup. IPMI requires both `secret.create` and `host.set_power_on_mode`; Wake-on-LAN and disabling power-on do not invent a Secret-creation requirement. The host handles removal of its old power-on Secret as part of its setter.
- Authenticated storage creation/attachment requires its provisioning method, `secret.create` and the existing temporary-Secret cleanup method `secret.destroy` before creation starts. Separate retained-Secret cleanup also has its own review and guard.
- Pool root-password change requires `session.change_password`, assessed against the captured connection profile's session rather than whichever pool is selected in the main view.
- Pool-wide VLAN creation/retagging and power batches now expose their known methods through the existing resource review. Retagging requires unplug only when preserving a previously detached interface.
- Cloud-init drive preparation checks `VDI.create`, `task.create` and `VBD.create` before creating a disk. HTTP upload authorization remains explicitly unknown; granted RPC methods alone do not prove HTTP access.

The dedicated dialogs retain their existing object, generation, confirmation and synthetic-preview guards. Permission information stays next to confirmation controls, including compact windows. Passwords, user-data and session references are not included in these notices.

## Verification

Protocol coverage exercises a denied later vApp field, VM start delay, snapshot-schedule field or organization-object setter and verifies that no workflow RPC is sent. Authenticated CHAP, mutual CHAP and SMB storage fixtures deny cleanup and verify that no Secret or storage object is created. Other cases cover unchanged fields, optional Secret creation, and the distinction between known cloud-init RPC denial and unknown HTTP authorization. Rune checks exercise captured reviews at 513×652 and 1440×900 and retain synthetic submission refusal.

The workspace passed 1,156 tests (512 core unit, 312 direct protocol, 319 Rune, five profile, four TLS and four host-probe tests), strict Clippy, formatting, whitespace checks and a desktop build. Six synthetic review scenes were rendered at 513×652 and 1440×900; all six compact reviews and representative wide views were visually inspected for legible notices and reachable controls. An independent source review found no blocker. No live infrastructure mutations or private screenshots were used.

## Remaining scope

HTTP transfer authorization, cross-pool workflows with separate authorities, protected map keys and source-dependent steps still need their own accurate capability evidence. Stage-specific management IPv6 requirements remain under the network workstream. Live RBAC behavior and desktop accessibility require the deferred disposable-lab/platform qualification; this increment does not claim them complete.
