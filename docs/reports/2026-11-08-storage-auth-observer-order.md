# Authentication recovery fixture ordering

This fixture-only increment removes an unsupported worker-retirement assumption
from the delayed storage Secret response test. Application behavior, startup
reconciliation, reservations and permission handling are unchanged.

The original worker publishes positive Secret completion evidence before finishing
its identity read and final evidence merge. A retained Secret in its public handle
therefore does not establish that the worker has retired. Starting a read-only
recheck at that point can race the final merge: the recheck's terminal save may
lose its revision comparison and conservatively retain `OutcomeUnknown`. Waiting
unconditionally for `PartiallyCompleted` after that race was invalid.

The facade fixture still holds the original reply until automatic recovery has
reported the missing receipt, and still verifies both unknown and acknowledged
unknown late-receipt outcomes, retained Secret identity, one Secret creation, and
zero storage-provisioning or Secret-cleanup calls. It no longer combines that race
with a second assertion about worker retirement.

A new worker fixture uses one-shot gates and an explicitly joined late-evidence
task. It deterministically demonstrates the first-publication/final-merge conflict,
verifies that the local reservation remains held, then rechecks after the writer
has finished. That recheck reports a known partial result with the same retained
Secret and makes no RPCs. No sleeps or accept-either-state assertions are used for
the worker ordering. History timeout diagnostics now report the caller location
and observed public state/step/Secret projections in synthetic tests.

The original integrated timeout had no caller-phase diagnostic. Its exact failing
phase was not reproduced: eight complete diagnostic protocol runs and 200 focused
repetitions passed before the fixture change. A ninth repeated whole-suite run
encountered broad connection/timeouts across 152 tests and was stopped; this is not
treated as evidence for the specific observer race. The deterministic worker case
establishes the invalid scheduling assumption independently.

Verification: the new worker case and all nine authentication protocol fixtures
pass; strict core all-target Clippy, formatting and whitespace checks pass. All
fixtures use synthetic loopback servers. No real host was contacted or mutated.
