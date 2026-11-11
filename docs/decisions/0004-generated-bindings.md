# Generated XAPI bindings in a separately licensed crate

The client's XAPI calls and record decoders were written by hand: a few hundred call
names as strings and some two thousand reads of record fields by key. The plan's first
milestone asked for bindings generated from an API description that matches the hosts,
and that was put off until after the first release (0.18.0). This record covers the
description, how its licence is handled, what is generated and what follows.

## The description

The upstream repository `xapi-project/xen-api` holds the API's data model as OCaml source
under `ocaml/idl`, and a program of its own, `ocaml/idl/json_backend/gen_json.ml`, that
writes that model out as JSON: every class with its fields and messages, each with its
types, parameters, result, errors, permitted roles and lifecycle.

The crate's description, `data/xenapi.json`, is what that program writes at tag
`v26.1.11`, commit `53cf465262b2236a29c34dafb2ae1ccfd6a40ac0`: 70 classes with 899 fields
and 1,943 messages. The lab's XCP-ng 8.3.0 hosts report toolstack 26.1.11 and API version
2.21. `data/regenerate.sh` clones the tag, builds the program with the distribution's
OCaml and runs it. The program was built twice in a throwaway Ubuntu 24.04 virtual
machine, the second time by that script from an empty package root, and wrote the same
file each time. The tool versions are recorded in `data/SOURCE`. Nothing of upstream's is
built when the client is built: the file is kept, and the build checks its digest.

The first description used here was a different file, `doc/data/xenapi.json`, which
upstream keeps under version control as a copy of that same output. At the tag it had
last been changed in May 2024 and named no release newer than 24.10.0: 67 classes, 845
fields and 1,828 messages. It was taken on the assumption that the file at a tag
describes that tag, and that was wrong (finding 56 of the live report). A read-only
comparison of the lab's live records with it, `lab/description_check.py`, showed 28
fields in six classes that hosts send and it lacked; against the generated description
the same comparison shows no difference in either direction, over the 27 classes that
have records on the lab. Two calls the older file lacked had been listed as XCP-ng's own
additions; they are upstream's and are in the generated description.

What it does not say:

- Whether a message has an asynchronous form. The host decides that, and finding 19 of the
  live report (a call with no asynchronous form) could not have been read from it.
- What a host of another version offers. A second host version needs its own description
  and a recorded difference.
- Whether a class the lab has no records of matches: 31 classes could not be compared,
  and three the host refuses to list.

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
- `data/xenapi.json` is the description as upstream's program wrote it, and
  `data/SOURCE` records the repository, tag, commit, program and SHA-256. The build
  refuses any other file. `data/regenerate.sh` is how it is produced.
- The generator and the few hand-written lines of the crate are under the same terms.

The description was at first a file copied unchanged from upstream's repository, which
is what the operator was shown when deciding. Since finding 56 it is the output of
upstream's own program run on upstream's data model at the same commit. It is derived
from the same LGPL-licensed source, stays in the same crate under the same terms, and
the notice says how it is produced. The change was reported to the operator with the
work and waits for their word before it reaches the main branch.

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
- A type for one field of a record, `FieldValue`, which every view method returns and
  which reads as the JSON value it wraps. The core's readers of a field take that type,
  so a whole record handed to one of them does not compile.
- Constants for every enumeration value, such as the four VM power states.

## Use in the core so far

- The event decoder, the power and snapshot checks, the inventory decoder and the
  network fabric and management topology decoders read their records through the
  generated views, where a field that its class lacks does not compile.
- The readers those decoders share (a required string, a label, a size, a reference and
  the like) take a `FieldValue`. A read that has no view, such as a key inside
  `other_config`, wraps its value by hand with `FieldValue::of`, which is visible where
  it is done.
- The operation modules read through views wherever the class of a record variable was
  established: by hand for the snapshot, power, console and recovery modules, and for the
  rest from the fields themselves, a variable being given a class when every field read
  from it belongs to exactly one class: first taking a function's reads of a name
  together, then taking each binding of the name (a `let`, a loop or closure parameter)
  with the reads it governs, which separates records of different classes held under one
  name in turn. The second kind were each read against the call that binds them. Each
  read names its view where it
  is made (`VmRecord::new(&record).power_state()`), so a view is never held across a
  point where the variable is bound to another record.
- VM power states are compared with the generated constants in the event decoder.
- `xapi_conformance_tests.rs` checks every string of the form `Class.name` or
  `Async.Class.name` in the core against the catalogue: a call must be a message of that
  class, and a field named by a decoder must be a field of it. It has a list for calls
  the description lacks, which is empty, and an entry that no longer applies fails the
  test.

What the move found:

- Every call and field the client names in that form exists.
- One fixture test asserted on `VM.reset_power_state`, a name the API does not have
  (`VM.power_state_reset` is the call), so that assertion could never have failed.
- The pool's HTTPS-only setting was read from a pool field that does not exist: hosts
  keep it. The fixture carried the field, so tests passed, and against a real pool the
  setting could never be shown or changed. The read did not compile against the pool's
  view (finding 55 of the live report).
- A disk attachment has no `reserved` field in the API, and two checks read one. Absent,
  it compared as set, so growing, copying or cleaning up a disk was refused whenever any
  guest had it attached, plugged or not, and multipath maintenance counted every
  attachment as active. Again the fixtures carried the field (finding 57).
- Two guards could never fire: a network interface has no `trunks` field and a disk no
  `blocked_operations` (finding 58).
- The description itself was two years older than the toolstack it was taken to describe
  (finding 56, above).

The last three came from looking at the variables no class fits. A variable whose fields
fit a class but for one is either two records under one name or a wrong field name.

What the views alone did not protect against: a reader that takes a field's value took
a JSON value, as a whole record is. One call that passed the record where its UUID was
expected compiled, and a test caught it. `FieldValue` closed that: the same call, put
back as a check, is refused by the compiler ("expected `FieldValue`, found `&Value`").
A field of the wrong class or the wrong meaning passed to a reader still compiles;
typed values, step 3 below, narrow that further. A variable given the wrong class
compiles too when that class has the fields read; the assignment from fields cannot
make that mistake where one class fits, and the assignments by hand were read against
the calls that produce each record.

## What follows, in order

The count used here is of indexes by a string literal on an expression (`record["key"]`)
in the core's sources outside its test files, which is how a field is read without a
view. Before the move the core had 2,094 of them over 252 distinct keys; on 2026-11-11
it has 646 over 185. Not all of them are fields: some are keys inside map-valued
fields, parts of the JSON-RPC envelope, disk image and archive metadata or the client's
own structures.

1. The reads by key that are left. Most are where the class of a record depends on a
   value at run time (network and storage operations read "the target's record", whose
   class follows the kind of target), where the fields read are too few to tell the
   class (a `uuid` alone), or in helpers that take a record of any class; each needs
   its class from the call that produced the record, or a helper per class. The loops
   that handle several classes at once stay by key.
2. Enumeration constants replace the remaining literal values.
3. Typed values: generated readers that turn a field into its Rust type, with the core's
   bounds applied to the result.
4. Calls: generated descriptions of each message's parameters, so that a call's argument
   count and order are checked where it is made.
5. The experimental areas, class by class, each with the fixture tests that cover it.

A workflow whose decoding moves onto the generated crate is run again on the lab before
it is called qualified on it: the live evidence gathered before is not carried over by
assumption.
