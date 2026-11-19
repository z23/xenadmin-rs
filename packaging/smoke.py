#!/usr/bin/env python3
"""Runs a built client once, without a window, and checks that it answers.

    smoke.py EXECUTABLE [--appimage]

The client is asked to draw a scene to a file. A packaged client is built without the
scene harness and refuses that in one line with status 1. The refusal comes from the
client's own main function: the loader found every library the executable is linked
against, and the client read its command line. It is answered before a display, the state
directory or the keyring is touched, so nothing is opened and nothing is written.

What this does not show: the libraries the client loads by name when it opens a window
(Wayland, Vulkan, EGL), a font, or a window. With --appimage the image is run with
APPIMAGE_EXTRACT_AND_RUN=1, so that it needs no FUSE: its runtime and AppRun hand the
arguments to the client inside.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

# Part of the client's refusal (render_to_files in crates/xenadmin-desktop/src/app.rs).
REFUSAL = 'made without the `scenes` feature'
SECONDS = 120


def check(executable, appimage=False):
    """Returns None when the client answered as a packaged client does, else what is wrong."""
    with tempfile.TemporaryDirectory(prefix='xenadmin-rs-smoke.') as scratch:
        scratch = Path(scratch)
        state = scratch / 'state'
        state.mkdir()
        snap = scratch / 'frame.png'
        environment = {name: value for name, value in os.environ.items()
                       if name not in ('DISPLAY', 'WAYLAND_DISPLAY', 'WAYLAND_SOCKET')}
        environment['XENADMIN_STATE_DIR'] = str(state)
        if appimage:
            environment['APPIMAGE_EXTRACT_AND_RUN'] = '1'
            # The runtime extracts under TMPDIR: it goes away with the scratch directory.
            environment['TMPDIR'] = str(scratch)
        try:
            done = subprocess.run([str(executable), '--scene', 'demo', '--snap', str(snap)],
                                  env=environment, stdin=subprocess.DEVNULL, capture_output=True,
                                  text=True, errors='replace', timeout=SECONDS)
        except subprocess.TimeoutExpired:
            return f'{executable} did not end within {SECONDS} s'
        except OSError as error:
            return f'{executable} could not be started: {error}'
        said = (done.stderr or '').strip()
        if done.returncode != 1 or REFUSAL not in said:
            return (f'{executable} ended with status {done.returncode}, not with the refusal of a '
                    f'packaged client (status 1, "{REFUSAL}"). It said: {said or "nothing"}')
        if snap.exists() or any(state.iterdir()):
            return f'{executable} wrote a file although it refused'
    return None


def main(arguments):
    appimage = '--appimage' in arguments
    paths = [argument for argument in arguments if argument != '--appimage']
    if len(paths) != 1:
        print('Usage: smoke.py EXECUTABLE [--appimage]', file=sys.stderr)
        return 2
    wrong = check(Path(paths[0]).resolve(), appimage)
    if wrong:
        print(f'smoke.py: {wrong}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
