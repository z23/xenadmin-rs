# Cross-pool and saved searches

Implemented locally: one search across readable inventories of connected pools, typed
pool/object identities, name/UUID/description/host text matching, object-type and power
filters, selected pool scope, exact required tags, folder descendants and exact custom
fields. Results can be ordered by name or grouped by pool, type or folder. Control domains
are excluded; templates, snapshots, disks, storage and networks are separately identified.

Aliases for the same pool contribute one inventory. A current connection is preferred,
then the selected profile; revision counters from different sessions are never compared.
Stale and incomplete inventory coverage is displayed. Unknown metadata never satisfies
a tag/folder/custom-field predicate. Result navigation rechecks the pool and exact object,
including a snapshot's ordinary parent VM and a disk's existing SR. Missing/orphaned
results produce an error instead of navigating to another object.

Saved filters persist privately with application preferences. Users can save/update/remove
definitions and import/export versioned JSON through native file pickers. Imports require
review, preserve existing definitions and refuse conflicting names/identities. Files contain
filters only, not result inventory, sessions or credentials. Import is bounded to 1 MiB,
100 definitions, bounded individual filters, regular files and no symlinks. Picker callbacks
from closed/replaced workspaces are discarded. Exports use the existing private atomic
file writer; persistence failures restore the previous in-memory catalogue.

Local evidence: four pure core filter/catalogue tests, Rune wide/compact interaction tests,
alias and deleted/orphaned snapshot navigation regressions, bounded file reader and native
picker cancellation/stale-callback checks. Full pre-review workspace suite passed 506 tests;
subsequent navigation/alias regressions and strict all-target Clippy passed separately.
Wide/compact native Vulkan renders were inspected using synthetic inventory only.

No live pool connections or infrastructure changes were used. Real portal behavior,
large live inventories and live navigation remain unqualified. Legacy XenCenter search
XML is explicitly rejected; interoperability with that format and resource-result export
remain open. Multi-selection, context menus and bulk actions are separate outstanding work.
