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
the notice says how it is produced. The operator was told of the change and accepted
upstream's generator output as the description on 2026-11-11, and it went to the main
branch then.

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
- A type for one field of a record, which every view method returns and which reads as
  the JSON value it wraps. The core's readers of a field take such a type, so a whole
  record handed to one of them does not compile. At first there was one type,
  `FieldValue`; since 0.19.0 there is one for each kind of field the API has (a flag,
  text, a time, a reference, an enumeration's value, a whole number, a real number, a
  set, a map), and `FieldValue` remains for a kind without a type and for where the kind
  does not matter.
- Constants for every enumeration value, such as the four VM power states.
- A check a view makes of its record in debug builds, when asked to. A record does not
  say which class it is of, but it carries every field of its class, so a record of
  another class has keys the view's class lacks. With the environment variable
  `XENADMIN_XAPI_VIEW_AUDIT` naming a file, each view writes there the keys of its
  record that its class lacks and the place the view was made: names only, never a
  value. Release builds do not contain the check.

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
- Enumeration values are written as the generated constants: 247 literals in
  comparisons and match arms on enumeration fields (power states, task states, disk and
  attachment types, address modes and the like), and 72 operation names passed to the
  helpers that ask whether a record offers an operation, each from the enumeration of
  that record's class. A value its enumeration does not have has no constant. A further
  276 followed on 2026-11-13, in the client's own types and in what it compares them
  with (item 2 below).
- `xapi_conformance_tests.rs` checks every string of the form `Class.name` or
  `Async.Class.name` in the core against the catalogue: a call must be a message of that
  class, and a field named by a decoder must be a field of it. It has a list for calls
  the description lacks, which is empty, and an entry that no longer applies fails the
  test. Where a call is written with its arguments beside it, which 362 are, it also
  checks that the call passes as many as the API gives the message; all do.

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

- The check of an installation image compared a disk's type with `iso`, which is not
  one of the API's disk types, so that test never held; the fixture carried such a
  disk (finding 60). It was the one literal of the 247 that was not a value of its
  field's enumeration.
- Seven places read a record through the view of another class (finding 61, and below).

Findings 56 to 58 came from looking at the variables no class fits. A variable whose
fields fit a class but for one is either two records under one name or a wrong field
name.

What the views alone did not protect against: a reader that takes a field's value took
a JSON value, as a whole record is. One call that passed the record where its UUID was
expected compiled, and a test caught it. `FieldValue` closed that: the same call, put
back as a check, is refused by the compiler ("expected `FieldValue`, found `&Value`").
A field of the wrong class or the wrong meaning passed to a reader still compiles;
typed values, step 2 below, narrow that further.

A variable given the wrong class compiles too when that class has the fields read, and
this record first said that the assignment from fields could not make that mistake
where one class fits. It can, and did in seven places. Three helpers that ask whether a
record offers an operation read `allowed_operations` and `blocked_operations`; only a VM
has both, so they were given the VM's view, but they were handed repositories and
disks, a CD drive, and a disk among VMs. Three more places read a field that several
classes have (`uuid`, `other_config`, `network`, `snapshot_of`) from a record whose
class is another, or is only known at run time. All seven read what they meant to,
because the field has the same name in both classes, and the read of
`blocked_operations` from a class without it was a guard that cannot fire. Each now
reads through its record's own class, or by key where the class is not known until run
time.

Two checks made since look for this, neither of them complete:

- Every view was compared with the call that produced its record (`fresh(session,
  "VM", ..)`, a `Class.get_record`, a store key, or what a helper's callers pass). Of
  1,329 reads through views, 946 could be traced that way and agree, 16 were flagged
  by a class name that stood nearby as a field key and were read by hand, and the rest
  could not be traced: records handed on through several functions or kept in the
  client's own structures. The comparison was a script and is not in the repository.
  Run again after the move of 2026-11-12 described below, it counted 1,440 reads
  through views: 1,062 traced and agreeing, 14 flagged in four functions and read by
  hand (each is given the class its view names; the script had followed a shadowed
  name or a parameter called after another class), and 364 it could not trace.
- The check in debug builds, described above, was switched on for the core's whole test
  suite and for the lab scenarios run since. Over the tests it showed three of the seven
  places, which the comparison could not trace, and fixtures that gave one class's
  record another's fields. What it still reports there is meant: fixtures' own
  switches, two records given a foreign field to show it is not passed on, repository
  fixtures that carry an empty `blocked_operations`, and one key the client adds to a
  record it holds. It sees only the reads a test or a scenario exercises. How many
  that is has been measured since 0.19.0, the check now listing each place that makes
  a view: the core writes 1,509 views on 1,472 lines, and the workspace's tests make
  a view at 1,408 of those lines. The 64 lines no test reaches are where a record of
  the wrong class would still go unseen by the tests; 13 are in the VM lifecycle
  backend and the rest are spread over twenty files.

## What follows, in order

The count used here is of indexes by a string literal on an expression (`record["key"]`)
in the core's sources outside its test files, which is how a field is read without a
view. Before the move the core had 2,094 of them over 252 distinct keys; on 2026-11-11
it had 646 over 185, and on 2026-11-12 649, three reads of a record whose class is known
only at run time having gone back to keys. Not all of them are fields: some are keys
inside map-valued fields, parts of the JSON-RPC envelope, disk image and archive
metadata or the client's own structures.

After 0.19.0, 111 more moved onto views, leaving 538 over 182. A read moved only where
the class of its record is written down next to it: the class named in the call that
binds the record, a function that reads one class throughout, or a parameter whose
every caller could be traced and passes the same class. Two looser attempts, which also
took the class from the fields read, were undone: the check in debug builds showed
helpers given records of other classes under them. With the stricter rule the check
over the core's tests reported one thing, a test fixture that gave a VIF a VBD's
`bootable`, which was the fixture's mistake.

A further 66 were then moved one by one, each after reading where its record comes from:
a record fetched as one class a few lines above, the members of a list fetched as one
class, the arms of a match that each handle one class, and helpers whose every caller
was read. That leaves 472 over 176. The helpers that ask any record whether it is idle
or read a flag from any record's `other_config` stay by key, as they are given several
classes.

1. The reads by key that are left. Most are where the class of a record depends on a
   value at run time (network and storage operations read "the target's record", whose
   class follows the kind of target), where the fields read are too few to tell the
   class (a `uuid` alone), or in helpers that take a record of any class; each needs
   its class from the call that produced the record, or a helper per class. The loops
   that handle several classes at once stay by key. Of the 472, the four inventory
   decoders hold 99 and some 116 are not fields at all; about 240 are reads whose class
   could not be traced by a script and have not been read by hand, and some 30 are in
   code given several classes.
2. Typed values: generated readers that turn a field into its Rust type, with the core's
   bounds applied to the result. As far as a view reaches, this is done.

   A view's method returns the type of its field's kind, and a field compares only with
   what its kind allows: a flag with a boolean, text, a reference and an enumeration's
   value with a string, a number with a number or its digits, and two fields with each
   other only when they are of one kind. The core's readers take the kind they read (a
   UUID reader takes text, a link reader a reference, a size reader a whole number, an
   operation list a set), so a field of another kind handed to one does not compile; 255
   calls were retyped that way and none of them turned out to pass the wrong kind.

   Since 2026-11-13 a typed field has the readers of its kind and nothing else (`get`
   for a flag; `text`; `reference`; `name`; `number` and `unsigned`; `members` and
   `strings` for a set; `entries`, and `get` and `text` by key, for a map). It no longer
   reads as the JSON value it wraps, so `as_bool` on a text field, which used to compile
   and give nothing, does not compile. The compiler made the move: with the JSON
   accessors gone it named every read, on a view held in a variable and in the tests as
   much as those written on a view, which were all the earlier scan could see. The core
   has 461 fewer JSON accessor calls for it, and no read turned out to have asked for a
   kind its field is not. Examples in the bindings that must not compile hold this, each
   beside one that differs only in the reader and does.

   The helpers that the workflows share were retyped the same way: 50 parameters in 27
   files say which kind they read (`impl Reads<'a, kind::Reference>`, and likewise a
   whole number, a set, a map or text), and 196 calls hand them a field. Such a helper
   takes a field of its kind, or a JSON value that did not come through a view (a read
   by key, a value taken out of a map), whose kind the caller vouches for by passing it.
   A field of another kind goes in only as its JSON value, asked for by name with
   `value()`. Seven places do that. Five are text that holds a number: the position of
   a disk or of an interface on its guest is a string of digits, handed to the reader of
   whole numbers in the OVF export and import. Two are inside the core's own strict
   readers, which take a typed field and tell a missing value from a malformed one. A
   test lists the five and fails on a sixth written on a view.

   Enumeration values are the generated constants wherever one enumeration is meant.
   276 more literals moved, which were values compared with the client's own (the kind
   of an attachment or a disk held in the inventory, a reviewed address mode or power
   state), the names the client's types carry for what they send (`Ipv4Mode::as_str`,
   the bond modes, the schedule of a snapshot policy), operation names asked of a
   record's `allowed_operations`, and values written into the records the client
   creates. Each was given its enumeration by reading where the value comes from, since
   `"None"`, `"destroy"` or `"plug"` belong to several. The core now writes 648
   constants of 33 enumerations. Sixty-six literals that only look like values were
   left: keys, labels, an SR's content type and the HA restart priority (both strings
   in the API), storage types, and words of the OVF, update and metrics formats.

   What is not done, and why. Fourteen names of power actions stay literals: each is the
   name of the call first (`VM.clean_shutdown`) and of the operation second, and a test
   holds each to being both. Three literals stand for a value of two enumerations at
   once (`"None"` for a VIF's IPv4 and IPv6 modes read in one loop, and the dynamic
   address modes of either family). A value taken out of a map-valued field has no kind
   of its own; `MapField::text` reads the common case and anything else is given a kind
   by hand. The core's bounds are applied by its own readers, not by generated ones: what
   a malformed value means differs from one workflow to the next. A read by key is
   untyped until it moves to a view (item 1). And a constant is a string like the
   literal it replaced, so nothing but review keeps a new literal from being written.
3. Calls: generated descriptions of each message's parameters, so that a call's
   arguments are checked where it is made. A test checks their number for the calls
   written with their arguments. Since 0.19.0 a debug build can also compare every call
   it sends with the API, where the call leaves the client: the number of arguments,
   and each one against the type the API gives its place (a boolean, a whole number, a
   string, a value of the named enumeration, a list or a map of those). It reports to
   the file the view check writes, names only, and the repository's check fails on any
   report. The catalogue gained, for this, how many of each call's parameters are
   required: the description marks 77 as optional, all of them last, and the client
   leaves some out (`host.disable`, `host.evacuate`, `VM.snapshot`, `VM.migrate_send`).
   Over the workspace's tests the check saw 280 distinct calls and reported none. Of the
   223 calls the core's sources name, the tests send 217; five of the rest are network
   and power operations that only the lab exercises, and one is named for a permission
   table alone. What it cannot see: a reference of the wrong class, which is a string
   like any other, two arguments of one type in each other's places, and any call that
   no test or scenario makes. Checking those needs the arguments typed where they are
   written, which is not done.
4. The experimental areas, class by class, each with the fixture tests that cover it.

A workflow whose decoding moves onto the generated crate is run again on the lab before
it is called qualified on it: the live evidence gathered before is not carried over by
assumption.
