# Resource operation record stack usage

Private, mutually exclusive resource workflow plans now use `Option<Box<Plan>>`.
Their public serialization remains skipped, and the private journal JSON, schema,
validation, admission and recovery rules are unchanged.

A synthetic nested batch/export worker exposed stack exhaustion in the debug
build while decoding a child and validating its parent. The pre-change record
was 6,640 bytes on x86-64 in the pending export branch (including its ownership
field); after boxing, this representation-only branch's record is 1,792 bytes. The new regression keeps the record
below 4 KiB. The pending export branch separately exercises the original failing
nested worker; the size assertion alone does not prove all stack paths.

Validation: all 1,163 existing workspace tests passed (517 core unit, 311 protocol,
322 Rune, five profile, four TLS and four host-probe tests); strict workspace
Clippy passed. Fixtures use local synthetic servers and temporary files. No host
or guest connections were made.
