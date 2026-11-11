# Generated XAPI bindings in a separately licensed crate

The client's XAPI calls and record decoders were written by hand: a few hundred call
names as strings and some two thousand reads of record fields by key. The plan's first
milestone asked for bindings generated from an API description that matches the hosts,
and that was put off until after the first release (0.18.0). This record covers the
description, how its licence is handled, what is generated and what follows.

## The description

`doc/data/xenapi.json` in the upstream repository `xapi-project/xen-api` is the
machine-readable output of the upstream API generator, kept under version control there.
It lists 67 classes with 845 fields and 1,828 messages, each with its types, parameters,
result, errors, permitted roles and lifecycle.

It is taken at tag `v26.1.11`, commit `53cf465262b2236a29c34dafb2ae1ccfd6a40ac0`. The
lab's XCP-ng 8.3.0 hosts report toolstack 26.1.11 and API version 2.21, so it describes
the API of the hosts the client is qualified on. No OCaml source is parsed and nothing is
built from upstream.

What it does not say:

- Whether a message has an asynchronous form. The host decides that, and finding 19 of the
  live report (a call with no asynchronous form) could not have been read from it.
- What XCP-ng adds to XAPI. `VM.get_secureboot_readiness` and
  `pool.get_guest_secureboot_readiness` answer on the lab and are absent upstream.
- What a host of another version offers. A second host version needs its own description
  and a recorded difference.

## Licence: one LGPL crate, the rest MIT

The upstream repository is distributed under the LGPL 2.1 with a linking exception; this
project is under the MIT licence. The operator first asked that LGPL material be kept out
of the repository, and for a day the bindings were hand-written and only checked against
a fetched copy of the description. On 2026-11-11 the operator then decided, after the
options were laid out, to take generated bindings and to keep everything derived from the
description in one crate under upstream's terms, as a library the MIT-licensed program
uses, with that crate replacing the hand-written bindings. The earlier attempts are not
on the main branch.

`crates/xenadmin-xapi` is that crate:

- Its `LICENSE` is upstream's licence file at the pinned commit, unchanged: the linking
  exception followed by the LGPL 2.1. Its manifest declares
  `LGPL-2.1-only WITH OCaml-LGPL-linking-exception`.
- Its `NOTICE` says what the crate is derived from and where the complete source is.
- `data/xenapi.json` is the description, unchanged, and `data/SOURCE` records the
  repository, tag, commit, path and SHA-256. The build refuses any other file.
- The generator and the few hand-written lines of the crate are under the same terms.

Nothing derived from the description may live outside that crate, and the other crates
stay under the MIT licence. Packages carry the crate's licence and notice with the other
dependency notices, under `third-party/xenadmin-xapi-VERSION/`, and the source for every
release is public at its tag. This section records how the licence is handled; it is not
legal advice, and the operator has said they do not take it as such.

## What is generated

The crate's build script generates Rust from the description each time the crate is
built, so there is no generated file to drift and none in the repository. The output
depends on the description alone and is sorted.

- A catalogue: every class with its fields, messages and enumerations as static data.
- A record view for every class: a wrapper around the record a host sent, with one method
  for each field of that class. A read through a view says which class it is of, and a
  field the class lacks has no method. The methods return the value as sent, so bounds
  and the refusal of malformed values stay with the core's decoders.
- Constants for every enumeration value, such as the four VM power states.

## Use in the core so far

- The event decoder, the power and snapshot checks and the inventory decoder read their
  records through the generated views: 331 reads in those files, where a field that its
  class lacks does not compile. The 86 reads by key that remain there are keys inside
  map-valued fields, the JSON-RPC envelope, loops that handle several classes at once,
  and two decoders (network fabric and management topology) that have not been moved.
- VM power states are compared with the generated constants in the event decoder.
- `xapi_conformance_tests.rs` checks every string of the form `Class.name` or
  `Async.Class.name` in the core against the catalogue: a call must be a message of that
  class, and a field named by a decoder must be a field of it. Names the description
  lacks are listed in the test with the reason each is used, and an entry that no longer
  applies fails the test.

What the move found:

- Every call and field the client names exists, with two XCP-ng additions as the only
  exceptions.
- One fixture test asserted on `VM.reset_power_state`, a name the API does not have
  (`VM.power_state_reset` is the call), so that assertion could never have failed.
- The pool's HTTPS-only setting was read from a pool field that does not exist: hosts
  keep it. The fixture carried the field, so tests passed, and against a real pool the
  setting could never be shown or changed. The read did not compile against the pool's
  view (finding 55 of the live report).

What the move did not protect against: the readers that take a field's value still take
a JSON value, as a whole record is. One call that passed the record where its UUID was
expected compiled, and a test caught it. Typed values, step 3 below, are what remove
that.

## What follows, in order

Measured on 2026-11-11, the core reads 254 distinct keys in 2,069 places; 190 are field
names somewhere in the description and 64 are keys inside map-valued fields, parts of the
JSON-RPC envelope or the client's own structures.

1. The remaining decoders move onto views: network fabric, management topology and the
   loops that handle several classes, then the operation modules, which hold most of the
   1,883 reads by key still in the core.
2. Enumeration constants replace the remaining literal values.
3. Typed values: generated readers that turn a field into its Rust type, with the core's
   bounds applied to the result.
4. Calls: generated descriptions of each message's parameters, so that a call's argument
   count and order are checked where it is made.
5. The experimental areas, class by class, each with the fixture tests that cover it.

A workflow whose decoding moves onto the generated crate is run again on the lab before
it is called qualified on it: the live evidence gathered before is not carried over by
assumption.
