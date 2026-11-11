#!/usr/bin/env python3
"""Print one version's changelog entry, for a release's notes.

Usage: release_notes.py VERSION [CHANGELOG]

Fails when the changelog has no entry for the version, when the entry is still marked as
in development, or when it is empty, so a tag cannot be released without notes.
"""
import pathlib
import re
import sys


class NotesError(Exception):
    pass


def entry(changelog: str, version: str) -> str:
    """The body under the `## VERSION ...` heading, up to the next second-level heading."""
    heading = re.compile(r"^## " + re.escape(version) + r"(?: \(([^)]*)\))?\s*$", re.MULTILINE)
    found = heading.search(changelog)
    if found is None:
        raise NotesError(f"the changelog has no entry for {version}")
    if "in development" in (found.group(1) or ""):
        raise NotesError(f"the changelog entry for {version} is still marked in development")
    following = re.compile(r"^## ", re.MULTILINE).search(changelog, found.end())
    body = changelog[found.end() : following.start() if following else len(changelog)].strip()
    if not body:
        raise NotesError(f"the changelog entry for {version} is empty")
    return body + "\n"


def main(arguments: list[str]) -> int:
    if len(arguments) not in (1, 2):
        print(__doc__, file=sys.stderr)
        return 2
    path = pathlib.Path(arguments[1] if len(arguments) == 2 else "CHANGELOG.md")
    try:
        sys.stdout.write(entry(path.read_text(encoding="utf-8"), arguments[0]))
    except (NotesError, OSError) as error:
        print(f"release notes: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
