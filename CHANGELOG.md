# Changelog

Each entry states what changed for someone running the client, what is known to be wrong
in that build, and the stored formats it reads and writes. Evidence for each entry is in
the linked reports; the [retained parity ledger](docs/PARITY-IMPLEMENTATION.md) remains the
authority on what is implemented and what is qualified.

0.18.0 is the first published version. Versions up to 0.17.0 were local development
checkpoints.

## 0.20.0 (in development)

Stored formats: operation journal 4, connection profiles 3. Unchanged from 0.19.0.

### Changed

- A cross-pool copy or move that the client declines because of the guest itself (a
  snapshot chain, a vTPM, HA protection, vApp membership, installation media left in a
  drive, a disk that is shared or has other consumers) or because the host does not
  allow the migration is refused in those words. It used to be reported as "Invalid
  XAPI response" followed by them, as if the host had answered wrongly. The cross-pool
  workflows remain experimental.
- Inside, 177 more reads of host records go through the generated bindings, leaving 472
  by key ([decision record](docs/decisions/0004-generated-bindings.md)). Nothing in the
  client's behaviour changes with them.
- Inside, debug builds can compare every call they send with the API: how many arguments
  it takes, which of them may be left out, and the type of each. The repository's tests
  run with that check and it reported nothing. Release builds contain none of it.

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
