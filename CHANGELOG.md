# Changelog

Each entry states what changed for someone running the client, what is known to be wrong
in that build, and the stored formats it reads and writes. Evidence for each entry is in
the linked reports; the [retained parity ledger](docs/PARITY-IMPLEMENTATION.md) remains the
authority on what is implemented and what is qualified.

Nothing has been published. Versions up to 0.17.0 are local development checkpoints.

## 0.18.0 (in development)

Stored formats: operation journal 4, connection profiles 3. Unchanged from 0.17.0.

### Changed

- Buttons have a visible fill or edge and follow one scheme: outlined buttons open a view
  or dismiss a dialog, filled buttons act on the selected object, red labels mark
  destructive or forced actions, and the accent colour marks the one action a view is for.
  Tabs sit in one strip with the current tab raised.
- The whole interface can be drawn from 80 to 200 percent, in Options or with Ctrl and
  plus, minus or 0. Detached console windows are not scaled yet.
- A notice names any connected host that is not XCP-ng 8.3, the only product line the
  client has been run against. It restricts nothing.

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

### Known issues

- The details pane's width is not saved.
- Children scrolled or resized out of view and back are missing from the accessibility
  tree until their container is rebuilt, and controls below a dialog's fold are outside the
  Tab order until the dialog is scrolled (findings 6 and 22 of the
  [live report](docs/reports/2026-11-09-lab-qualification.md)). Screen reader use is
  unverified.
- Only XCP-ng 8.3.0 has been exercised, on nested virtualization. Most workflows have a
  recorded successful path and no recorded failure or interruption path.
- Not verified: a person's pointer in a graphical guest through the desktop (presses sent
  by the test harness do land where aimed), clipboard exchange with a guest,
  non-US keyboard layouts, a standalone X server, and a person using the client on a
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
