# Intra-pool receiver URL on the disposable lab

Pull request 17 (`bee10f06337785aa3c051b7b9d043c17114c9e9b` on
`cursor/migration-receiver-url-874c`) was run against the harness lab. Result: **pass**.

An intra-pool storage move accepted a destination address on the transfer network
that is not that host's management address. A receiver URL whose host is outside the
pool was refused before `VM.assert_can_migrate`, and that address was not stored.
Shared-storage `VM.pool_migrate` was not run.

## Environment

- Commit `bee10f06`, crate 0.20.0. Workstation kernel 7.2.15-1-omarchy, QEMU 11.1.2,
  libvirt 12.8.0.
- Manifest `9e8e4f49-b358-4c76-a189-d9f5e19c1061`. Installer
  `xcp-ng-8.3.0-20260806.iso`, SHA-256
  `1d06e2d88a0fbf9a38384aa46637fd5d1ad568ba0df7ab3ed95fb047655f22e6`.
- Pool `lab-pool-1794220201`, XCP-ng 8.3.0, xapi 26.1, Xen 4.17.6-9, HA off. Members
  `192.168.254.11` (coordinator) and `192.168.254.12`. The third lab host was not used.
- Guest `lab-guest-1794316526`, running, no snapshots, two disks on the coordinator's
  local `ext` repository (1 GiB and about 70 MiB).
- Transfer network `Pool-wide network 1`. The destination's address there was
  `192.168.253.12`; its management address was `192.168.254.12`.

Sanitized evidence is
`/var/lib/libvirt/images/xenadmin-rs-lab/evidence/1794634029-pr17-receiver-url.json`.
It has no session reference.

## What ran

Before the move, one `host.migrate_receive` on the destination over that network was
read and its session logged out. The map's URL hosts were `http` with no port and no
user info: `master` was the coordinator management address `192.168.254.11`; `SM` and
`xenops` were the destination transfer address `192.168.253.12`. Query values were not
recorded.

The refused call went through a local proxy to the real coordinator. The proxy
rewrote the three URL hosts in the live `host.migrate_receive` result to `203.0.113.50`
and would have dropped `VM.assert_can_migrate` and `VM.migrate_send`. The client
rejected the operation with "Invalid XAPI response: migration receiver endpoint is
outside the pool". The proxy recorded one `host.migrate_receive`, three rewritten
URLs, and zero blocked migration calls. The host audit for that session has one
`host.migrate_receive` and no `VM.assert_can_migrate` or `VM.migrate_send`. The guest
stayed on the coordinator with the same disks. A scan of the client state directory
did not find `203.0.113.50`.

The same guest was then migrated live to the other member and back, over
`Pool-wide network 1`, with both disks placed on the destination's local `ext`
repository. Each move succeeded in about 22 seconds, and the client verified identity,
host and disks. The return destination's transfer address was `192.168.253.11`, against
management `192.168.254.11`. The host audit for each move shows `host.migrate_receive`,
then `VM.assert_can_migrate`, then `Async.VM.migrate_send`. A separate read afterwards
showed the guest running on the coordinator, both disks on that host's local storage,
and no pending task.

An earlier proxy attempt never called `host.migrate_receive`: the harness pinned its
own certificate instead of the coordinator's. Nothing was moved.

## Limit

The outside-pool host was substituted in front of the coordinator. This toolstack did
not itself return a foreign receiver URL. Explicit ports, user info, fragments and a
non-`OpaqueRef` session token were not exercised on the lab. The harness scheduled a
logout of the refused receiver session and then stopped that process before the logout
ran; the following seconds of audit show no `session.logout`, and this toolstack does
not implement `session.get_all`, so that session was not confirmed closed. No guest or
task was left in progress.
