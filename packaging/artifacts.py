#!/usr/bin/env python3
"""Stage only executables reported by the current Cargo invocation."""
import argparse
import json
import os
from pathlib import Path
import shutil
from install import regular


def executables(messages, arch):
    required = {'xenadmin-rs', 'host-probe'}
    found = {}
    for line in messages.read_text().splitlines():
        message = json.loads(line)
        target = message.get('target', {})
        name = target.get('name')
        if message.get('reason') != 'compiler-artifact' or name not in required or 'bin' not in target.get('kind', []) or message.get('profile', {}).get('test'):
            continue
        path = message.get('executable')
        if path:
            if name in found and found[name] != path:
                raise ValueError(f'Multiple build artifacts for {name}')
            found[name] = path
    if set(found) != required:
        raise ValueError('The build did not report both required executable artifacts')
    machine = {'x86_64': 62, 'aarch64': 183}.get(arch)
    if machine is None:
        raise ValueError('Unsupported package architecture')
    for path in found.values():
        regular(Path(path))
        with open(path, 'rb') as stream:
            header = stream.read(20)
        if len(header) != 20 or header[:6] != b'\x7fELF\x02\x01' or int.from_bytes(header[18:20], 'little') != machine:
            raise ValueError('Cargo target executable does not match the native Linux package architecture')
    return {name: Path(path) for name, path in found.items()}


def stage(messages, destination, arch):
    found = executables(messages, arch)
    destination.mkdir(parents=True, exist_ok=True)
    for name, path in found.items():
        shutil.copyfile(path, destination / name)
        (destination / name).chmod(0o755)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('messages', type=Path)
    parser.add_argument('destination', type=Path, nargs='?')
    parser.add_argument('--print-path', choices=('xenadmin-rs', 'host-probe'))
    args = parser.parse_args()
    if args.print_path:
        if args.destination is not None:
            parser.error('Choose a destination or --print-path')
        print(executables(args.messages, os.uname().machine)[args.print_path])
    elif args.destination is not None:
        stage(args.messages, args.destination, os.uname().machine)
    else:
        parser.error('Provide a destination or --print-path')
