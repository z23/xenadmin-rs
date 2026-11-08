# VM operation serialization privacy

Public serialization of a VM operation now excludes raw result references, private
snapshot/device/lifecycle/migration plans, clone baselines and cancellation-dispatch
evidence. Public outcomes and created-object UUIDs remain available. Activity
exports continue to use their explicit public projection.

The private journal stores the same flat payload fields as before. Loading,
history, restart recovery and admission all restore them through one decoder;
the separate SQLite task column remains authoritative. There is no schema change.
Malformed private evidence fails closed rather than being discarded.

Three synthetic regression tests cover public serialization versus durable
recovery, legacy flat payloads and authoritative task columns, and corrupt private
plans. The recovery test also verifies that an unresolved created VM continues
to reserve its identity against conflicting operations.

Validation used a dedicated target directory to prevent cross-worktree artifact
collisions: all 410 workspace tests passed, strict workspace Clippy passed, and
formatting and diff checks passed. No hosts were contacted and no private
screenshots or inventory were copied.
