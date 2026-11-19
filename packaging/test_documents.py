from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parent.parent
STAGED = re.compile(r'^install -m\d+ (\S+) "\$stage_dir/\$package_name/(\S*)"$', re.MULTILINE)
LINK = re.compile(r'\]\(([^)\s]+)\)')


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


if __name__ == '__main__':
    unittest.main()
