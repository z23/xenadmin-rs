# Temporary desktop ownership release

A default-concurrency checkpoint-workspace run failed while reacquiring an ordinary
`DesktopInstance` immediately after dropping it. The original failure was in
`desktop_activation_reclaims_only_stale_owned_socket_and_preserves_replacements`, at its
second acquisition. No activation acknowledgement or callback timeout was involved.

The guard previously closed its lock descriptor without explicitly unlocking. Linux flock
ownership belongs to the open file description: a duplicate, including a descriptor
temporarily inherited by a concurrent fork before close-on-exec, can keep that lock held.
The failing run did not trace which child inherited a descriptor. A deterministic regression
instead duplicates the descriptor, drops the guard, and checks immediate reacquisition while
the duplicate remains open. This reproduces the lifetime defect without process timing.

Drop now joins the activation service and explicitly unlocks an ordinary owner. The actual
desktop's `retain_until_process_exit` path deliberately does not unlock; its existing
cross-process fixture still proves that dropping the UI guard cannot admit a second owner.

All ten focused desktop ownership/activation fixtures and strict core all-target Clippy
passed. The broader checkpoint workspace rerun is recorded with that increment. No live
desktop, host session or production resource was used.
