"""Download an Alpine package set and its dependency closure for the offline lab appliances.

Reads the package index shipped on the Alpine ISO (packages already present there are
skipped), the `main` and `community` indexes of the matching Alpine branch, resolves the
closure of the requested roots and downloads the .apk files into a directory. Packages are
signed by Alpine; apk verifies them when they are installed inside the lab.

    python3 lab/alpine_closure.py --iso alpine-virt-3.24.4-x86_64.iso --branch v3.24 \
        --out ~/lab-downloads/alpine-apks/nas nfs-utils nfs-utils-openrc rpcbind rpcbind-openrc
"""
import argparse
import re
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

MIRROR = "https://dl-cdn.alpinelinux.org/alpine"


def parse_index(path):
    packages = {}
    with tarfile.open(path) as archive:
        text = archive.extractfile("APKINDEX").read().decode()
    for block in text.split("\n\n"):
        record = {}
        for line in block.splitlines():
            if len(line) > 1 and line[1] == ":":
                record.setdefault(line[0], line[2:])
        if "P" in record:
            packages[record["P"]] = {
                "version": record["V"],
                "depends": record.get("D", "").split(),
                "provides": record.get("p", "").split(),
            }
    return packages


def name_of(dependency):
    return re.split("[=<>~]", dependency)[0]


def closure(roots, iso_index, repositories):
    provided_on_iso = set(iso_index)
    for record in iso_index.values():
        provided_on_iso.update(name_of(p) for p in record["provides"])
    providers = {}
    for repository, index in repositories.items():
        for name, record in index.items():
            providers.setdefault(name, (repository, name))
            for provided in record["provides"]:
                providers.setdefault(name_of(provided), (repository, name))
    needed = {}
    pending = list(roots)
    while pending:
        dependency = pending.pop()
        if dependency.startswith("!"):
            continue
        name = name_of(dependency)
        if name in provided_on_iso or name in needed:
            continue
        if name not in providers:
            raise ValueError(f"no provider for {name}")
        repository, package = providers[name]
        if package in needed or package in provided_on_iso:
            continue
        record = repositories[repository][package]
        needed[package] = (repository, record["version"])
        pending.extend(record["depends"])
    return needed


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--iso", required=True, help="Alpine ISO whose bundled packages need no download")
    parser.add_argument("--branch", required=True, help="Alpine branch of the ISO, such as v3.24")
    parser.add_argument("--arch", default="x86_64")
    parser.add_argument("--out", required=True, help="Directory that receives the .apk files")
    parser.add_argument("roots", nargs="+")
    args = parser.parse_args()
    out = Path(args.out).expanduser()
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        subprocess.run(["xorriso", "-osirrox", "on", "-indev", args.iso, "-extract",
                        f"/apks/{args.arch}/APKINDEX.tar.gz", str(work / "iso.tar.gz")],
                       check=True, capture_output=True)
        iso_index = parse_index(work / "iso.tar.gz")
        repositories = {}
        for repository in ("main", "community"):
            url = f"{MIRROR}/{args.branch}/{repository}/{args.arch}/APKINDEX.tar.gz"
            urllib.request.urlretrieve(url, work / f"{repository}.tar.gz")
            repositories[repository] = parse_index(work / f"{repository}.tar.gz")
    needed = closure(args.roots, iso_index, repositories)
    for package, (repository, version) in sorted(needed.items()):
        target = out / f"{package}-{version}.apk"
        if not target.exists():
            urllib.request.urlretrieve(f"{MIRROR}/{args.branch}/{repository}/{args.arch}/{target.name}", target)
        print(f"{repository}/{target.name}")
    print(f"{len(needed)} packages in {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
