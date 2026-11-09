import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import appimage
from appimage import fetch
import urllib.error
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

    def test_transient_download_errors_are_retried_and_client_errors_are_not(self):
        data = b'reviewed bytes'
        desc = {'url': 'https://example.invalid/tool', 'sha256': __import__('hashlib').sha256(data).hexdigest()}

        class Response:
            def __init__(self, body):
                self.body = body

            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def read(self, limit):
                return self.body[:limit]

        def opener(answers):
            calls = []

            def urlopen(request, timeout):
                calls.append(request.full_url)
                answer = answers.pop(0)
                if isinstance(answer, Exception):
                    raise answer
                return Response(answer)
            return urlopen, calls

        def gateway(code):
            return urllib.error.HTTPError(desc['url'], code, 'gateway', {}, None)

        slept = []
        original = (appimage.urllib.request.urlopen, appimage._sleep)
        appimage._sleep = slept.append
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                appimage.urllib.request.urlopen, calls = opener([gateway(504), urllib.error.URLError('reset'), data])
                self.assertEqual(fetch(root, 'tool', desc).read_bytes(), data)
                self.assertEqual(len(calls), 3)
                self.assertEqual(slept, list(appimage.RETRY_DELAYS[:2]))
                appimage.urllib.request.urlopen, calls = opener([gateway(404), data])
                with self.assertRaises(ValueError):
                    fetch(root, 'other', desc)
                self.assertEqual(len(calls), 1)
                appimage.urllib.request.urlopen, calls = opener([gateway(502)] * (len(appimage.RETRY_DELAYS) + 1))
                with self.assertRaises(ValueError):
                    fetch(root, 'other', desc)
                self.assertEqual(len(calls), len(appimage.RETRY_DELAYS) + 1)
                mirrored = dict(desc, mirrors=['https://example.invalid/mirror'])
                appimage.urllib.request.urlopen, calls = opener([gateway(504)] * (len(appimage.RETRY_DELAYS) + 1) + [data])
                self.assertEqual(fetch(root, 'mirrored', mirrored).read_bytes(), data)
                self.assertEqual(calls[-1], 'https://example.invalid/mirror')
                self.assertEqual(len(calls), len(appimage.RETRY_DELAYS) + 2)
                appimage.urllib.request.urlopen, calls = opener([gateway(404), b'tampered bytes'])
                with self.assertRaises(ValueError):
                    fetch(root, 'other', mirrored)
                self.assertEqual(len(calls), 2)
                appimage.urllib.request.urlopen, calls = opener([b'tampered bytes', data])
                with self.assertRaises(ValueError):
                    fetch(root, 'other', desc)
                self.assertEqual(len(calls), 1)
                self.assertFalse((root / 'other').exists())
        finally:
            appimage.urllib.request.urlopen, appimage._sleep = original

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
