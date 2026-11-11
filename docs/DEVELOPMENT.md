# Inspect a running development client

Start the desktop using:

```sh
./scripts/dev.sh
```

Connect in the desktop normally. The development launcher builds the workspace and enables
read-only local inspection with `XENADMIN_DEV_INSPECT=1`. The inspector uses those existing
connections, including their current permissions, trust choices, event inventory and session
generation. It requires no second host login. The normal desktop launch leaves it disabled.

Run these commands from the same checkout and user account:

```sh
./scripts/inspect.sh
./scripts/inspect.sh --watch 30
./scripts/inspect.sh --inventory --limit 25
./scripts/inspect.sh --inventory --vm VM_UUID
./scripts/inspect.sh --inventory --class snapshots --vm VM_UUID
./scripts/inspect.sh --inventory --class disks --vm VM_UUID
./scripts/inspect.sh --inventory --class tasks --object TASK_UUID
./scripts/inspect.sh --diagnose-vm VM_UUID
./scripts/inspect.sh --history --vm VM_UUID
```

Status identifies the application version, process and inspection start time, then lists
active connection UUIDs, state, freshness, inventory revision and errors.
Add `--connection CONNECTION_UUID` when more than one profile is active. Inventory supports
`--offset` and `--limit` for paging. Its typed classes are `vms`, `hosts`, `storage`,
`networks`, `snapshots`, `disks`, `interfaces`, `tasks` and `messages`. `--object UUID`
selects one object; `--vm UUID` also filters snapshot parents and disk/NIC attachments.
Power diagnosis reads the VM's current host record and
asks whether each action is valid. A blocked operation remains blocked. History reports
the local journal's outcomes, errors and individual step progress. These views help separate
stale UI state, host restrictions and interrupted operations.
Unfiltered history also includes `resource_operations` for pool, host, storage and network
administration, including partial steps and newly created UUIDs. Private task references and
creation baselines stay inside the backend.

The wire protocol is versioned JSON over `dev-inspect.sock` in the application's state
directory. The directory must be user-owned with mode `0700`; the socket has mode `0600`
and checks the connecting process's UID. Both requests and responses have size limits,
requests have deadlines, and concurrent queries are bounded. There is no TCP listener.
Only inspection requests are accepted; this interface does not submit host mutations.
Closing the desktop stops inspection and removes its socket.

Output is structured JSON and is private development data: it can contain VM names,
UUIDs, administrator messages and operation details. It omits credentials, session tokens
and raw XAPI references. Nothing is uploaded or logged to a file automatically. Use
synthetic fixtures for committed tests and public reports.

To inspect another instance's explicitly selected state directory:

```sh
XENADMIN_STATE_DIR=/path/to/private/state ./scripts/inspect.sh
```

For standalone diagnosis when the desktop is not running, use the authenticated
[read-only host probe](RUNNING.md#use-the-read-only-host-probe).

## Verification

`./scripts/check.sh` runs isolated protocol and UI fixtures. It does not connect to a
production host. Protocol and trust fixtures wait for asynchronous publication with
bounded polling deadlines; `XENADMIN_TEST_TIME_SCALE` (an integer from 1 to 20, default 1)
multiplies those deadlines on slower shared runners and CI sets 4. The scale never changes
what a fixture asserts. The inspector's Unix-socket tests cover private ownership and modes,
protocol bounds, rejection of unsupported requests, socket lifecycle, and the public data
projection. Headless UI captures contain synthetic data only. Infrastructure-changing
qualification belongs in the explicitly designated [disposable lab](../lab/README.md).

`crates/xenadmin-xapi` generates its record views, enumeration constants and catalogue
from `data/xenapi.json` when it is built, and refuses a description whose digest is not
the one in `data/SOURCE`. The check compares every XAPI call and field the core names as a
string with that catalogue. That crate is under the LGPL 2.1 with a linking exception,
unlike the rest of the workspace: keep anything derived from the description inside it
([decision](decisions/0004-generated-bindings.md)). The description is produced by
`crates/xenadmin-xapi/data/regenerate.sh`, which builds and runs upstream's generator at
a toolstack tag on a throwaway Debian or Ubuntu machine; a new description goes in with
its tag, commit and digest in `data/SOURCE`. `lab/description_check.py` compares a
description with the fields of a lab pool's live records, read-only, and is how a
description is shown to match the hosts it is for.

The development and inspection launchers use Cargo to select the current build executable.
They honor `CARGO_TARGET_DIR` and target-specific output paths; inspection builds the local
probe when necessary without starting a second host session.
