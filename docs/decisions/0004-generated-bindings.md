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
  test. Where a call was written with its arguments beside it, which 362 were, it also
  checked that the call passed as many as the API gives the message; all did. Those
  calls are now typed messages, which the compiler checks (item 3 below), and the test
  holds instead that only the session layer still writes a call as a name and a list.

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

On 2026-11-13 every read by key that was left was read where it stands, and 202 more
moved: 77 in the network operations, 53 in host and pool operations and the inventory
decoder, 36 in storage and transfers, 35 in device and VM operations and one of the
session's own record. The rule was the one above, applied by reading: a record bound by
a call that names its class, a branch that handles one class, a parameter whose every
caller passes one class, and an object of an archive only behind the check of its
class. For "the target's record" in the network, storage and resource operations the
class is the one the operation's own validation fixes for that kind of operation, which
is in another file than the read; those are the moves that rest on more than the lines
around them. The count went from 477 (the 472 above and five written since) to 289 over
132 keys. The check over the workspace's tests, now of views made at 1,592 places,
reported one thing: a fixture's own switch on a pool record that was not named as
fixtures' switches are.

What the reading found, besides the moves:

- The inspection of an XVA archive refused a host's default template by a key of
  `other_config` and by a second key there, `is_default_template`, which is not a key
  hosts write: it is a field of the record. The field is now what is read, with the
  older key beside it.
- The pass that wrote enumeration values as constants had turned one record key into a
  constant, because a PIF's `IPv6` field has the name of an address type's value. The
  string was the same and nothing changed; it is a field read through the view now.
- An archive's object was viewed as a disk a few lines before its class was checked.
  The outcome was the same refusal; the check now comes first.
- The helper that asks whether a record offers an operation is given a repository by
  one caller and reads `blocked_operations`, which a repository does not have (already
  noted where it is written). It stays by key.

1. The reads by key that are left: 289, of which some 77 are in test modules inside the
   source files. None is left because its class was not looked for. About 140 are not
   fields: keys inside map-valued fields (`other_config`, a PBD's `device_config`,
   `restrictions`), the archive's own keys beside a record, RRD documents, `qemu-img`
   output, the JSON-RPC envelope and the client's own JSON. Some 60 are in code that is
   given records of several classes: the helpers that ask any record whether it is idle
   or what its `uuid` is, the loops over several classes in the inventory decoder, and
   operations whose target may be one of several classes (renaming a pool, a host, a
   repository or a network). A dozen are writes. Reads whose key is a variable
   (`record[field]`) are not in this count and stay as they are. Moving the 60 needs a
   helper per class or a class carried with the record, which is a change of design and
   not of spelling.
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

   The supported part was run again on the lab on this code the same day: 21 of the
   core's 22 scenarios, host maintenance and moves within a pool, all passing, with the
   coordinator's power cut left out
   ([live report](reports/2026-11-09-lab-qualification.md)).
3. Calls: generated descriptions of each message's parameters, so that a call's
   arguments are checked where it is made. For the calls written with their arguments
   this is done.

   First came checks at run time. A test checked the number of arguments of the calls
   written with them. Since 0.19.0 a debug build compares every call it sends with the
   API, where the call leaves the client: the number of arguments, and each one against
   the type the API gives its place (a boolean, a whole number, a string, a value of the
   named enumeration, a list or a map of those). It reports to the file the view check
   writes, names only, and the repository's check fails on any report. The catalogue
   gained, for this, how many of each call's parameters are required: the description
   marks 77 as optional, all of them last, and the client leaves some out
   (`host.disable`, `host.evacuate`, `VM.snapshot`, `VM.migrate_send`). Over the
   workspace's tests the check sees 280 distinct calls and reports none. It cannot see a
   reference of the wrong class, which is a string like any other, two arguments of one
   type in each other's places, or any call that no test or scenario makes.

   Since 2026-11-13 the bindings have a type for every message that takes the session,
   1,941 of them. Its fields are the call's parameters under the API's names, each in
   the type of its kind: a flag is a `bool`, text and an enumeration's value a string, a
   whole number an `Int` (sent as digits, as the client always sent it), a reference a
   `Ref` of the class the API names. A set, a map and a record are JSON values handed
   over whole. A message with one parameter is a tuple, and where the last parameters
   may be left out there is a type for each length (`host::Evacuate`,
   `host::EvacuateWithNetwork`), 2,016 types in all. `Session::send` takes one; a call
   that is prepared first and sent later is made from one as its name and arguments.

   The core writes 363 calls this way, of 121 different messages: 298 sent where they
   are written and 65 prepared. A script rewrote them from the catalogue and the
   compiler then went over each. What it checks there, in every build and whether or not
   a test reaches the call: how many arguments; which is which, since a field is given
   by name and two arguments of one type cannot change places unseen (23 of the
   messages the core calls take two or more of one type, such as the four limits of
   `VM.set_memory_limits`); the kind of each; and for a reference, its class.

   The rewrite found nothing sent wrongly. Five places gave `task.destroy` or
   `task.get_record` a reference that might be absent, which would have gone as null;
   each now says what it does when there is none. The tests are answered the same 280
   distinct calls as before, and the debug check reports none.

   What is not done. A reference is text, and its class is the caller's word, written
   where the reference is used (`refs::vm(&reference)`). The compiler holds that word
   against the parameter it is given to, not against where the reference came from: a
   reference would have to carry its class from its origin, a view's field of type
   `host ref` or the answer to `VM.get_by_uuid`, and neither views nor replies give one
   yet. An enumeration's value is text, and a set, a map and a record are JSON; the
   debug check still judges those when a test or a scenario sends them. And 43 calls are
   still a name and a list: 38 whose name is put together at run time for one of several
   classes (`{class}.get_record`) and five whose arguments are built elsewhere. The four
   that the session layer sends itself are listed by a test, which fails on another.
   Names written alone, for permissions and for the journal, are checked against the
   catalogue as before.
4. The experimental areas, class by class, each with the fixture tests that cover it.

A workflow whose decoding moves onto the generated crate is run again on the lab before
it is called qualified on it: the live evidence gathered before is not carried over by
assumption.
