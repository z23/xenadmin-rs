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

## The front end: scenes, tests and inspection

The front end is GPUI Kit, in `crates/xenadmin-desktop`, which builds two binaries: the
client `xenadmin-rs` and the scene tool `xenadmin-scenes`. Everything the client shows is
described by the toolkit-free model in `crates/xenadmin-ui`. (Until the cutover of the
[migration](GPUI-MIGRATION.md) this crate was `crates/xenadmin-gpui` with the client
`xenadmin-gpui`, beside the Rune client that the releases up to 0.19.0 were built from;
reports of that time use those names.)

Never start the client to try something out unless you mean to open a window on your own
state: every start other than `--version` and the headless `--scene NAME --snap/--tree`
form opens a real window, and without `--demo` or `--scene` it takes the state directory
and restores its connections. Use the scene tool and the tests.

### Scenes

A scene is a model set up by a function and rendered as the whole window: title and menu
bar, toolbar, navigation pane, object header, tabs, tab body, status bar, and any open
dialog. The scene tool draws one without a display:

```sh
cargo run -p xenadmin-desktop --features scenes --locked --bin xenadmin-scenes -- --list
cargo run -p xenadmin-desktop --features scenes --locked --bin xenadmin-scenes -- \
  demo --theme dark --size 1440x900 --scale 1 \
  --snap artifacts/ui/demo.png --tree artifacts/ui/demo.json
cargo run -p xenadmin-desktop --features scenes --locked --bin xenadmin-scenes -- --commands
```

- `--list` prints every scene: the registered scenes of the model
  (`xenadmin_ui::scene::registry()`, under the names they had in the Rune front end), then
  the fixtures of the renderer and of the shell (`gallery-*`, which are not product
  scenes).
- `--commands` prints the command registry, one line per command (id, label, menu path,
  toolbar slot, context kinds, local row, shortcut, area). The Commands table of the
  [migration ledger](GPUI-MIGRATION-LEDGER.md) is checked against it. `gallery-shell`
  shows the toolbar and menus of the plan with commands that do nothing, and
  `gallery-empty` the window of a first start.
- `--snap` writes the frame as a PNG and needs a graphics adapter; a software one (Mesa
  lavapipe) is enough. `--tree` alone needs none.
- `--scroll KEY=PIXELS` and `--reveal KEY` (both repeatable, run in the order given) scroll
  before the capture, so that what is below the fold is looked at in a frame of the real
  size instead of a tall one. `--scroll` turns the wheel over the element with that key
  (`tab-scroll` is the body of a tab, `<surface>-body` of a dialog; a grid and the tree
  scroll under their own keys); `--reveal` scrolls an element into view. The frame's
  completeness (`missing`) is judged before the first step.
- `--tree` writes JSON: `elements` (every element with an id, in reading order, with its
  id path, role, accessible label, value, description, bounds, and focused, checked,
  selected, expanded and disabled state), `missing` (elements the description has and the
  frame lacks) and `model` (the model's own account: title, navigation rows, header,
  status, tabs, the resolved menus and toolbar with the reason of every disabled command,
  the flattened tab body and surfaces).
- The tool exits with 1 when the frame lacks an element its description has, 2 for a
  wrong command line.
- `--theme` is `dark` or `light`, `--size` the window in logical pixels (800x600 is the
  smallest size the main window can be given; below 1100 wide the navigation pane is
  behind its toggle), `--scale` 1 or 2 device pixels per logical pixel.

The client takes the same flags, when it is built with the `scenes` feature, and returns
before it opens a window, starts the backend or touches the state directory:

```sh
cargo run -p xenadmin-desktop --features scenes --locked --bin xenadmin-rs -- \
  --scene demo --snap artifacts/ui/client-demo.png --tree artifacts/ui/client-demo.json
```

Without `--snap` and `--tree`, `--demo` and `--scene NAME` open a real window on the
fixture. A preview takes no lock on the state directory, restores no session, opens no
inspection socket and writes nothing. `--version` is answered before anything else. The
released client is built without the feature `scenes` and refuses `--snap` and `--tree`.

`./scripts/check.sh` renders every scene into `artifacts/ui/` (with `scenes.txt` and
`commands.txt`), and CI keeps them as the artifact `ui`. The script unsets `DISPLAY` and
`WAYLAND_DISPLAY` itself: nothing in it opens a window. A PNG depends on the fonts and the
graphics adapter of the machine that made it: it is something to look at, and assertions
belong on the tree and in the tests. A headless render does not establish Wayland
behaviour, the clipboard, keyboard input or a screen reader.

### The console view

The console is the one view that is not a description. The model
(`crates/xenadmin-ui/src/areas/console/`) owns every console and describes its tab and
its detached windows as ordinary blocks, one of which is the element `console-canvas`;
`crates/xenadmin-desktop/src/console/` draws that element and performs the windows.
`canvas.rs` uploads the session's frame once per frame generation (it looks after every
poll of the model, since a new frame alone redraws nothing else), paints it letterboxed
(Fit) or one to one inside a view that scrolls on both axes (Actual size), and sends the
pointer and the keys to the guest through the functions of the model's `canvas` module.
While the canvas has the keyboard every key is the guest's: no command shortcut, focus
traversal or Escape of the application happens. `keys.rs` takes the keys by evdev code
from a second keyboard listener on the Wayland display
([decision 0006](decisions/0006-console-key-codes.md)); where there is no listener (X11,
the headless test platform) it forwards GPUI's keystroke with a code looked up by the
key's name on the US layout, which stays experimental. `windows.rs` opens one OS window
per detached console and tells the model when one is gone. The scene `console-window`
renders detached window 1 instead of the main window, and `console-frame` and
`console-actual-size` carry a synthetic frame. `tests/console.rs` drives all of it on the test platform, which has no
Wayland seat: the listener's part, the guest's cursor (not drawn) and the wheel's
direction in a guest are not established by any test here. The headless runs against the
lab ([report](reports/2026-11-19-gpui-live-headless.md)) showed a real guest's frames on
the canvas and keys reaching the guest through the Kit's test key events, which is the
path without a listener; the listener has run on a real seat only in the spike
([decision 0006](decisions/0006-console-key-codes.md)), never with a console.

### The performance charts

The Performance tab describes one block `metrics-chart` per series, which only names the
chart; `crates/xenadmin-desktop/src/charts/` draws it and reads its data from the model by
the block's key at every render (`xenadmin_ui::areas::performance::chart`), so it keeps
no state and never waits. The element is bespoke because of the gap rule: the Kit's line
chart joins the neighbours of a missing sample, and here every unbroken run of samples is
a stroke of its own, a lone sample is a dot, and nothing is drawn where data is missing.
The card carries the chart as text for the element tree and a screen reader (title,
summary, the labels of the top and bottom edge, the first and last time; the plot is an
image named by the series and its summary). `tests/charts.rs` checks what each plot
painted and, in a captured frame, that the gap has no ink. The last cards are below the
fold: bring one into view with `--reveal metrics-chart-3-plot`, or render `performance` at
a tall size (`--size 1440x1700`) to see all four.

The scene `large-inventory` (16 hosts, 2,000 guests) has a check and a measurement in
`tests/large_inventory.rs`. The check counts the rows the tree and the grids build
(`xenadmin_desktop::stats`) and holds that a frame, a turn of the wheel and a selection change
build the rows in view and never the inventory; it needs no adapter. The measurement is
ignored and asserts nothing about speed: it prints the time of the first frame, of a scroll
and a selection change in the tree and in the Search grid, and of a replaced inventory, as
p50 and p95, with the frame drawn by the offscreen renderer. Run it in a release build on
an idle machine and record the numbers with the CPU and the adapter:

```sh
cargo test --release -p xenadmin-desktop --features scenes --locked --test large_inventory -- --ignored --nocapture
```

### Tests

```sh
cargo test -p xenadmin-ui --locked                       # the model: no window, no toolkit
cargo test -p xenadmin-desktop --features scenes --locked  # windows on the Kit's test platform
```

`tests/renderer.rs` drives the generic renderer and `tests/shell.rs` the shell through
real events (pointer, keys, text) on a headless window. A test finds an element by its
id, which is the key the model gave it, or by its accessible label. `tests/wanted.rs` and
`tests/wanted_events.rs` hold the window halves of the tests of the Rune front end, area by
area; `tests/theme.rs` the contrast of controls against their surfaces; `tests/lab_live.rs`
the ignored runs against the lab ([verification](VERIFICATION.md#live-lab-scenarios)).
Without `--features scenes` the tests that need the Kit's test harness are not built.

`python3 scripts/gpui_ledger_tables.py check` compares the
[migration ledger](GPUI-MIGRATION-LEDGER.md) with the tree: every scene registered, every
test the ledger names as a successor present, the counts of its summary, and, with
`--commands artifacts/ui/commands.txt`, the Commands table against the registry. The
sources of the removed Rune crate are read only when `XENADMIN_RUNE_SOURCES` names a
directory that holds them (for example an export of commit `89d700c`, the last with the
Rune crate: `git archive 89d700c crates/xenadmin-desktop | tar -x -C DIR`, then
`XENADMIN_RUNE_SOURCES=DIR/crates/xenadmin-desktop/src`).

### Inspection

The inspection socket belongs to the model, not to the front end: the client starts it
when this process owns the state directory and `XENADMIN_DEV_INSPECT=1` is set, prints the
`Read-only developer inspection:` line, and `./scripts/inspect.sh` reads it (the protocol
is the one the Rune client served). `./scripts/dev.sh` starts the client that way:

```sh
./scripts/dev.sh            # XENADMIN_DEV_INSPECT=1 cargo run --locked --bin xenadmin-rs
```

One client owns a state directory: a second start on the same directory asks the first to
show its window and exits. To run a development build beside an installed client, give it
a directory of its own (`XENADMIN_STATE_DIR`) and pass the same variable to
`./scripts/inspect.sh`. The request to show the window is tested on the Kit's test
platform. On Wayland the first client asks the compositor again with the token the second
start forwarded (`wayland_activate.rs`): in a virtual machine on sway 1.12 that marked the
first window urgent, and focused it where sway's policy allows a launcher's token to;
before that fix a second start often showed nothing
([report](reports/2026-11-19-gpui-vm-session.md), finding 1). No other compositor was
tried. A start from a shell that names no display ends with a message before it asks.

## Verification

`./scripts/check.sh` runs isolated protocol and UI fixtures. It does not connect to a
production host and opens no window. In order: formatting, Clippy, the guard that the
model depends on no UI toolkit, the guard that no `rune` crate is in the workspace's
dependency tree, the workspace's tests, Clippy and the tests of the front end with the
feature `scenes`, the guard against `block_on` in the front end, a render of every scene,
the view audit, the vendored RFB tests, the Python tests of `lab/` and `packaging/`, and a
build that must report both executables. Protocol and trust fixtures wait for asynchronous publication with
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
