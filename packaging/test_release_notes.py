import pathlib
import unittest

import release_notes

CHANGELOG = """# Changelog

## 1.2.0 (in development)

### Fixed

- Something not yet released.

## 1.1.0 (2026-01-02)

Stored formats: journal 4.

- One change.

## 1.0.0

## Stored state between versions

- A rule.
"""


class ReleaseNotes(unittest.TestCase):
    def test_a_released_entry_is_returned_up_to_the_next_heading(self):
        self.assertEqual(
            release_notes.entry(CHANGELOG, "1.1.0"),
            "Stored formats: journal 4.\n\n- One change.\n",
        )

    def test_an_entry_in_development_is_refused(self):
        with self.assertRaisesRegex(release_notes.NotesError, "in development"):
            release_notes.entry(CHANGELOG, "1.2.0")

    def test_a_missing_or_empty_entry_is_refused(self):
        with self.assertRaisesRegex(release_notes.NotesError, "no entry"):
            release_notes.entry(CHANGELOG, "9.9.9")
        with self.assertRaisesRegex(release_notes.NotesError, "empty"):
            release_notes.entry(CHANGELOG, "1.0.0")

    def test_repository_links_are_written_against_the_tagged_tree(self):
        notes = (
            "See the [scope](docs/FEATURES.md) and [findings](docs/reports/a.md#findings).\n"
            "Not [this](https://example.test/x), [this](#known-issues) or [this](/rooted).\n"
        )
        base = "https://example.test/owner/repo/blob/v1.1.0/"
        self.assertEqual(
            release_notes.absolute_links(notes, base),
            "See the [scope](https://example.test/owner/repo/blob/v1.1.0/docs/FEATURES.md) and "
            "[findings](https://example.test/owner/repo/blob/v1.1.0/docs/reports/a.md#findings).\n"
            "Not [this](https://example.test/x), [this](#known-issues) or [this](/rooted).\n",
        )

    def test_a_version_is_matched_whole(self):
        with self.assertRaisesRegex(release_notes.NotesError, "no entry"):
            release_notes.entry(CHANGELOG, "1.1")

    def test_the_repository_changelog_has_an_entry_for_the_workspace_version(self):
        import tomllib

        root = pathlib.Path(__file__).resolve().parent.parent
        version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        changelog = (root / "CHANGELOG.md").read_text(encoding="utf-8")
        try:
            release_notes.entry(changelog, version)
        except release_notes.NotesError as error:
            # Development versions carry an entry that is not yet releasable.
            self.assertIn("in development", str(error))


if __name__ == "__main__":
    unittest.main()
