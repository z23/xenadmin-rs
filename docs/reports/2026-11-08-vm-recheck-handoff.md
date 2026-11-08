# Retain VM rechecks while the previous worker exits

An integrated workspace run exposed a real scheduling race in ordinary VM
operations: the operation handle could publish OutcomeUnknown before the
connection actor removed its worker. A Recheck arriving in that interval was
validated but never started. The existing lost-settings-reply fixture timed out
waiting for success; an isolated rerun passed, consistent with the scheduling gap.

The connection now queues that explicit recheck by operation ID. After the last
existing worker exits, it reloads the journal and starts a read-only observer only
if the operation remains nonterminal, recheckable and independently owned.
Repeated requests coalesce. An intervening acknowledgment or other terminal result
prevents the queued observer from starting. Connection teardown drops the local
queue; journal recovery remains authoritative on reconnect.

The existing direct-protocol fixture now repeats the immediate Recheck sequence
sixteen times and still requires success with exactly one VM setter per run.
That regression passed. The original failing full-run log was retained locally;
no deadline or outcome assertion was relaxed. Combined workspace verification is
recorded in the following integration report.
