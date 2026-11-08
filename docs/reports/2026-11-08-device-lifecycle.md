# VM device lifecycle increment

This increment implements the halted-VM disk, CD-drive and NIC workflows tracked in
[the retained parity ledger](../PARITY-IMPLEMENTATION.md), rows P07-02/03/04/07/10/13/15.
The remaining ledger rows and live-host qualification stay open.

## Implemented workflows

- Create a named disk with an exact positive capacity on a selected eligible SR, then
  attach it at a reviewed device slot. If attachment fails, the created disk remains
  identified in operation history instead of being silently deleted.
- Attach an existing unattached, managed, non-shared user disk with selected read-only or
  read/write access. Attaching existing data does not claim ownership of that disk.
- Detach a disk while retaining the backing data. A separate irreversible delete flow
  lists consumers and removes only an exclusively owned, non-shared user disk.
- Add an empty CD/DVD drive. Existing insert/eject controls select or replace ISO media.
- Add a NIC on a selected network with an explicit device slot and either a validated
  unicast MAC or a host-generated address. Delete an existing NIC through a separate review.
- Move a NIC to another network, or replace it to change its constructor-only MAC address.
  The review explains that failure after removal can leave the VM without that NIC.
  Supported MTU, rate-limit QoS, locking mode and allowed-IP settings are preserved;
  unsupported extra configuration is refused before removing the original interface.

These configuration changes require a halted ordinary VM and a current writable
connection. Existing plug/unplug controls retain their separate live-operation policy.
The desktop accepts typed resource choices rather than editable opaque references.
Reviews retain the source pool, connection generation, VM and exact device UUIDs when
the operator changes table selection. Stale connections or changed eligibility disable
submission. No snapshot, SR/network administration, or disk migration modes are added here.

## Outcome handling

Each step is persisted before dispatch and rechecks current identity and ownership.
Creation records a bounded UUID baseline and an operation marker. Verification rejects
pre-existing or mismatched results. A lost reply remains unknown until read-only
reconciliation can identify the result; reconciliation never repeats a write or submits
remaining steps. Created disk, attachment and NIC UUIDs appear with step results in the
VM devices page, including partial outcomes that require operator follow-up.

## Verification and limits

Desktop tests cover typed reviews at wide and compact window sizes, captured VM/pool
context, invalid capacities/slots/MACs, detach-versus-delete behavior, consumer listing,
changed ownership, pool switching and missing live observations. The canonical lowercase
inventory class keys now correctly expose device cards and permission messages.

`cargo test -p xenadmin-desktop` passed all 38 desktop tests, including eight new device
lifecycle tests. `cargo clippy -p xenadmin-desktop --all-targets -- -D warnings` passed.
The final `cargo clippy --workspace --all-targets --locked -- -D warnings` and
`cargo fmt --all -- --check` checks also passed. The synthetic device page rendered
successfully at 1440×900 using Rune's Vulkan backend and was visually inspected.

`cargo test --workspace` passed 169 tests: 90 core unit tests, five profile tests,
28 direct-host protocol tests, four TLS tests, four host-probe tests and 38 desktop tests.
The core tests include 16 device lifecycle fixtures covering successful workflows,
ownership and consumer changes, lost replies, restart reconciliation, pre-existing result
identities, imported-disk ownership and preserved NIC settings. NIC allowed-IP sets are
compared canonically so reordered server data does not create a false conflict.

All fixture inventory is synthetic. No private screenshots or host inventory were copied
into the repository, and no production-host changes were performed. Protocol and Rune
tests do not establish compatibility with an actual XCP-ng host, native desktop
accessibility, or the disposable-lab destructive qualification gate.
