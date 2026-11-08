# Disposable lab acceptance record

Status: prepared, not executed. All targets must belong to the local harness manifest.
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
| Direct recovery | Login through both members with XO unreachable; one pool owner | Coordinator outage yields stale state, then fresh generation; no duplicate writes | Pending |
| Power and console | Start/resume on chosen host; clean and force operations; expected guest state | Wrong state/permission/admin block shows actual reason; target cannot change underneath console | Pending |
| VM settings | CPU/memory/boot changes read back correctly | Concurrent edit is rejected; lost setter reply stays unknown until recheck | Pending |
| Snapshot marker | Snapshot marker A, change to B, restore and read A | Interrupted create/restore never automatically repeats the write | Pending |
| Snapshot clone | New VM and template have distinct UUIDs and preserved marker; VM stays halted | Interrupted conversion keeps the new object visible and recheck performs no write | Pending |
| Snapshot deletion | Selected snapshot and only reviewed exclusive owned disks disappear | Shared/unowned/unselected disks and untouched guest remain; changed ownership stops cleanup | Pending |
| Checkpoint deletion | Saved memory image is removed before record cleanup completes | Changed memory identity or failed cleanup stops remaining steps and names unresolved UUIDs | Pending |
| VM devices | New/attached disks and NICs belong to the intended VM; CD media/size match | Foreign/shared device, changed size, failed attachment and lost response preserve data and partial outcome | Pending |
| Storage/network administration | Resource graph matches host state and reviewed host/network selection | Used SR/network, active management path, permission refusal and partial pool-wide result are explicit | Pending |
| Migration/transfers | UUID/marker/device placement verified at destination; console relocates | Expiry, cancellation and transport interruption preserve known destination and ownership evidence | Pending |
| Host/pool administration | Reviewed disposable host/pool operation changes only intended resources | Member failure, unavailable HA capacity and missing permissions remain explicit | Pending |
| Metrics/tasks/alerts | Values/timestamps/tasks match host evidence and show gaps | Missing samples, failed collection and external tasks remain distinguishable | Pending |
| Connections/restore | Only opted-in saved-credential profiles restore; aliases share one owner | Locked keyring/changed trust/deleted profile cannot consume a late credential result | Pending |
| Linux desktop | Native Wayland/X11 input, clipboard, file selection and accessibility | Resize, focus loss, reconnect and renderer failures preserve target/input safety | Pending |

For later workflows, record the stable row ID from `docs/PARITY-IMPLEMENTATION.md` and
both the successful path and a refusal/interruption path. Capability absence is recorded
as unsupported, not passed. Keep a failed or incomplete scenario pending until evidence
shows its outcome. Fixture tests do not change this live acceptance record.

Cleanup uses the manifest's owned resources only. Wait for shutdown before undefining.
The harness retains disks and evidence; deletion of those files is a separate deliberate step.
