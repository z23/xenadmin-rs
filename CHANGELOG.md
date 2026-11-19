# Changelog

Each entry states what changed for someone running the client, what is known to be wrong
in that build, and the stored formats it reads and writes. Evidence for each entry is in
the linked reports; the [retained parity ledger](docs/PARITY-IMPLEMENTATION.md) remains the
authority on what is implemented and what is qualified.

0.18.0 is the first published version. Versions up to 0.17.0 were local development
checkpoints.

## 0.20.0 (in development)

Stored formats: operation journal 5, connection profiles 3. The journal format changes
from 4. This version marks a journal as format 5 the first time it opens it. 0.19.0 then
refuses that journal, says it was written by a newer application and leaves it as it is,
so 0.19.0 cannot submit changes from a state directory this version has used. Nothing
stored is rewritten: no table and no recorded operation changes. Format 5 is the first
that may hold a batch of snapshot exports, which 0.19.0 cannot read. It may also hold a
batch of cross-pool moves, which 0.19.0 cannot read either. The number is not raised
again for that, because no published version reads format 5.

### Added

- Several snapshots of a VM can be exported in one reviewed batch. In the VM's Snapshots
  view, Shift or Ctrl with a click selects 1 to 32 snapshots, and "Export selection…"
  opens a review that lists each snapshot's name and UUID, its own new output path, the
  format and the permission assessment. The snapshots are exported one after another,
  each as an archive of its own: plain, gzip or zstd XVA, or for disk snapshots an OVF
  directory or an OVA file. A memory checkpoint is exported as XVA only, and its archive
  holds the guest's memory only where "Include guest memory" is chosen for that
  checkpoint; otherwise it holds configuration and disks. Every snapshot and destination
  is checked before the first transfer, Stop finishes the current archive and skips the
  rest, and a recheck reads what exists without downloading or publishing anything. While
  several snapshots are selected, restore, edit, copy, inspect, delete and the single
  export are unavailable, because each takes one snapshot. The batch is experimental: it
  is verified by fixtures and by lab runs in which a disk snapshot and a checkpoint with
  its memory were exported in one batch and read back, a batch was stopped during its
  first export, and one was cut off by the client closing and acknowledged afterwards;
  nobody has used the dialog
  ([report](docs/reports/2026-11-12-batch-snapshot-export.md)).
- Several VMs can be moved to another connected pool in one reviewed batch. With 1 to 32
  VMs selected in the inventory, "Move selection to another pool…", beside "Move
  selection…" and in the row menu, opens a review in the three stages of the single
  cross-pool move. The first takes the destination pool and host. The second takes one
  destination repository for every disk of the selection, one destination network for
  each source network, and the destination host's migration address. The third lists,
  for each VM, how it moves and where each of its disks and NICs goes, shows what the
  sessions on both pools are allowed, and asks once for the whole batch for the
  acknowledgement that host-to-host transfer may be unencrypted HTTP or TLS without
  certificate verification. A running guest moves live and stays running, a halted guest
  stays halted, and both may be in one batch. A suspended or paused guest, a template,
  or a VM the single move would refuse makes the whole selection unavailable; none is
  left out. A batch moves and does not copy. Before the first transfer every VM is
  checked on both pools as the single move checks it, and the destination repository
  must have room for the disks of all of them. The VMs then move one after another, each
  by the single cross-pool move, with both pools reserved until the batch ends. Stop
  finishes the current VM's move and skips the rest. It does not ask the hosts to cancel
  a transfer, and a transfer that is under way in a batch cannot be cancelled from the
  client. After an interruption or a restart nothing continues by itself, because the
  receiver a move is prepared with is not kept. With both pools connected again, Recheck
  reads the moves that had started, repeats no transfer and starts no remaining VM. A
  move whose outcome cannot be read stays unknown, with both pools reserved, until it is
  acknowledged. The batch is experimental. It is verified by fixtures and by lab runs
  in which two guests were moved to another pool and back, halted and then running, a
  batch was stopped during its first move, and one was cut off by the client closing
  and settled by a recheck after the restart; nobody has used the dialog
  ([report](docs/reports/2026-11-12-cross-pool-bulk-move.md)).

### Changed

- The interface is rebuilt on a different toolkit, GPUI Kit, and laid out differently.
  The Rune interface of 0.19.0 and earlier is gone from this version. The move itself
  changed nothing in the backend: no commit of it touches the core, the XAPI bindings, the
  journal or the profile store (the other entries of this version do). What someone who knows 0.19.0 meets:
  - A menu bar (File, View, Pool, Server, VM, Storage, Templates, Tools, Window, Help) and
    a toolbar, both filled from one table of commands, which also fills the context menus
    and the row of actions at the top of a tab. A command that cannot be used now is shown
    disabled with its reason. What the old window's toolbar buttons opened (Connections,
    Options, Activity and the others) is reached from the menus, the Home tab and the
    Notifications view.
  - Navigation views on the left, chosen at the bottom of the pane: Infrastructure (the
    tree of connections, pools, servers and guests), Objects (by type), Organization,
    Saved Searches and Notifications. Below a window width of 1100 the pane is behind a
    toggle.
  - Tabs per object in place of the details pane: a guest has General, Memory, Storage,
    Networking, Console, Performance, Snapshots and Search, and a server, a pool, a
    repository and a network have their own sets. A surface that only shows state is a
    tab; a dialog opens only for an action.
  - Properties dialogs with pages for a guest, a template, a server, a pool, a
    repository, a network and a disk, in place of the separate settings dialogs.
  - Wizards with steps and a final review for the longer actions (New VM, New Storage,
    New Network and Migrate to Server among them).
  - The Notifications view with its Alerts and Events tabs in place of the Activity
    dialog: progress, unknown outcomes, recheck, acknowledgement and history are on
    Events. The members of a batch are folded under the batch, with a checkbox to show
    them as rows.
  - The window cannot be made smaller than 800 by 600. The compact layout of the old
    window (down to 513 by 652) does not exist. A separate console window can be as small
    as 480 by 640.
  - Preferences are read from the existing file. The width of the old details pane is
    read and no longer written, as there is no such pane. The state directory of a
    released client (0.17 and 0.19 were tried, on copies) is read without loss
    ([report](docs/reports/2026-11-19-gpui-upgrade-state.md)).
  - "Reduce animation" in Options still applies. The desktop's own reduced-motion
    setting, which the old interface followed as well, is no longer followed: the
    toolkit reads none.
  - The command line keeps `--version`, `--help` and `--demo`. The options that belonged
    to the old toolkit (`--html`, `--video`, `--frames`, `--script`, `--native`,
    `--list-scenes` and the like) are gone, and `--theme` and `--size`, which 0.19.0
    accepted on an ordinary start, now apply to `--demo` only: on an ordinary start the
    client prints its usage and ends with status 2. The appearance is set in Options.
  - Started where no display is named, the client says so in one line and ends, also
    when a client is already running, whose window such a start used to raise. When the
    display cannot be reached, has no seat, or no usable font or Vulkan driver is found,
    it says which in one line and ends with status 1.
  - A font of one of these families must be installed: Noto Sans, DejaVu Sans, Cantarell,
    Adwaita Sans, Ubuntu, Arial, Helvetica or Segoe UI. The client embeds none and
    follows no fontconfig alias; the [installation guide](docs/INSTALLATION.md) names
    the packages. The archives therefore carry no font notices any more.
  - Accessibility: screen readers are not supported, as before. The new interface gives
    its elements roles, names and descriptions. Orca was started with it once, in a
    virtual machine, and what it would speak for the demo's views was read from its log;
    no person has used a screen reader with it, and the statement stays "not supported"
    until such a run is recorded.

  How far this is verified, which is less than for the interface it replaces: every area
  of the old interface is ported, and the new one passes its model's tests, its window
  tests with real pointer, key and text events on the toolkit's headless test platform,
  and a render of every scene. On 2026-11-19 it was run against the lab without a window
  through the supported areas: connecting with pinned certificates and following the
  inventory, power singly and as a batch with their events, a guest's console with keys
  typed through the canvas and a host's text console, a console screenshot saved,
  snapshots, performance, host maintenance with a real evacuation, live migration both
  ways, disconnecting and connecting again and a second start, and a client closed in the
  middle of an operation
  ([report](docs/reports/2026-11-19-gpui-live-headless.md)); and through their refusals,
  interruptions and recoveries: reboot, suspend and the force actions, a batch that
  stops on a failure, the refused reviews, a migration with its disks and its
  cancellation, a halted VM's disks moved, a snapshot with memory, the console in a
  second window and the clipboard typed as keys, a member's toolstack restart, reboot
  and shutdown through maintenance, and recovery after a restart of the coordinator's
  toolstack, a member that lost power and a certificate that is not the pinned one
  ([report](docs/reports/2026-11-19-gpui-live-paths.md)). On the same day the packages CI
  built from the branch (commit `e808a2f`) were run in windows for the first time, in
  virtual machines with software rendering and nobody at them: installed on clean Ubuntu
  24.04 and Arch systems ([report](docs/reports/2026-11-19-gpui-clean-install.md)), and
  driven in a Wayland session on sway with keys and pointer injected at the machine's
  devices, through connecting to the lab, the recovery gate, consoles, the clipboard,
  the file chooser, a second window and fullscreen
  ([report](docs/reports/2026-11-19-gpui-vm-session.md)). See the known issues for what
  has not been shown.

- Host maintenance and moving a VM within its pool join the supported part
  ([feature scope](docs/FEATURES.md)): entering and leaving maintenance with evacuation,
  a toolstack restart, a reboot or shutdown through maintenance, a live migration with
  or without the VM's disks, and a halted VM's move between repositories. Their dialogs
  no longer carry the experimental notice. What changed is the evidence, not the code:
  their failure paths were recorded on the lab, among them a member's reboot and a
  storage live migration each cut off by the client closing and settled from the journal
  on restart without being sent again. The other changes the host dialog offers keep
  the notice.
- The bar above the VM table has the new entry. In a window 513 points wide it takes
  three rows where it took two, and the table shows one row fewer. In a window 1440
  points wide the table keeps its place.
- A recheck of a cross-pool move or copy that was interrupted before it had prepared
  anything ends it as not sent. The journal shows that case exactly, because the
  transfer's plan is stored before the first call that changes a host. It used to stay
  unknown until it was acknowledged.
- A connected session now checks its inventory against the hosts once a minute: it reads
  the host records directly and compares them with the ones its inventory holds. If the
  same host differs at two checks running and no event for it arrived in between, the
  events that keep the inventory current are not arriving, and the whole inventory is
  read again; the connection shows itself as
  synchronizing for that moment. This is a safeguard after one lab run in which the
  inventory did not follow a host for fifteen minutes (finding 64), which twelve further
  attempts under the same conditions did not reproduce.
- A forced shutdown or reboot that would interrupt a migration of the VM is refused
  unless its consequence is accepted. The host admits a forced action during a migration
  and does not always end the migration afterwards; on the lab that left a task pending
  and partial disk copies that took a host restart to clear, twice in two runs (finding
  63 of the [live report](docs/reports/2026-11-09-lab-qualification.md)). The review
  says so and has a box to tick, "Force anyway: I accept that the migration may be left
  unfinished on the hosts"; until it is ticked the action cannot be confirmed, and the
  client's core refuses it as well. A forced action over any other running task is
  reviewed as before.
- A cross-pool copy or move that the client declines because of the guest itself (a
  snapshot chain, a vTPM, HA protection, vApp membership, installation media left in a
  drive, a disk that is shared or has other consumers) or because the host does not
  allow the migration is refused in those words. It used to be reported as "Invalid
  XAPI response" followed by them, as if the host had answered wrongly. The cross-pool
  workflows remain experimental.
- Inside, 177 more reads of host records go through the generated bindings, and then
  202 more, each after reading where its record comes from, leaving 289 by key
  ([decision record](docs/decisions/0004-generated-bindings.md)). None of those left is
  there because its record's class was not looked for: they are keys inside maps, the
  client's own JSON, or code that is given records of several classes. Nothing in the
  client's behaviour changes with them, but for the archive check under Fixed.
- Inside, a call to a host is written as a typed message: its arguments by the API's
  names, each in the type of its kind, and a reference with the class it is of. The
  bindings generate one for each of the API's 1,941 messages, and the core writes 363
  calls with them. A missing argument, one of another kind, two in each other's places
  or a reference of another class no longer compiles, in a call that no test reaches as
  much as in one that is tested. What a host is sent is unchanged.
- Inside, a field read through the bindings has the type of its kind, so a flag, text, a
  reference, a number, a set and a map cannot be compared or handed to a reader as one
  another. A typed field now has only the readers of its kind: reading one as another
  kind of value, which used to compile and give nothing, no longer compiles, and the
  helpers the workflows share say which kind they read. The values of the API's
  enumerations are the generated constants in the client's own types as well, 276 more
  of them. Nothing in the client's behaviour changes with this
  ([decision record](docs/decisions/0004-generated-bindings.md)); the supported part
  was run again on the lab with it, all but the coordinator's power cut
  ([live report](docs/reports/2026-11-09-lab-qualification.md)).
- Inside, debug builds can compare every call they send with the API: how many arguments
  it takes, which of them may be left out, and the type of each. The repository's tests
  run with that check and it reported nothing. Release builds contain none of it.

### Fixed

- An export or import that was interrupted after its task was created on the host and
  before its transfer was sent stayed unknown, a recheck could not settle it, and in a
  batch it kept the whole batch unknown until it was acknowledged, although nothing had
  been read or written. A recheck now ends it as not sent: the journal records a
  transfer as sent before its request goes out, so one without that mark sent nothing.
  In a batch the task the client created for it is left on the host, because a batch's
  recheck only reads.
- A disk snapshot that the host does not offer for export, because it is busy or the
  export is blocked on it, is no longer shown as ready to export and then refused when
  the transfer is prepared. It is shown as unavailable, as a VM and a memory checkpoint
  already were.
- An XVA archive of a host's default template is refused when the template is marked
  in the record's own field. The inspection looked for that mark under a key where
  hosts do not write it and so relied on the older `other_config` key alone. Importing
  archives remains experimental.
- Creating or removing a tunnel or an SR-IOV network could not have worked, nor could
  the review that precedes it: the client wrote the two classes of the API with a
  capital letter, and a host answers a call on a class it does not know by that exact
  name with "no such message" (finding 66 of the
  [live report](docs/reports/2026-11-09-lab-qualification.md)). The names are now the
  API's. The inventory itself listed tunnels as before, since a host's event
  subscription takes class names in any capitals. Both areas remain experimental and
  have not been run against a host.
- A host that answered a login by naming its coordinator at a plain `http` address was
  followed there, and the pool's password went with the second login unencrypted. Only
  a loopback address can be named that way, since no other plain address is accepted
  as an endpoint. A coordinator is now followed over HTTPS only, and such an answer is
  refused before the password is sent (issue 2).
- Starting the application with one of the options that render it without a window
  (`--html`, `--video`, `--frames`, `--script` and the like) still restored a saved
  session and asked the keyring for its password. Those options went with the old
  toolkit and are now refused; the ones that remain (`--demo`, and in development
  builds `--scene` with `--snap` and `--tree`) are previews: no session is restored and
  the running desktop is not claimed (issue 1).
- A `preferences.json` that the client cannot read (one value of a kind it does not
  accept, a truncated file) was replaced by the defaults at the first close, and the
  saved searches, external tools, graph presets and hidden history it held were gone,
  with no message; 0.19.0 does the same. The file is now kept beside the new one as
  `preferences.json.unreadable`. The window still says nothing about it.
- Editing a connection's address kept the certificate pin of the old address and
  attached it to the new one. A connection's saved pins and certificate details are
  now dropped when its address changes, so the host at the new address has to be
  trusted for itself (issue 7).
- The inspection of a host backup before a restore took an entry's name from the GNU
  long-name field when the archive had one, although a PAX path in the same archive
  said otherwise, and likewise for a link's target. An entry whose two names disagree
  is now refused (issue 3).
- Cancelling that inspection did not take effect, and its ten-minute limit was not
  applied, while the file began with a long run of empty gzip members. The stop is now
  checked between reads of the compressed file (issue 4).
- The address a disk transfer or a host archive is sent to was built in a way that cut
  an IPv6 address short at its first colon and turned shorthand forms such as `127.1`
  into another address. It is now built from the host's address as an IP address or a
  host name, anything else is refused, and the result is checked against what was
  asked for (issue 6).
- Removing a secret that the client had created for a host's power-on or for storage
  credentials took it for unused when a host or a repository still named it in another
  spelling of the same identifier: as a URN, in braces or in capitals. The identifiers
  are now compared as identifiers, and such a secret is kept (issue 8).
- An intra-pool move that copies disks refuses a receiver address outside the pool
  before the move is sent. The destination's address on the chosen transfer network
  is still accepted when it is not that host's management address. A pool host's
  management address is still accepted. The refused address is not stored. Fixtures
  cover both cases. Nobody has run this on the lab over a transfer network separate
  from management (issue 5).

### Known issues

No person has used the new interface in a window. It has run in windows only in virtual
machines, with software rendering, input injected by a script and nobody at them
([clean installation](docs/reports/2026-11-19-gpui-clean-install.md),
[driven session](docs/reports/2026-11-19-gpui-vm-session.md)). In particular:

- It has not drawn on a hardware GPU, and the only compositor it has run on is sway 1.12:
  not Hyprland, GNOME or KDE. Typing into a console in a Wayland session goes through a
  keyboard path (key codes read by a second listener on the display,
  [decision 0006](docs/decisions/0006-console-key-codes.md)) that has carried injected
  keys to a lab guest on sway, exactly and under a second layout, and has never been used
  with a real keyboard. The clipboard with the window focused, the file chooser of the
  desktop portal, fullscreen and the window's decorations (drawn by the compositor) were
  observed in that session and nowhere else.
- Starting the client a second time asks the compositor to show the running window. On
  sway that marks the window urgent and focuses it only where the compositor's policy
  allows; elsewhere it has not been tried.
- In an X session the window has been opened, with scripted input, and no console was
  tried there: its keyboard path (the toolkit's keystrokes, with codes looked up on the
  US layout) is experimental and has not run in a window. Whether the desktop's busy
  indication ends when the window appears in an X session has not been looked at.
- Over a console the local pointer stays visible and the guest's own cursor is not drawn.
- No person has used a screen reader with it. The reason a menu item is disabled is in
  its name or a tooltip, not in an accessible description.
- The packages that have been installed and run (archive and AppImage, on clean Ubuntu
  24.04 and Arch systems, and the AppImage through the recovery gate: find the Xen
  Orchestra VM, start it and use its console with Xen Orchestra down) are those CI built
  from the branch at commit `e808a2f`, in the virtual machines above. They lack what was
  fixed afterwards: in them a second start often shows nothing, a held key does not
  repeat in a console, "Reduce animation" has no effect, and a start without a display,
  a seat, a font or a Vulkan driver panics, stays silent or does not end. No package
  holds those fixes yet, and the recovery gate has not been run by a person or on a
  release artifact.
- No keyring was present in any run of the new interface: a saved password and
  connection at startup have not been tried with it.
- The lab runs of 2026-11-19 were headless: the interface was driven without a window.
  Still not run through the new interface: a coordinator change and maintenance of the
  coordinator, the host's own refusal of an evacuation, and an operation in flight while
  the coordinator's toolstack restarts; the core's behaviour there is unchanged and was
  recorded with earlier versions.
- Found in those runs and not changed
  ([report](docs/reports/2026-11-19-gpui-live-paths.md), findings;
  [report](docs/reports/2026-11-19-gpui-vm-session.md), findings 3 and 4):
  - the review that cancels a running migration goes stale about once a second, so it
    can be refused and must then be opened and confirmed again at once;
  - a member that has lost power looks like an enabled server for about ten minutes, and
    then only the General tab says "Host live = No"; the tree marks maintenance,
    restarting and switched off all as "Disabled";
  - while connected there is no way to review the certificate of a pool member whose
    console is refused for it: disconnect, then Connect with the member's endpoint under
    Certificate options;
  - the password field is emptied after each refused attempt and after a certificate
    review;
  - the menu bar is not opened by F10 or Alt with a letter;
  - a host's refusal is shown by its code, for example `VM_REQUIRES_SR`.
- The Snapshots tab's row of actions has seven commands and wraps in an 800 wide window
  at an interface size above 100%; selecting a row then moves a short table.
- The experimental areas are verified in the new interface on fixtures and by rendering
  only. The lab runs their entries in this changelog cite were made through the Rune
  interface or through the core's own scenarios
  ([ledger](docs/PARITY-IMPLEMENTATION.md)).
- An open drop-down list and the tooltip of a disabled control cannot be captured by the
  headless scene tool and have not been looked at.

Those of 0.19.0 stand, with these from the lab on 2026-11-12:

- A forced shutdown or reboot during a storage migration can leave the migration's task
  pending on the host and partial copies of the disks on the destination. On the lab
  that happened in two runs of two that day, and clearing it took toolstack restarts and
  a power cycle of the destination host (finding 63 of the live report). The client
  refuses such a forced action unless that is accepted in its review. It cannot repair
  the result.
- Once, in a session opened less than a minute after the coordinator's toolstack had
  been restarted, the inventory did not show a member's toolstack restart, or that the
  restart had enabled the host again, for fifteen minutes. The operation that followed
  was refused on its fresh read of the host, so nothing was changed by mistake. It did
  not happen again in fourteen further runs, twelve of them made to provoke it, and its
  cause is not known. The client now checks its inventory against the hosts once a minute
  and reads it again if they disagree, which bounds how long such a state can last
  (finding 64).

- After a coordinator loses power, changes made in the last minutes before it can be
  undone, the pool's database not having been written out in between (finding 65). On
  the lab a VM that the client had deleted and verified gone was back as a record
  without disks; and, worse, a guest that had been moved between pools two to three
  minutes before a second power cut was left with no disk attachment, its disks sitting
  unattached and unnamed on the repository. The disks were intact and could be attached
  again. The client's history still shows what it verified at the time.

## 0.19.0 (2026-11-12)

The second published version, a beta for XCP-ng 8.3 with the same supported part as
0.18.0 ([feature scope](docs/FEATURES.md)). Most of what changed is inside: the client now
reads host records through bindings generated from the API description of the toolstack
it is qualified on, and that work found and fixed defects that test fixtures had hidden.
The [live report](docs/reports/2026-11-09-lab-qualification.md) says which lab scenarios
were run again on this code and which were not.

The packages are a portable archive and an AppImage for x86-64 Linux, each with its
SHA-256 checksum; [installation](docs/INSTALLATION.md) says how to check and install
them. This is an independent project, not an official XCP-ng or Vates application.

Stored formats: operation journal 4, connection profiles 3. Unchanged from 0.18.0. The
preferences file gains the details pane's width, which 0.18.0 ignores.

### Changed

- The packages now contain one component under a different licence from the rest: the
  XAPI bindings, generated from the upstream API description, are under the LGPL 2.1
  with a linking exception. Their licence and a notice of their source are under
  `third-party/xenadmin-xapi-VERSION/`. Everything else stays under the MIT licence.
- Inside, the core reads host records through those generated bindings in its decoders
  and in most of its operations, and every call and field the client names is checked
  against the generated catalogue. A field read through them has a type of its own, so
  code that expects one field cannot be given a whole record.
- The bindings are generated from the API description that upstream's own generator
  writes for the toolstack release the client is qualified on (26.1.11). The description
  first used was a copy upstream keeps in its repository, which turned out to date from
  2024 and to lack 3 classes, 54 fields and 115 calls of that release.
- Inside, values of the API's enumerations (power states, task states, disk types,
  operation names) are written as generated constants, so one the API does not have
  cannot be written, and debug builds can check that each record is read as the class it
  is of. Seven reads through the wrong class and one comparison with a disk type that
  does not exist were found and corrected; none of them changed what the client did.
- A console opened in a window of its own is drawn at the interface size, its picture
  included, and follows a change of the size or the appearance made in the main window.
  Ctrl with plus, minus or 0 works in such a window while its console is not holding
  the keyboard.
- The details pane's width is kept between runs, as the sidebar's and the columns' are.
- `xenadmin-rs --version` prints the version and exits. It used to start the client as
  if no argument had been given.

### Fixed

- The pool's HTTPS-only setting was never shown and could not be changed on a real pool:
  the client looked for it on the pool, and hosts keep it per host. It is now read from
  the hosts, shown where they agree, and can be turned on and off.
- Growing, copying or cleaning up a virtual disk was refused as "in use" whenever a guest
  had the disk attached, even a halted guest: the check read a field of the attachment
  that hosts do not send. A disk attached to a guest that is not using it can now be
  grown and copied. For the same reason multipath maintenance treated every attached
  disk as active; it now looks only at disks that are plugged in.
- A cross-pool copy or move of a guest that had changed since its review (its power
  state, or the host it runs on or belongs to) was refused as "Invalid XAPI response".
  The refusal now says that the guest changed and asks for a new review.
- The performance page ran to the edge of its panel, with its button to go back
  overhanging. It has the margin the other pages have.

### Known issues

Those of 0.18.0 stand, with these changes, each from the operator's own seat on
2026-11-12:

- The details pane's width is saved, and they saw it kept.
- "Type clipboard as keys" worked for them on a graphical guest and on a text console of
  the lab. A guest with a keyboard layout other than US has still not been tried.
- In a graphical guest, dragging works and the wheel scrolls in the expected direction.

They also saw a console window follow a change of the interface size made in the main
window, and the main window follow one made in the console window.

## 0.18.0 (2026-11-11)

The first published version, a beta for XCP-ng 8.3. Its supported part is connections and
certificate trust, the live inventory, VM power, the console, snapshots, performance charts
and activity; everything else is experimental and marked so in the client
([feature scope](docs/FEATURES.md)).

The packages are a portable archive and an AppImage for x86-64 Linux, each with its
SHA-256 checksum; [installation](docs/INSTALLATION.md) says how to check and install them,
and [connections](docs/CONNECTIONS.md) how to reach a host. This is an independent
project, not an official XCP-ng or Vates application.

Stored formats: operation journal 4, connection profiles 3. Unchanged from 0.17.0.

### Added

- "Type clipboard as keys" in the console types the clipboard's text into the guest or
  host as key presses, so pasting works with any guest and in a text console. It asks
  first, giving the number of characters and line breaks and never the text; line breaks
  are sent as Enter. It takes up to 2,000 characters of what a US keyboard types
  directly, and refuses a text with anything else whole. A key press, Release input or
  leaving the console stops what is left. Punctuation comes out as written only if the
  guest uses a US keyboard layout.

### Changed

- Buttons have a visible fill or edge and follow one scheme: outlined buttons open a view
  or dismiss a dialog, filled buttons act on the selected object, red labels mark
  destructive or forced actions, and the accent colour marks the one action a view is for.
  Tabs sit in one strip with the current tab raised.
- The whole interface can be drawn from 80 to 200 percent, in Options or with Ctrl and
  plus, minus or 0. Detached console windows are not scaled yet.
- A notice names any connected host that is not XCP-ng 8.3, the only product line the
  client has been run against. It restricts nothing.
- The supported part of the client is now stated: connections and trust, the inventory, VM
  power, the console's display, keyboard and pointer, snapshots, performance charts and
  activity. Everything else is experimental, and each of its dialogs carries a notice
  saying so ([feature scope](docs/FEATURES.md)).

### Fixed

- The Clear button of the inventory search and of both Activity filters did nothing.
- The console's Actual size view showed an empty panel and took no input.
- After the window lost focus with a key held in a console, the guest could receive one
  more press and release of that key.
- Performance charts stopped after their first load: every refresh was refused as
  malformed. A host that had run HA showed no charts at all.
- Sidebar and column widths were saved only when the window closed, and lost when a
  connection was activated.
- Leaving maintenance on a host that had just rebooted could end "outcome unknown" although
  the host was enabled: the host refuses the enable while it is still booting, and the
  repeat of that call was refused by the client's own journal. The repeat is now recorded
  and followed, and the step says how often the call was repeated.
- Removing a directory user's access left that user's open sessions working. The removal
  now closes them once it is confirmed. Sessions of a removed group's members are not
  closed by it; XAPI rechecks directory sessions periodically and ends them then, which
  has not been timed on the lab.
- A request to cancel, recheck or acknowledge an operation that was still queued when a
  session was lost could make the connection report Ready, with the inventory from before
  the loss, while the new session was still loading its own.
- The refusal of a toolstack restart while tasks are unfinished now says that an open
  console counts as one.
- A saved password could not be read or written while the desktop's password store was
  locked, even when the store has no password of its own and opens for any application
  that asks. The client now asks. It does not open a store's own unlock dialog.
- After "Send clipboard" the console said the text had been sent, which read as success
  whether or not the guest could receive it.

### Known issues

- The details pane's width is not saved.
- Children scrolled or resized out of view and back are missing from the accessibility
  tree until their container is rebuilt, and controls below a dialog's fold are outside the
  Tab order until the dialog is scrolled (findings 6 and 22 of the
  [live report](docs/reports/2026-11-09-lab-qualification.md)). Screen readers are not
  supported.
- Clipboard exchange with a guest is experimental. It needs guest tools that implement
  it, did not work with the Linux guests tried, and is unverified with a guest that has
  such tools. Typing the clipboard as keys does not depend on it.
- Nobody has yet used the "Type clipboard as keys" button on a real seat; the typing
  itself reached a lab guest exactly.
- Only XCP-ng 8.3.0 has been exercised, on nested virtualization. Most workflows have a
  recorded successful path and no recorded failure or interruption path.
- Not verified: the wheel's direction and dragging in a graphical guest (the operator
  confirmed pointer movement and clicks there), non-US keyboard layouts, a person using
  the client on an X session without Wayland (a scripted check there passed) or on a
  freshly installed system (installation, software rendering and an offline start there
  are recorded in the [clean-install report](docs/reports/2026-11-11-clean-install.md)).

## 0.17.0 (checkpoint, 2026-11-11)

Stored formats: operation journal 4, connection profiles 3.

The first checkpoint with live-host evidence; see the
[0.17 checkpoint report](docs/reports/2026-11-11-checkpoint-0.17.md) for the build, its
verification and its artifacts. Its known defects are the first three entries under
"Fixed" above and the performance chart and width entries after them.

## Earlier checkpoints

0.1 to 0.16 are recorded in their own reports, linked from the README's release history.
They were verified against fixtures only.

## Stored state between versions

- The operation journal and the profile store carry a format number. A release reads and
  migrates every older format. It refuses a newer one and leaves the file as it found it:
  nothing is ever downgraded.
- Preferences have no format number. A setting an older release does not know is dropped
  the next time that release saves them.
- Going back to an older release is therefore safe only when this file lists the same
  stored formats for both, and only after unresolved operations have been rechecked or
  acknowledged in the newer one.
- A release that changes a stored format says so in its entry here.
