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

A view's method returns its field in the type of the field's kind: `FlagField`,
`TextField`, `TimeField`, `RefField`, `ChoiceField` (an enumeration's value), `WholeField`,
`RealField`, `SetField` or `MapField`, and `FieldValue` for a kind without a type. Compare a
field with the Rust type of its kind, and give a reader the kind it reads; a value taken
out of a map by key has no kind, so wrap it where it is read (`TextField::of(&map["key"])`).
Read a field with the readers of its type: `get` for a flag, `text`, `reference`, `name`,
`number` or `unsigned`, `members` or `strings` for a set, `entries` for a map and `get` or
`text` for one of its keys. A typed field has no JSON accessors, so asking a whole number
for text does not compile. A helper that reads one kind says so in its signature
(`fn reference<'a>(value: impl Reads<'a, kind::Reference>)`) and then takes a field of
that kind, or a JSON value that did not come through a view. `value()` gives a field's
JSON value for the rare read no reader covers, such as text that holds a number; a test
lists those written on a view, so add to its list on purpose or not at all.

A view reads a field of its class from whatever record it is given, so a record of
another class reads without complaint where the field has the same name. To look for
that, run the tests or a lab scenario from a debug build with
`XENADMIN_XAPI_VIEW_AUDIT=/path/to/file`: every view then appends to that file the keys
of its record that its class lacks, with the place the view was made (class, key and
place, separated by tabs; names only). A record of another class shows up by the keys
only its own class has, and a fixture that carries a field the API does not have shows
up too. Over the core's tests the file holds only what is meant: fixtures' own switches
(`fixture_…`), a storage backend and a crash dump given a foreign field to show it is
not passed on, and the `ref` the client adds to a CD drive's record it holds.

With the same variable set, a debug build compares every call it sends with the API and
appends a line for each that does not fit: `call`, the call's name, the position of the
parameter (the session is the first) or `count`, and what was wrong, such as
`bool given a string` or `takes 3 to 4, given 2`. Parameters the API marks as optional
may be left out from the end. A line `sent` and a name records each call made, and a
line `viewed`, a class and a place each place that made a view, so that what the two
checks covered can be counted. `scripts/view_audit.py` fails on any `call` line;
`scripts/check.sh` runs it over the workspace's tests. A reference of the wrong class is
a string like any other and is not seen.

Write a call as its message from `xenadmin_xapi::calls` and send it with `Session::send`:
`session.send(calls::vm::Start { vm: refs::vm(&reference), start_paused: false, force: false })`.
The fields are the API's parameters by name and kind, so a missing argument, one of the
wrong kind and a reference of another class do not compile. Give a reference with the
function for its class in `refs`, from text or from a value a host sent, and a whole
number with `Int::from` or `.into()`; a message with one parameter is a tuple
(`calls::vm::GetRecord(refs::vm(&reference))`) and one with none a unit. For a call that
is prepared now and sent later, `.request()` gives its name and arguments and `.task()`
the same under `Async.`. Only a call whose class is not known until run time is still
written as a name and a list (`session.call(&format!("{class}.get_record"), ..)`); a test
lists the calls the session layer writes that way and fails on another. Write
enumeration values as the generated constants (`vm_power_state::HALTED`,
`vdi_operations::DESTROY`), not as literals: a value the API does not have then does
not compile. That holds for the client's own types too (`Ipv4Mode::as_str` returns
`ip_configuration_mode::DHCP`). The exception is a name that is a message's name first,
such as a power action's, which a test holds to the API instead.

The development and inspection launchers use Cargo to select the current build executable.
They honor `CARGO_TARGET_DIR` and target-specific output paths; inspection builds the local
probe when necessary without starting a second host session.
