# Disposable lab acceptance record

Status: first run recorded on 2026-11-09 in the
[live qualification report](../docs/reports/2026-11-09-lab-qualification.md); the Result
column summarises it. All targets must belong to the local harness manifest.
Use three hosts: two in the primary pool, one in a separate pool for transfer tests.
The isolated network has no forwarding. Each host has a separate disposable data disk.
Use lab-only credentials and synthetic guest names. No existing production VM is a target.

Record the application commit/version, installer SHA-256, host/API versions, storage
driver, workstation kernel, QEMU/libvirt versions, manifest ID and date with each run.
Create one disposable guest with a persistent text marker and a separate guest that must
remain unchanged. Save before/after UUIDs and marker hashes locally. Do not add passwords,
session references, real inventories or private captures to the repository.

| Scenario | Required observation | Failure/interruption observation | Result |
| --- | --- | --- | --- |
| Direct recovery | Login through both members with XO unreachable; one pool owner | Coordinator outage yields stale state, then fresh generation; no duplicate writes | Login through all three hosts and the native desktop passed with no XO present; coordinator power loss yielded stale state within 31 s, a fresh generation on return and no duplicate writes (finding 7 records the host-side loss of a recent write) |
| Power and console | Start/resume on chosen host; clean and force operations; expected guest state | Wrong state/permission/admin block shows actual reason; target cannot change underneath console | Power matrix and console input passed; unavailable actions were reported, not attempted; chosen-host start, block reasons and console target safety pending |
| VM settings | CPU/memory/boot changes read back correctly | Concurrent edit is rejected; lost setter reply stays unknown until recheck | Description, vCPU and memory edits read back correctly; boot changes and both interruption cases pending |
| Snapshot marker | Snapshot marker A, change to B, restore and read A | Interrupted create/restore never automatically repeats the write | Passed (A, B, restore, A read back through the guest); a create interrupted by a client restart was recovered as unknown and rechecked to Succeeded with one snapshot and no repeated write; interrupted restore pending |
| Snapshot clone | New VM and template have distinct UUIDs and preserved marker; VM stays halted | Interrupted conversion keeps the new object visible and recheck performs no write | Distinct UUIDs and disks, marker A read from the booted clone, VM halted after creation; interrupted conversion pending (finding 8 fixed the withheld Start) |
| Snapshot deletion | Selected snapshot and only reviewed exclusive owned disks disappear | Shared/unowned/unselected disks and untouched guest remain; changed ownership stops cleanup | Six snapshots with exclusively owned disks deleted and verified absent; shared-disk and changed-ownership cases pending |
| Checkpoint deletion | Saved memory image is removed before record cleanup completes | Changed memory identity or failed cleanup stops remaining steps and names unresolved UUIDs | A checkpoint of the running tool-less guest was created and deleted with its disks, leaving no memory image on the pool; the failure cases are pending |
| VM devices | New/attached disks and NICs belong to the intended VM; CD media/size match | Foreign/shared device, changed size, failed attachment and lost response preserve data and partial outcome | Imported image attached and a blank data disk added during provisioning; scratch-disk create, rename, reposition, detach, grow, copy, reattach, owned deletion and a rejected non-owning deletion passed on the halted guest; NIC add and remove and CD drive with the host's tools image passed; changed size, failed attachment and lost response pending |
| Storage/network administration | Resource graph matches host state and reviewed host/network selection | Used SR/network, active management path, permission refusal and partial pool-wide result are explicit | Internal network create, MTU and rename, NIC add and remove, and deletion passed; local repository probe, create, rescan, detach, reattach and destroy passed on the standalone host; shared storage and the failure cases pending |
| Migration/transfers | UUID/marker/device placement verified at destination; console relocates | Expiry, cancellation and transport interruption preserve known destination and ownership evidence | Same-pool migration with both disks relocated and the console reopened on the destination passed; a cross-pool copy to the standalone host verified new identities and read marker A from the booted copy, halted cross-pool moves there and back verified source removal and placement, and XVA and OVA exports were imported into the other pool with verified identities; interruption cases pending |
| Host/pool administration | Reviewed disposable host/pool operation changes only intended resources | Member failure, unavailable HA capacity and missing permissions remain explicit | Maintenance, reviewed ejection and reviewed join passed with the other members untouched; pool rename (including a pool XCP-ng left unnamed), member disable and enable, a member toolstack restart and a member reboot through maintenance passed; failure cases pending |
| Metrics/tasks/alerts | Values/timestamps/tasks match host evidence and show gaps | Missing samples, failed collection and external tasks remain distinguishable | Guest and host RRD series at a five-second step passed after the non-finite JSON fix; gaps, tasks and alerts pending |
| Connections/restore | Only opted-in saved-credential profiles restore; aliases share one owner | Locked keyring/changed trust/deleted profile cannot consume a late credential result | Pending |
| Linux desktop | Native Wayland/X11 input, clipboard, file selection and accessibility | Resize, focus loss, reconnect and renderer failures preserve target/input safety | Nested Wayland session: keyboard input to the guest and AT-SPI activation passed; resize exposed the AT-SPI clipping finding; X11, clipboard, file selection, pointer, Orca and the failure cases pending |

For later workflows, record the stable row ID from `docs/PARITY-IMPLEMENTATION.md` and
both the successful path and a refusal/interruption path. Capability absence is recorded
as unsupported, not passed. Keep a failed or incomplete scenario pending until evidence
shows its outcome. Fixture tests do not change this live acceptance record.

Cleanup uses the manifest's owned resources only. Wait for shutdown before undefining.
The harness retains disks and evidence; deletion of those files is a separate deliberate step.
