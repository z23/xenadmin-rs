import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from appimage import fetch
from install import digest


class AppImagePackaging(unittest.TestCase):
    def test_cached_tools_are_verified_before_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / 'tool'
            path.write_text('reviewed bytes')
            desc = {'url': 'https://example.invalid/not-requested', 'sha256': digest(path)}
            self.assertEqual(fetch(root, 'tool', desc), path)
            path.write_text('changed bytes')
            with self.assertRaises(ValueError):
                fetch(root, 'tool', desc)
            path.unlink()
            (root / 'other').write_text('reviewed bytes')
            path.symlink_to(root / 'other')
            with self.assertRaises(ValueError):
                fetch(root, 'tool', desc)

    def test_apprun_preserves_arguments_and_routes_probe_and_installer(self):
        with tempfile.TemporaryDirectory(prefix='image space ') as tmp:
            root = Path(tmp)
            shutil.copyfile(Path(__file__).parent / 'AppRun', root / 'AppRun')
            (root / 'AppRun').chmod(0o755)
            (root / 'usr/bin').mkdir(parents=True)
            script = '#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv))\n'
            for name in ('usr/bin/xenadmin-rs','usr/bin/host-probe','usr/install.py'):
                path = root / name
                path.write_text(script)
                path.chmod(0o755)
            for route, binary in (([], 'xenadmin-rs'), (['--probe'], 'host-probe'), (['--install'], 'install.py')):
                args = ['a space', '$(no-command)', '--flag', '']
                output = json.loads(subprocess.check_output([root / 'AppRun', *route, *args], text=True))
                self.assertEqual(Path(output[0]).name, binary)
                self.assertEqual(output[1:], args)
