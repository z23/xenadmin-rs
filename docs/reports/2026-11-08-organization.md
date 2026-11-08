# Tags, folders and custom fields increment

The Organize workspace supports tag assignment/removal, tag rename/deletion across captured
pool objects, folder creation/moves/rename/removal, custom text/date definitions and values,
and removal of definitions with explicitly reviewed value cleanup. Object selection can be
filtered by name, UUID, metadata, exact tag, folder subtree or kind. Tag/folder group buttons
provide direct filters. A review is limited to 256 changes; larger sets require smaller batches.

Folders use the reference client's `other_config.folder` paths and pool `EMPTY_FOLDERS`
registry. Custom field definitions use pool `gui_config["XenCenter.CustomFields"]`; object
values use the matching `other_config` prefix. XML input is bounded, rejects DTDs and unsupported
field types, and writes escaped names. Date entry uses unambiguous `YYYY-MM-DD` with calendar
validation. Ordinary VMs, templates, snapshots, hosts, pools, SRs, networks and VDIs are included;
control domains are excluded from organization editing.

The backend admits one pool-scoped resource operation with typed edits. Every selected object
is resolved by UUID and its previous values checked before the first mutation, then again
before its setter. Shared maps preserve unrelated keys. Each step is durably dispatched and
verified; a refused later step preserves earlier changes. A lost reply retains an unknown
outcome and pool reservation. Recheck only inspects dispatched edits and never applies the
remaining plan. Definition removal rechecks all object classes for remaining values and
refuses consumers outside the reviewed cleanup.

Pool-wide UI actions require complete readable metadata. Reviews name the exact captured
objects; objects created or tagged by another client after review are not implicitly added.
XAPI map setters have no compare-and-swap transaction, so fresh preflight cannot prevent an
external writer racing a setter. Refresh captures a new review rather than silently replacing
its target list. Deleting a folder changes grouping only, never VM or disk existence.

## Verification

Seven new synthetic direct-connection fixtures cover combined edits, preservation of unrelated
maps, changed tags/definitions/object kind, partial refusal, lost replies and no replay,
definition cleanup and refusal of unreviewed consumers, public metadata and read-only access.
Two core tests exercise date/path limits and XML parsing/escaping. Four Rune tests cover
folder descendants, ordered custom-field cleanup, exact filters and the compact review path.
Compact and wide synthetic renders were inspected. Strict workspace Clippy passed.
Concurrent repeat runs also reproduced an intermittent pre-dispatch journal rejection shared
with the host slice; journal initialization is under separate investigation. It did not
dispatch the rejected change, and successful fixture runs are not evidence that this race
has been resolved.

Live host and native accessibility qualification remain deferred to the prepared disposable
lab. This increment does not implement saved/cross-pool search definitions, vApps, general
bulk power/move/export actions or extension/localization workflows; those retained rows remain
open. Test inventory and screenshots are synthetic.

## Reference evidence

[XAPI VM metadata](https://xapi-project.github.io/xen-api/classes/vm.html) and
[pool metadata](https://xapi-project.github.io/xen-api/classes/pool.html) provide the tags and map
fields/setters. The local reference XenAdmin `XenModel/Tags.cs`, `Folders.cs`, folder actions,
`CustomFields/CustomFieldsManager.cs` and `CustomFieldDefinition.cs` establish interoperable
metadata conventions. No reference implementation source was copied.
