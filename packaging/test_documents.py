from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parent.parent
STAGED = re.compile(r'^install -m\d+ (\S+) "\$stage_dir/\$package_name/(\S*)"$', re.MULTILINE)
LINK = re.compile(r'\]\(([^)\s]+)\)')
REPOSITORY = 'https://github.com/z23/xenadmin-rs/'
PROJECT = REPOSITORY + 'blob/main/'


def shipped():
    """What scripts/package.sh installs into the package: source path to path in the package."""
    files = {}
    for source, target in STAGED.findall((ROOT / 'scripts/package.sh').read_text()):
        files[source] = target + Path(source).name if target.endswith('/') or not target else target
    return files


class ShippedDocuments(unittest.TestCase):
    def test_the_guides_are_shipped(self):
        files = shipped()
        self.assertEqual(files.get('docs/RUNNING.md'), 'README.md')
        self.assertEqual(files.get('docs/INSTALLATION.md'), 'INSTALLATION.md')
        self.assertEqual(files.get('docs/CONNECTIONS.md'), 'CONNECTIONS.md')

    def test_every_relative_link_of_a_shipped_document_resolves_in_the_package(self):
        # A document in the package is read without the repository around it: a link to a
        # report or a decision must be absolute, and a link to another shipped document
        # must name a file that is beside it under the name it is shipped with.
        files = shipped()
        present = set(files.values())
        broken = []
        for source, target in files.items():
            if not source.endswith('.md'):
                continue
            for link in LINK.findall((ROOT / source).read_text()):
                if re.match(r'[a-z][a-z0-9+.-]*:', link) or link.startswith('#'):
                    continue
                path = link.split('#', 1)[0]
                resolved = (Path(target).parent / path).as_posix()
                # Links are written for the repository: they must hold there as well.
                in_repository = (ROOT / source).parent / path
                if resolved not in present or not in_repository.exists():
                    broken.append(f'{source}: {link}')
        self.assertEqual(broken, [])

    def test_every_link_of_a_shipped_document_to_the_project_names_a_file_of_this_tree(self):
        # The absolute links point at the main branch of the project. They resolve for a
        # reader only once that branch holds this tree; here they must at least name a
        # file this tree has, so that they resolve then.
        missing = []
        seen = 0
        for source in shipped():
            if not source.endswith('.md'):
                continue
            for link in LINK.findall((ROOT / source).read_text()):
                if not link.startswith(PROJECT):
                    continue
                seen += 1
                if not (ROOT / link[len(PROJECT):].split('#', 1)[0]).is_file():
                    missing.append(f'{source}: {link}')
        self.assertEqual(missing, [])
        self.assertGreater(seen, 0, 'the shipped guides link to the reports')

    def test_a_link_to_the_project_is_one_to_its_main_branch(self):
        # Any other link into the project's repository (another branch, a commit, the tree
        # view) is not checked by the test above and would go unnoticed.
        other = []
        for source in shipped():
            if not source.endswith('.md'):
                continue
            for link in LINK.findall((ROOT / source).read_text()):
                if link.startswith(REPOSITORY) and not link.startswith(PROJECT):
                    rest = link[len(REPOSITORY):]
                    if rest.startswith(('blob/', 'tree/', 'raw/')):
                        other.append(f'{source}: {link}')
        self.assertEqual(other, [])


class PackagingScript(unittest.TestCase):
    def test_an_appimage_that_fails_its_run_takes_the_published_files_back(self):
        # The AppImage is written into artifacts/ before it can be run. The script must not
        # leave it, its checksum or the archive of the same run there when the run fails.
        script = (ROOT / 'scripts/package.sh').read_text()
        guarded = script.index('if ! python3 packaging/smoke.py "artifacts/$package_name.AppImage" --appimage; then')
        taken_back = script[guarded:script.index('\n    fi\n', guarded)]
        for name in ('.AppImage"', '.AppImage.sha256"', '.tar.gz"', '.tar.gz.sha256"'):
            self.assertIn('"artifacts/$package_name' + name, taken_back)
        self.assertIn('exit 1', taken_back)
        self.assertNotIn('\npython3 packaging/smoke.py "artifacts/', script)
        self.assertNotIn('\n    python3 packaging/smoke.py "artifacts/', script)


if __name__ == '__main__':
    unittest.main()
