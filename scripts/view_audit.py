#!/usr/bin/env python3
"""Checks what the record views and the calls reported while the tests ran.

    view_audit.py FILE

FILE is what debug builds write when `XENADMIN_XAPI_VIEW_AUDIT` names it: for every view
made of a record that carries a key its class lacks, a line with the class, the key and the
place the view was made, separated by tabs. A record read through the view of another class
is seen that way, and so is a fixture with a field the API does not have.

The tests give a few records such keys on purpose; those are listed here. Anything else
fails the check and is printed. An absent or empty file passes: nothing was reported.

The file also holds a line for every call that was sent with an argument the API's type
for that parameter does not admit, or with a number of arguments the API does not take:
`call`, the call's name, the parameter's position or `count`, and what was wrong. No call
is expected to be reported, so each of those fails the check. Lines that start with `sent`
name the calls that were made at all, and lines that start with `viewed` the places that
made a view; both are counted and never fail anything.
"""
import sys

# (class, key): why a test's record of that class carries that key.
EXPECTED = {
    ("SM", "device_config"): "a storage backend given a secret to show it is not passed on",
    ("host_crashdump", "filename"): "a crash dump given a path to show it is not passed on",
    ("VBD", "ref"): "the reference the client adds to a CD drive's record it holds",
}
# The bindings' own test reads a repository's record through a VM's view to show this file.
EXPECTED_PLACES = ("crates/xenadmin-xapi/src/lib.rs",)


def unexpected(lines):
    """The reported lines that no test means, as (class, key, place)."""
    found = set()
    for line in lines:
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 3 or parts[0] in ("viewed", "call"):
            continue
        cls, key, place = parts
        if key.startswith("fixture_"):  # a fixture's own switches
            continue
        if (cls, key) in EXPECTED or place.rsplit(":", 1)[0].endswith(EXPECTED_PLACES):
            continue
        found.add((cls, key, place))
    return sorted(found)


def calls(lines):
    """The calls reported with arguments that do not fit, as (call, position, what)."""
    found = set()
    for line in lines:
        parts = line.rstrip("\n").split("\t")
        if len(parts) == 4 and parts[0] == "call":
            found.add(tuple(parts[1:]))
    return sorted(found)


def main(arguments):
    if len(arguments) != 1:
        print("usage: view_audit.py FILE", file=sys.stderr)
        return 2
    try:
        with open(arguments[0], encoding="utf-8") as file:
            lines = file.readlines()
    except FileNotFoundError:
        return 0
    found = unexpected(lines)
    for cls, key, place in found:
        print(f"view_audit: a {cls} view at {place} was given a record with the key {key!r}, "
              f"which {cls} does not have", file=sys.stderr)
    misfits = calls(lines)
    for call, position, what in misfits:
        where = "its arguments" if position == "count" else f"parameter {position}"
        print(f"view_audit: {call} was sent with {where} wrong: {what}", file=sys.stderr)
    sent = {line.rstrip("\n").split("\t")[1].removeprefix("Async.")
            for line in lines if line.startswith("sent\t") and line.count("\t") == 1}
    if sent:
        print(f"view_audit: {len(sent)} distinct calls were sent and compared with the API")
    viewed = {line.rstrip("\n").split("\t")[2]
              for line in lines if line.startswith("viewed\t") and line.count("\t") == 2}
    if viewed:
        print(f"view_audit: views were made at {len(viewed)} distinct places and compared "
              "with their classes")
    return 1 if found or misfits else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
