# Host refusal of an evacuation through the GPUI front end

2026-11-21. The ignored test `lab_p3_evacuation_refusal` in
`crates/xenadmin-desktop/tests/lab_live.rs` ran the real shell and core, with no
window, on the Kit's headless platform. The lab hosts were already running.
Evidence is `evidence/gpui/1795228393-p3-evacuation-refusal` beside the lab
manifest, outside this repository. The summary says passed, 19.2 seconds,
application `0.20.0`.

Connection and certificate pinning finished at 12.3 seconds. By 17.4 seconds
the host had rejected the maintenance operation, and the member was still
enabled. The guest has no disk, so the host answered before any guest moved.

What the run did:

- Connected to `xenadmin-lab-9e8e4f49-h1` with pinned certificates.
- Created `lab-evac-1795228406` from the template "Other install media", with
  no disk, on the coordinator's local ext repository and the management
  network. Set its home server to `xenadmin-lab-9e8e4f49-h2` and started it.
  Those commands went through `Model::submit_tracked_command`, which is the
  submit a confirmed review uses. The New VM wizard was not clicked.
- The guest was running on `h2`. Enter Maintenance Mode opened the review for
  `h2`. The review text was "Move guest: lab-evac-1795228406
  (fb634d4f-2623-38e0-2c8a-ff981f0ff955)".
- Confirm produced one Rejected resource operation. Its detail was "The host
  reports VMs that prevent evacuation (lab-evac-1795228406: VM_LACKS_FEATURE);
  resolve their migration restrictions before retrying". `h2` stayed enabled.
- Delete, through the same submit, removed the VM. `h2` was still enabled.

Not this run: a window, a hardware GPU, a keyring, the New VM wizard, and an
operation still in flight when a toolstack restarts.
