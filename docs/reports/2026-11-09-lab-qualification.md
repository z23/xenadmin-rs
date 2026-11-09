# Disposable lab qualification, first live run

This report records the first live XCP-ng runs of the client. Each scenario names the
ledger rows it exercises and separates the observed host behaviour from what remains open.
Raw evidence (sanitized JSON written by the live harness) stays outside the repository.

## Identity

- Application: workspace version 0.17.0, development branch after `86d0565` (CI deadline
  scaling and lab harness changes merged to main).
- Lab: three nested KVM guests from the [harness manifest](../../lab/README.md), manifest
  `9e8e4f49`, isolated libvirt network without forwarding, hosts installed unattended
  from `xcp-ng-8.3.0-20260806.iso` (SHA-256
  `1d06e2d88a0fbf9a38384aa46637fd5d1ad568ba0df7ab3ed95fb047655f22e6`, publisher checksum
  list signature verified against the documented XCP-ng key fingerprint).
- Hosts report product version 8.3.0. Each host has a 64 GiB SATA system disk with a local
  EXT repository and an untouched 32 GiB SATA data disk.
- Workstation: Linux 7.2.15 (Arch-derived), libvirt 12.8.0, QEMU 11.1.2, AMD nested KVM.
- Live harness: `crates/xenadmin-core/tests/lab_live.rs`, run with the manifest and a
  lab-only password file; targets come only from the manifest.

## Workstation prerequisites found during setup

- Cloudflare WARP captured every destination through a policy-routing table consulted
  before the main table and dropped non-tunnel traffic in its own nftables input and
  output chains. The lab could not fetch answer files or reach XAPI until WARP was
  disconnected. The harness now verifies that every lab address resolves to the lab
  bridge after the bridge exists.
- ufw's default policy dropped DHCP requests to the bridge without logging them (its
  after-rules skip DHCP ports). Rules allowing the bridge, DHCP and the answer-file port
  were required.

## Scenarios

| Scenario | Rows | Result | Observation |
| --- | --- | --- | --- |
| Direct recovery, read-only | P01-01, P01-11, P01-13, P19-01 | Passed | Credential-free certificate inspection, exact-endpoint pin, read-only login and current inventory on all three standalone hosts; wrong pins and the untrusted self-signed certificate were refused before any credential was sent |
| Reviewed pool join | P13-02 | Passed after fixes | First attempt was refused locally and the second left an unknown outcome (findings 1 and 2). After the fixes a standalone host joined the two-host pool in eleven seconds: `pool.join` receipt recorded, destination membership verified, member visible on the destination, the old standalone connection kept its original pool identity |
| Guest provisioning | P11-09, P08-02, P07-03, P07-02, P03-01, P04-01 | Passed after fix | A 68 MiB isohybrid Alpine image imported as a raw disk into the coordinator's local repository, a guest created from the generic template without a disk, the image attached as its first disk, a blank data disk added, the guest started, and the embedded console delivered 720x400 frames through the authenticated tunnel; the first attempt stopped at the memory setter (finding 3) |
| Console input, snapshot and reviewed restore | P04-02, P05-02, P05-03, P05-04, P19-04, P19-06 | Passed | Keyboard input through the authenticated tunnel logged into the guest and configured its network; the guest wrote marker A into its data disk's first sector and reported it to a listener on the isolated bridge; a running-guest disk snapshot succeeded; the guest changed the marker to B and reported it; the reviewed restore cleanly shut the guest down, reverted configuration and disks and started it; after the fresh boot a read-only pass reported A and never B. Earlier passes were invalid because the live guest image lacks filesystem tools, not because of the client |
| Guest power matrix | P03-01, P03-02, P03-03 | Passed | Clean shutdown, start, pause, unpause, hard reboot, hard shutdown and start each completed with the expected power state. The tool-less live guest advertises no clean reboot, suspend or resume, and the client recorded those as unavailable rather than attempting them |
| Reviewed maintenance and ejection | P12-03, P13-02 | Passed | Member entered maintenance (disabled observed), reviewed ejection task completed, member left the coordinator's inventory and returned as a standalone pool with a new identity |

## Findings

1. **`host.list_methods` is not implemented on XCP-ng 8.3.0.** The host answers
   NOT_IMPLEMENTED. The join and security-operation reviews treated the missing method
   catalogue as proof that the operation was unsupported and refused before dispatch.
   They now treat an unimplemented catalogue as "cannot pre-verify" and let the server's
   own explicit refusal of unknown methods remain authoritative; an explicitly empty
   catalogue still refuses. The security overview already reported the catalogue error
   rather than failing. Fixture coverage was added for both paths.
2. **A joined member is briefly not live.** After `pool.join` the member restarts its
   toolstack; the coordinator reports it disabled and not live for roughly half a minute.
   Post-join verification failed on the first read and left the operation unknown. It now
   keeps re-reading within its timeout; an operator recheck through the destination
   connection still resolves a late case. Fixture coverage was added.

3. **Halted guests do not advertise `changing_memory_limits` or `changing_VCPUs`.** XCP-ng
   8.3.0 lists `changing_static_range` and `changing_dynamic_range` for a halted guest and
   no vCPU operation at all, yet `VM.set_memory_limits`, `VM.set_VCPUs_max` and
   `VM.set_VCPUs_at_startup` succeed. VM creation gated its memory and vCPU steps on the
   absent names and stopped partially complete, leaving a device-less guest. The gate now
   accepts the range pair for the combined limits setter and lets a halted, idle guest's
   vCPU setters reach the server; a running guest still needs the advertised capability.
   Unit coverage was added.

## Console observation

The host console applied no implicit shift to keysym events: `:` arrived as `;` and
uppercase as lowercase until the harness sent Shift around shifted characters, as a
physical keyboard and the desktop's key forwarding do. A blinking text cursor alternates
two frames, so frame-settling checks must tolerate that.

## Limitations

This run used the core facade through the live harness, not the Rune desktop window.
Native desktop sessions, interruption paths for the passed workflows, memory checkpoints,
migration, storage and network administration and the remaining ledger rows are recorded
as they are exercised. The lab now holds one running guest, its snapshots and one
device-less guest left by the partial creation; they stay for later deletion scenarios.
