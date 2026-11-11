# XAPI bindings checked against the upstream description

The client's XAPI calls and record decoders are written by hand: a few hundred call names
as strings and some two thousand reads of record fields by key. The plan's first
milestone asked for bindings generated from an API description that matches the hosts,
and that was put off until after the first release (0.18.0). This record covers the
description, what the operator decided about it, the first increment and what follows.

## The description

`doc/data/xenapi.json` in the upstream repository `xapi-project/xen-api` is the
machine-readable output of the upstream API generator, kept under version control there.
It lists 67 classes with 845 fields and 1,828 messages, each with its types, parameters,
result, errors, permitted roles and lifecycle.

It is pinned in `crates/xenadmin-core/xapi-description.pin` at tag `v26.1.11`, commit
`53cf465262b2236a29c34dafb2ae1ccfd6a40ac0`, with its SHA-256. The lab's XCP-ng 8.3.0 hosts
report toolstack 26.1.11 and API version 2.21, so it describes the API of the hosts the
client is qualified on. No OCaml source is parsed and nothing is built from upstream.

What it does not say:

- Whether a message has an asynchronous form. The host decides that, and finding 19 of the
  live report (a call with no asynchronous form) could not have been read from it.
- What XCP-ng adds to XAPI. `VM.get_secureboot_readiness` and
  `pool.get_guest_secureboot_readiness` answer on the lab and are absent upstream.
- What a host of another version offers. A second host version needs its own pin and a
  recorded difference.

## Nothing derived from it is stored

The upstream repository is distributed under the LGPL 2.1 with a linking exception; this
project is under the MIT licence. The operator decided on 2026-11-11 to keep LGPL material
out of this repository where that can be avoided. So the description is not copied here,
and neither is anything produced from it by a program: no generated catalogue, records or
call wrappers are checked in. A first version of this work generated a catalogue of names
and types into the source tree; it was withdrawn before it reached the main branch.

That rules out generated bindings as first planned, because generated code has to exist
where the client is built. What remains is the benefit that mattered: a name the API does
not have is caught before a host has to refuse it. The bindings stay hand-written and are
checked against the description, which `scripts/xapi-description.sh` fetches into
`target/`, verifies against the pinned digest and leaves out of the repository. This
paragraph records a choice and is not legal advice.

Sources under a permissive licence exist and were not used: the vendor's SDK and the
generated bindings inside the reference XenAdmin source are BSD-licensed. They describe
the API of their own release, not of the lab's toolstack, and would need a parser for
generated source. They are an option if generated code is wanted later.

## First increment

- `xapi_conformance_tests.rs` reads the crate's own sources and the fetched description.
  Every string of the form `Class.name` or `Async.Class.name` must be a message of that
  class, or a field of it where a decoder names one. Names the description lacks are
  listed in the test with the reason each is used, and an entry that no longer applies
  fails the test.
- `xapi_records.rs` holds record views: a view wraps the record a host sent and has one
  method for each field the client reads from that class. A read through a view says which
  class it is of, and a field that is not listed cannot be read. The methods return the
  value as sent, so bounds and the refusal of malformed values stay with the decoders. The
  check compares every view's fields with its class in the description.
- The event decoder and the power and snapshot checks in `xapi.rs` read VM, host and task
  records through views. Of its 58 reads by key, the seven that remain are parts of the
  JSON-RPC envelope, a key inside a map-valued field and a helper shared by several
  classes.

The check script fetches the description and then requires the check; without a network
the fetch is reported and the check is skipped, and everything else still runs. CI caches
the fetched file by the pin.

On its first run the check passed for every call and field the client names, with the two
XCP-ng additions as its only exceptions. It found one fixture test asserting on
`VM.reset_power_state`, a name the API does not have (`VM.power_state_reset` is the call),
so that assertion could never have failed.

## What follows, in order

Measured on 2026-11-11, the core reads 254 distinct keys in 2,069 places; 190 are field
names somewhere in the description and 64 are keys inside map-valued fields, parts of the
JSON-RPC envelope or the client's own structures. A key alone does not say which class
its record is, which is why the reads move onto views.

1. Views for the remaining classes the supported core reads (pool, VM metrics and guest
   metrics, console, session), then the inventory decoder.
2. Enumeration values (power states, operation names) checked the same way, so that a
   mistyped value is caught as a mistyped field is.
3. Calls: parameter counts and order checked against the description's messages.
4. The experimental areas, class by class, each with the fixture tests that cover it.

A workflow whose decoding moves onto views is run again on the lab before it is called
qualified on them: the live evidence gathered before is not carried over by assumption.
