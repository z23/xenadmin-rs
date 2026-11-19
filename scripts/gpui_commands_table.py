#!/usr/bin/env python3
"""Print the registered commands as rows of the migration ledger's Commands table.

usage: gpui_commands_table.py [artifacts/ui/commands.txt]

The input is what `xenadmin-scenes --commands` prints (scripts/check.sh writes it to
artifacts/ui/commands.txt): one line per command, eight fields separated by a tab: id,
label, menu path, toolbar slot, context kinds, local row, shortcut, area. The output is a
Markdown table with the ledger's columns; the columns the registry cannot know (Rune
source, GPUI status, package) say where the command is registered instead.
"""

import sys

FIELDS = 8


def rows(text):
    for number, line in enumerate(text.splitlines(), 1):
        if not line:
            continue
        fields = line.split("\t")
        if len(fields) != FIELDS:
            raise SystemExit(f"line {number}: {len(fields)} fields, expected {FIELDS}")
        yield fields


def cell(text):
    return text.replace("|", "\\|")


def table(text):
    out = [
        "| CommandId | label | menu path | object kinds | toolbar | local row | shortcut | area |",
        "| --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for identifier, label, menu, toolbar, context, local, shortcut, area in rows(text):
        cells = [f"`{identifier}`", label, menu, context, toolbar, local, shortcut, f"`{area}`"]
        out.append("| " + " | ".join(cell(value) for value in cells) + " |")
    return "\n".join(out) + "\n"


def main(arguments):
    if len(arguments) > 1 or (arguments and arguments[0] in ("-h", "--help")):
        raise SystemExit(__doc__)
    path = arguments[0] if arguments else "artifacts/gpui/commands.txt"
    with open(path, encoding="utf-8") as source:
        sys.stdout.write(table(source.read()))


if __name__ == "__main__":
    main(sys.argv[1:])
