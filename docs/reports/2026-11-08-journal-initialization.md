# Journal initialization concurrency — 2026-11-08

A synthetic organization workflow intermittently failed before dispatch with the generic
journal error. Stage-only instrumentation reproduced SQLite code 5 (`SQLITE_BUSY`) during
WAL/durability setup: the sixth parallel seven-test group failed, and the preceding group
also recorded a failed history open. No live host was used.

A separate deterministic test exposed a locking violation in the same initialization path.
With a SQLite transaction held open, a child process could not obtain a conflicting POSIX
lock. Calling `Journal::open` for another connection opened and immediately closed the
existing database through `std::fs`; afterward the child could obtain that lock. The test
failed on the old implementation and passes after the change. SQLite documents that a
non-SQLite `close()` can cancel locks owned by other connections in the process. See
[SQLite locking guidance](https://www.sqlite.org/howtocorrupt.html#_posix_advisory_locks_canceled_by_a_separate_thread_doing_close_).

Initialization now creates a new database with mode 0600 using `create_new`; it never opens
an existing database through a second file API. A process mutex orders creation, SQLite open,
WAL setup and migration, so another thread cannot open the new file before its creator closes
the initial descriptor. SQLite continues to coordinate actual transactions. Existing WAL
mode is read and retained; the mode setter runs only when conversion is needed. Full
synchronous durability is configured per connection. Schema versions, migrations and
reservations remain unchanged; no new retry or sleep loop was added.

Initialization errors emit only a fixed stage and numeric SQLite/OS code on local stderr.
They do not emit SQL, paths, record content, sessions or credentials. The public error
remains unchanged. The lock test establishes the descriptor bug directly; the stress
reproduction establishes the original failure stage, not that descriptor closure alone
caused every observed `SQLITE_BUSY` failure.

Verification:

- A child-process lock probe fails on the old path and passes on the corrected path.
- Eight concurrent workers initialize journals, admit/complete 64 operations and read
  history; all records survive, file mode remains 0600 and SQLite integrity check passes.
- `cargo test -p xenadmin-core`: 227 passed (141 unit, 77 direct protocol, five profile
  and four TLS tests). Strict all-target core Clippy, formatting and diff checks passed.
- The identical organization fixture group passed 100 consecutive rounds (700 tests) on
  isolated current main with the fix, with no initialization diagnostic or failed test.

No application data or existing journal was removed. All database fixtures were temporary.
