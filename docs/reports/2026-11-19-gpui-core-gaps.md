# GPUI migration: what the front end found missing in the core

Note added later the same day: this report was written before the front end was run against the lab
and before the cutover of the code. Its sentence that nothing of the GPUI front end has run against an
XCP-ng host is superseded by the [live headless report](2026-11-19-gpui-live-headless.md); that
nothing has run in a real window still holds. The crate it calls `xenadmin-gpui` is
`crates/xenadmin-desktop` now. The gaps themselves are unchanged; the text below is as written.

Date: 2026-11-19. Branch `gpui-migration`.

The migration may not change `xenadmin-core`, `xenadmin-xapi` or `xenadmin-process` (`AGENTS.md`, "Front-end
migration"): a gap is recorded and the port continues elsewhere. This is that record. It collects the
`core_gaps` the porting engineers reported with their results, and the points of their summaries that
trace back to something the core does not offer.

Nothing below has been changed in the core, and no change is proposed as done. Each entry says where the
gap was met, what the front end does instead today, and what a change in the core would make possible.
The statements about the core are the engineers', made while porting against its public interface; the
core was not read again to write this report. Where a statement could not be confirmed from the front
end's own code or tests, the entry says so.

Nothing of the GPUI front end has run against an XCP-ng host or in a real window. "Tested" below always
means a model test or a headless test on synthetic data, or a test against the front end's local fixture
server (`xenadmin_ui::testing`, feature `test-support`).

## Summary

| # | Gap | Met in | Effect today |
| --- | --- | --- | --- |
| 1 | No public constructor or fixture for `ConnectionSnapshot`, `Connection`, `ObservedVm`, `ObservedSnapshot`, `ObservedResource`, `Operation` | kernel, every area | No plain model test has a current session; success paths are tested only where the fixture server serves the call |
| 2 | A failed `ConnectionSnapshot` with a certificate endpoint cannot be built | `connections` | The offer of `server.review-certificate` after a certificate failure is untested |
| 3 | `PermissionRequirements` has no public constructor | kernel | Tests borrow one from an operation |
| 4 | `ProfileId` has no `From<Uuid>` and no parser | kernel | The fixed demo profile id is built through serde |
| 5 | `InventorySearch` has no filter for an explicit set of objects | `search` | "Exactly these objects" is a filter applied after the query; it cannot be saved or reported |
| 6 | `Connection::control_operation` returns `Ok` for a request the worker later refuses | kernel, `events` | A refused Recheck shows the text of an accepted one |
| 7 | A virtual disk's name and description change only through a VM device command | `vm_devices` | A disk that no VM attaches has no General page |
| 8 | A console handle exists only on a connection | `console` | No test has a console stream |
| 9 | A restorable boot record needs the core's private recovery plan | `power` | The restore-boot path is tested only for "hidden without records" |
| 10 | The core guards a forced action during a migration only for Recovery commands | `power` | The front end routes Force Shut Down and Force Reboot to the task-aware review itself |
| 11 | The inventory has no "hidden object" flag | kernel | View > Hidden Objects is left out |
| 12 | Changed-block tracking exists per virtual disk only | `vm_properties` | The Advanced page of a VM has no changed-block tracking setting |

## 1. Constructors and fixtures for tests

Met first by the kernel and then by every area. `ConnectionSnapshot`, `Connection`, `ObservedVm`,
`ObservedSnapshot`, `ObservedResource` and `Operation` can be made only inside `xenadmin-core`. A test in
`xenadmin-ui` therefore cannot have a current, non-synthetic session, cannot hold an observed object,
cannot build a `VmCommand` for one, and cannot put a handle into `m.operations`.

What the front end does instead:

- Refusals are tested directly: `model::admit(demo, unresolved, connected)` for the submission choke point,
  and in each area the demo guard, staleness, "not writable" and the area's own checks.
- `xenadmin_ui::testing` (feature `test-support`) runs the real backend against a local fixture server.
  It serves a session, one VM, `session.get_record` as a local superuser, one `Async.VM.start` with its
  task, a switch that loses the reply and a switch that holds the task. The tests that need a live
  session use it (reconnect, a lost session and its renewal, power start and its unknown outcome, live
  observation in several areas).
- Everything the fixture server does not serve is untested past the point of submission. The engineers
  reported this for: placement (`Async.VM.start_on`), recovery and forced actions, batch power,
  snapshots (five dialogs, the capture and the preview query), migration and cross-pool transfer,
  storage discovery and every storage write, device and disk writes, VM lifecycle and cloud-init,
  transfers, network and management-network operations with their guest-impact queries, pool join and
  dissolution, pool security queries, host backup, restore and the status report query, schedules and
  alarms, organization metadata, vApps, bulk deletion, bulk properties, bulk export, metrics data and the
  data-source catalog. For these, capture, eligibility, review and refusal are tested; an accepted
  submission, its tracking to an outcome and a current observation going stale are not.

What a core change would allow: a constructor or builder behind a test-support feature for a current
`ConnectionSnapshot` and the observed types would let every area assert `current() == true` and its
success path in an ordinary model test, without a socket and without extending the fixture server call by
call.

A related observation, not a gap of the core: before the fixture server answered `session.get_record`,
the core treated the closed connection as unknown permissions and wrote to stderr. Since the fixture
answers it, fixture sessions have granted permissions.

## 2. A failed snapshot with a certificate endpoint

Met in `connections`. `server.review-certificate` is offered when a connection failed on a changed
certificate, which the front end reads from `failure().certificate_endpoint` of the snapshot. Such a
snapshot cannot be built outside the core, and the fixture server speaks plain HTTP.

Instead: `trust::inspect_failed` is tested by itself; the command's eligibility for a failed connection
is not. For the same reason a coordinator change is not exercised by a test (the plain-HTTP fixture
cannot be the target of a redirect).

A core change would allow: with a constructor for a failed snapshot, the entry point of the changed-pin
review could be tested end to end.

## 3. `PermissionRequirements` cannot be constructed

Met in the kernel (`permissions.rs`). The field `writes` is `pub(crate)` and there is no constructor.

Instead: tests take the requirements of a real operation, for example
`HostOperation::ExitMaintenance.permission_requirements()`.

A core change would allow: tests of the assessment with requirements chosen for the case, not borrowed.

## 4. `ProfileId` from a UUID

Met in the kernel (`session.rs`). `ProfileId` has neither `From<Uuid>` nor a parser.

Instead: the fixed id of the demo profile is built through serde (`session::profile_id`).

A core change would allow: a direct conversion. The effect is confined to that one helper.

## 5. `InventorySearch` without an explicit object set

Met in `search`. A multi-selection scopes the Search tab to "exactly these objects". `InventorySearch`
has no filter for a set of object ids.

Instead: the query runs without it and `Preset::only` filters the result keys afterwards.

Consequence for the user: such a scope cannot be part of a saved search and does not appear in the
`filters` field of an inventory report.

A core change would allow: an id-set filter in `InventorySearch` would make the scope part of the
definition, so it could be saved, exported and reported.

## 6. `control_operation` accepts what the worker refuses

Met by the kernel when operation handles were given to their sessions, and visible in `events`.
`Connection::control_operation` returns `Ok` when the request is queued. If the worker then refuses it,
for example `UnavailableAction` for a Recheck of a record without a host task, the refusal arrives only
as the error text of the snapshot. It is tied neither to the operation nor to the request.

Instead: nothing. The Events tab shows "Checking existing evidence; no mutation is replayed." for a
Recheck the core refused, and the refusal appears as the session's error line. A test pins this
behaviour with a start whose reply was lost: the core refuses its Recheck with "The requested action is
unavailable for this VM".

What the front end could do without the core: check `record.can_recheck()` in
`events::recheck_eligibility` and disable the command. That is a behaviour change and was not made.

A core change would allow: a refusal returned by the call, or reported on the operation record, would let
the row say that the Recheck was refused.

## 7. A disk's name through a VM device command only

Met in `vm_devices`. Name and description of a virtual disk are changed with
`DeviceProperty::DiskMetadata`, which needs a VBD and an observed VM.

Instead: the General page of a disk's Properties dialog exists only when a VM attaches the disk. For a
detached disk the dialog has Size and Location (and Custom Fields) but no General page.

A core change would allow: a disk command that does not go through a VM would give every disk the page
the plan lists.

## 8. No console handle without a connection

Met in `console`. A core `Console` handle exists only on a live connection.

Instead: the console's state machine, eligibility, descriptions and requests to the front end are tested
up to the handle; the input mapping is tested where it is computed (`canvas.rs`, `keys.rs`,
`scancode.rs`). `Session::send`, the recovery glue in `Session::poll` and everything a connected console
sends are not exercised. In the GPUI tests a scene's console has no stream, so no test sees a held key
or anything sent to a guest.

A core change would allow: a console handle over an in-memory stream would let the model and the canvas
be tested with frames and with recorded input.

## 9. The recovery plan of a restorable boot record

Met in `power`. A record that offers "restore boot" carries the core's private recovery plan.

Instead: `vm.recovery-restore-boot`, `review_restore_boot` and the button under the record are tested
only for the case without records (hidden). The power confirmation and the placement dialog hold an
`ObservedVm` (gap 1): the placement dialog is tested up to its review through the fixture server; the
confirmation's view is exercised only as far as its refusals.

A core change would allow: a fixture record with a plan would make the restore review testable.

## 10. Forced actions during a migration

Met in `power`. As the engineer reported it, the core applies its guard against forcing a VM while a
migration task runs to Recovery commands only.

Instead: the front end decides. Force Shut Down and Force Reboot open the task-aware review when the
ordinary action is refused and tasks are known, and always when one of the tasks is a migration. This
keeps what the Rune client did. It was not confirmed against the core for this report; the background is
`docs/reports/2026-11-12-upstream-forced-shutdown-during-migration.md`.

A core change would allow: the same guard on the ordinary forced commands would make the rule
independent of which front-end path issues the command.

## 11. No flag for hidden objects

Met in the kernel. The plan's View menu has Hidden Objects; the inventory carries no such flag.

Instead: the item is left out, not stubbed. The other four View toggles exist; they are not persisted,
because the format of the preferences file may not change.

A core change would allow: with the flag in the inventory the toggle could filter.

## 12. Changed-block tracking per disk only

Met in `vm_properties`. The plan puts changed-block tracking on the Advanced page of a VM. As the
engineer reported it, the client has that setting only per virtual disk.

Instead: the Advanced page has firmware and Secure Boot. Where the per-disk setting is offered was not
checked for this report.

A core change would allow: a VM-level operation would let the page match the plan.

## 13. One sentence for every reason a toolstack restart is unavailable

Met in `host_admin`, in the live run of 2026-11-19
([live paths report](2026-11-19-gpui-live-paths.md), `lab_p3_member_toolstack_restart`). The server's
text console was open in the Console tab, which the host counts as a pending task. The core's
availability of `HostOperation::RestartToolstack` then says "Pool HA must be known disabled, a current
toolstack start marker must be supplied and pool tasks must be finished": one sentence for three
conditions, without saying which one holds. The sentence that names the cause ("Finish pending pool
tasks before restarting the toolstack. An open VM or host console counts as one") exists in the core
only as the refusal of a submitted operation, which the front end never reaches because the
availability already refused.

Instead: the Server menu shows Restart Toolstack disabled with the core's sentence as its reason. The
user is told that pool tasks must be finished and is not told that their own open console is one.

A core change would allow: an availability that names the condition that failed (or exposes the
pending tasks it counted) would let the menu say "close the open console first".

## Not gaps of the core

Reported beside the above, and belonging to the front end:

- A failed file prompt and a cancelled one were indistinguishable to the model. The kernel has since
  added `FileRequest::on_failure`; some Rune messages ("Cannot open save dialog: ...") still have no
  successor where an area does not use it.
- `m.open_url` has no failure reply, so a link that fails to open is not reported in the dialog.
- Resource operations have no submission choke point (Rune had none) and merge only the history of the
  scope session.
- There is no object kind for a custom field, so custom fields are not a heading of the Organization view.
