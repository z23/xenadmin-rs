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
  is verified by fixtures and by one run on the lab, where a disk snapshot and a
  checkpoint with its memory were exported in one batch and read back; nobody has used
  the dialog ([report](docs/reports/2026-11-12-batch-snapshot-export.md)).
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
  acknowledged. The batch is experimental. It is verified by fixtures and by one lab
  run in which two halted guests were moved to another pool and back; a running guest,
  Stop and an interruption have not been tried against a host, and nobody has used the
  dialog ([report](docs/reports/2026-11-12-cross-pool-bulk-move.md)).

### Changed

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
  same host differs at two checks running, the events that keep the inventory current are
  not arriving, and the whole inventory is read again; the connection shows itself as
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
- Inside, 177 more reads of host records go through the generated bindings, leaving 472
  by key ([decision record](docs/decisions/0004-generated-bindings.md)). Nothing in the
  client's behaviour changes with them.
- Inside, a field read through the bindings has the type of its kind, so a flag, text, a
  reference, a number, a set and a map cannot be compared or handed to a reader as one
  another, and a test checks that each is read as the kind the API gives it. Nothing in
  the client's behaviour changes with this.
- Inside, debug builds can compare every call they send with the API: how many arguments
  it takes, which of them may be left out, and the type of each. The repository's tests
  run with that check and it reported nothing. Release builds contain none of it.

### Fixed

- A disk snapshot that the host does not offer for export, because it is busy or the
  export is blocked on it, is no longer shown as ready to export and then refused when
  the transfer is prepared. It is shown as unavailable, as a VM and a memory checkpoint
  already were.

### Known issues

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
  undone: on the lab a VM that the client had deleted and verified gone was back as a
  record without disks, the pool's database not having been written out in between
  (finding 65). The client's history still shows the deletion as verified, which it
  was at the time.

And one that a fixture test showed while the batch of snapshot exports was written:

- An export in a batch, of a VM or of a snapshot, that is interrupted after its task
  was created on the host and before anything was transferred stays unknown. A recheck
  does not settle it, although nothing was written, and it has to be acknowledged.

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
