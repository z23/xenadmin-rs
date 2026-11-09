#!/usr/bin/env python3
"""Build an x86-64 AppImage from a verified portable package."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
from install import atomic, digest, manifest, regular, verify
from manifest import generate

ROOT = Path(__file__).resolve().parent
MAX_INPUT = 64 * 1024 * 1024


# Release mirrors answer with transient gateway errors now and then; a pinned input is
# retried a few times before the build is refused. Client errors and hash mismatches never are.
RETRY_DELAYS = (2, 5, 10)
_sleep = time.sleep


def transient(error):
    if isinstance(error, urllib.error.HTTPError):
        return error.code >= 500 or error.code == 429
    return isinstance(error, (urllib.error.URLError, TimeoutError))


def download(name, url):
    request = urllib.request.Request(url, headers={'User-Agent': 'xenadmin-rs-packaging'})
    for attempt, delay in enumerate((*RETRY_DELAYS, None)):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read(MAX_INPUT + 1)
        except (urllib.error.URLError, TimeoutError) as error:
            if delay is None or not transient(error):
                raise
            print(f'Packaging input {name} fetch failed (attempt {attempt + 1}): {error}; retrying in {delay}s', flush=True)
            _sleep(delay)
    raise AssertionError('unreachable')


def fetch(cache, name, descriptor):
    path = cache / name
    if path.exists() or path.is_symlink():
        regular(path, MAX_INPUT)
        if digest(path) != descriptor['sha256']:
            raise ValueError(f"Cached packaging input changed: {name}")
        return path
    # Every source must serve the pinned bytes; a mirror only changes who answers.
    sources = [descriptor['url'], *descriptor.get('mirrors', [])]
    data = None
    for url in sources:
        try:
            data = download(name, url)
            break
        except (urllib.error.URLError, TimeoutError) as error:
            print(f'Packaging input {name} unavailable from {url}: {error}', flush=True)
            failure = error
    if data is None:
        raise ValueError(f"Packaging input {name} unavailable from {len(sources)} source(s): {failure}")
    if len(data) > MAX_INPUT or hashlib.sha256(data).hexdigest() != descriptor['sha256']:
        raise ValueError(f"Packaging input hash or size mismatch: {name}")
    atomic(path, data)
    return path


def source_member(archive, suffix):
    with tarfile.open(archive) as tar:
        matches = [m for m in tar if m.name.endswith('/' + suffix)]
        if len(matches) != 1 or not matches[0].isfile() or matches[0].size > 1024 * 1024:
            raise ValueError(f"Missing or ambiguous runtime notice: {suffix}")
        with tar.extractfile(matches[0]) as stream:
            return stream.read()


def build(source, destination, cache):
    data = manifest(source)
    verify(source, data)
    if data['arch'] != 'x86_64' or os.uname().machine != 'x86_64':
        raise ValueError('AppImage currently has reviewed x86-64 inputs only')
    if os.path.lexists(destination) or os.path.lexists(str(destination) + '.sha256'):
        raise ValueError('Refusing to replace an existing versioned AppImage')
    destination.parent.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    inputs = json.loads((ROOT / 'appimage-inputs.json').read_text())
    paths = {name: fetch(cache, name, value) for name, value in inputs.items()}
    for name in ('appimagetool-x86_64.AppImage', 'runtime-x86_64'):
        paths[name].chmod(0o755)
    with tempfile.TemporaryDirectory(prefix='.appimage-', dir=destination.parent) as temporary:
        work = Path(temporary)
        app = work / 'XenAdmin.AppDir'
        shutil.copytree(source, app / 'usr')
        notices = app / 'usr/third-party/appimage-runtime'
        notices.mkdir(parents=True)
        for name, path in paths.items():
            if name not in ('appimagetool-x86_64.AppImage', 'runtime-x86_64'):
                shutil.copyfile(path, notices / name)
        (notices / 'LICENSE-runtime').write_bytes(source_member(paths['runtime-source.tar.gz'], 'LICENSE'))
        (notices / 'COPYING-libfuse').write_bytes(source_member(paths['fuse-3.15.0.tar.xz'], 'LGPL2.txt'))
        (notices / 'LICENSE-squashfuse').write_bytes(source_member(paths['squashfuse-0.5.2.tar.gz'], 'LICENSE'))
        shutil.copyfile(ROOT / 'appimage-inputs.json', notices / 'inputs.json')
        shutil.copyfile(ROOT / 'runtime-sources.md', notices / 'README.md')
        generate(app / 'usr', data['version'], data['arch'])
        shutil.copyfile(ROOT / 'AppRun', app / 'AppRun')
        (app / 'AppRun').chmod(0o755)
        shutil.copyfile(ROOT / 'xenadmin-rs.desktop', app / 'xenadmin-rs.desktop')
        shutil.copyfile(ROOT / 'xenadmin-rs.svg', app / 'xenadmin-rs.svg')
        shutil.copyfile(ROOT / 'xenadmin-rs.svg', app / '.DirIcon')
        subprocess.run(['desktop-file-validate', str(app / 'xenadmin-rs.desktop')], check=True)
        output = work / 'result.AppImage'
        env = os.environ.copy()
        env['ARCH'] = data['arch']
        env['VERSION'] = data['version']
        subprocess.run([str(paths['appimagetool-x86_64.AppImage']), '--appimage-extract-and-run',
                        '--no-appstream', '--runtime-file', str(paths['runtime-x86_64']),
                        str(app), str(output)], check=True, env=env)
        output.chmod(0o755)
        os.link(output, destination)
        checksum = f'{digest(output)}  {destination.name}\n'
        checksum_file = work / 'checksum'
        checksum_file.write_text(checksum)
        os.link(checksum_file, str(destination) + '.sha256')
    return destination


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--cache', type=Path, default=Path('target/package-tools'))
    args = parser.parse_args()
    try:
        print(build(args.source.resolve(), args.destination.absolute(), args.cache.resolve()))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'AppImage build refused: {error}\n')
