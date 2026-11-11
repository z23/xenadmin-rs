#!/usr/bin/env python3
"""Compares the fields of a lab pool's live records with the API description the client's
bindings are generated from.

    description_check.py HOST SHA256_FINGERPRINT [DESCRIPTION_JSON]

HOST is the pool coordinator. For every class of the description that offers
`get_all_records`, the script reads the pool's records of that class and compares the keys
they carry with the fields the description lists. It prints, as JSON, the fields live
records carry that the description lacks, the fields the description lists that no live
record carries, the classes that had no records to compare, and the classes the host
refused. A field of a class without records cannot be compared.

Read-only: the only calls are a login, `get_all_records` for each class and a logout. The
password is read from the file named by XENADMIN_LAB_PASSWORD_FILE and is never taken from
the command line or printed, and nothing is sent to a host whose certificate does not
match the fingerprint. Record contents are not printed, only field names.

DESCRIPTION_JSON defaults to the description in `crates/xenadmin-xapi/data`. The script
reads it where it is; it holds no copy of it.
"""
import json
import os
import sys

import other_admin

USAGE = "usage: description_check.py HOST SHA256_FINGERPRINT [DESCRIPTION_JSON]"
DESCRIPTION = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates",
                           "xenadmin-xapi", "data", "xenapi.json")


def compare(description, ask):
    """The differences between the description's fields and the live records' keys."""
    outcome = {"compared": [], "without_records": [], "refused": {},
               "live_fields_the_description_lacks": {},
               "described_fields_no_record_carries": {}}
    for cls in sorted(description, key=lambda c: c["name"]):
        name = cls["name"]
        if not any(message["name"] == "get_all_records" for message in cls["messages"]):
            continue
        try:
            records = ask(f"{name}.get_all_records")
        except other_admin.Refused as refusal:
            outcome["refused"][name] = str(refusal)
            continue
        if not isinstance(records, dict) or not records:
            outcome["without_records"].append(name)
            continue
        described = {field["name"] for field in cls["fields"]}
        live = set()
        for record in records.values():
            live.update(record)
        outcome["compared"].append(name)
        if live - described:
            outcome["live_fields_the_description_lacks"][name] = sorted(live - described)
        if described - live:
            outcome["described_fields_no_record_carries"][name] = sorted(described - live)
    return outcome


def main(arguments):
    if len(arguments) not in (2, 3):
        print(USAGE, file=sys.stderr)
        return 2
    host, fingerprint = arguments[:2]
    path = os.environ.get("XENADMIN_LAB_PASSWORD_FILE")
    if not path:
        print("XENADMIN_LAB_PASSWORD_FILE must name the lab-only root password file", file=sys.stderr)
        return 2
    with open(path, encoding="utf-8") as file:
        password = file.readline().strip()
    try:
        with open(arguments[2] if len(arguments) == 3 else DESCRIPTION, encoding="utf-8") as file:
            description = json.load(file)
        outcome = other_admin.act(host, fingerprint, password,
                                  lambda ask: compare(description, ask), [])
    except (other_admin.Refused, ValueError, OSError) as error:
        print(f"description_check: {error}", file=sys.stderr)
        return 1
    print(json.dumps(outcome, indent=1, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
