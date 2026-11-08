# Concurrent cancellation and stopped resource workers

A synthetic XVA cancellation check exposed a journal race: a cancellation update
could win the final compare-and-swap while the stream worker exited, leaving the
durable operation labelled Tracking with no remaining tracker. The pool reservation
was held, but the client no longer described the recovery state accurately.

The exiting worker now reloads the latest journal row and records OutcomeUnknown
through a bounded compare-and-swap loop. Positive resource/task evidence and the
cancellation request survive; terminal acknowledgement is never reversed. A
cancellation observed before dispatch remains definite. An explicit recheck can
forward a still-unsent cancellation request once and inspect the existing task;
it never repeats the upload.

The same audit found that generic late-receipt merging could revive a recovered
unknown row. That merge now retains the unknown state, and original workers stop
before another step or after receiving a late task/result. Explicit recovery
workers can still verify known results. This includes the generic resource path,
the transfer task-creation boundary and the pool-join completion receipt; it is not
a redesign of the separate host or transfer plan engines.

Verification passed 434 core tests: 273 unit tests, 152 direct-connection fixtures,
five profile fixtures and four TLS fixtures. Two new deterministic tests cover
worker exit in Dispatching, Tracking, Verifying and CancellationRequested states,
late positive evidence, reservation retention, acknowledgement and cancellation
before dispatch. The XVA fixture now requires durable unknown state before an
explicit recheck, then checks one cancellation RPC, retained IDs and one upload.
Existing lost-reply/restart fixtures also verify that failed explicit rechecks
still publish their outcome. No production host was accessed.
