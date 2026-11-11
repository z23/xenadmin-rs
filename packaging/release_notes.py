#!/usr/bin/env python3
"""Print one version's changelog entry, for a release's notes.

Usage: release_notes.py VERSION [CHANGELOG] [--link-base URL]

Fails when the changelog has no entry for the version, when the entry is still marked as
in development, or when it is empty, so a tag cannot be released without notes. With
--link-base, links that are relative to the repository are written against that URL, so
they still lead somewhere on a release page.
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


def absolute_links(notes: str, base: str) -> str:
    """Rewrites Markdown links to files in the repository against `base`. Links with a
    scheme, to an anchor or to a rooted path are left as they are."""
    link = re.compile(r"\]\((?![a-z][a-z0-9+.-]*:|#|/)([^)\s]+)\)")
    return link.sub(lambda found: f"]({base.rstrip('/')}/{found.group(1)})", notes)


def main(arguments: list[str]) -> int:
    base = None
    if "--link-base" in arguments:
        at = arguments.index("--link-base")
        if at + 1 >= len(arguments):
            print(__doc__, file=sys.stderr)
            return 2
        base = arguments[at + 1]
        arguments = arguments[:at] + arguments[at + 2 :]
    if len(arguments) not in (1, 2):
        print(__doc__, file=sys.stderr)
        return 2
    path = pathlib.Path(arguments[1] if len(arguments) == 2 else "CHANGELOG.md")
    try:
        notes = entry(path.read_text(encoding="utf-8"), arguments[0])
        sys.stdout.write(absolute_links(notes, base) if base else notes)
    except (NotesError, OSError) as error:
        print(f"release notes: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
