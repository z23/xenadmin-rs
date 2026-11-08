#!/usr/bin/env python3
"""Include dependency declarations and supplied license notices in binary archives."""

import json
import pathlib
import shutil
import subprocess
import sys


def supplied_notices(directory):
    prefixes = ("license", "licence", "copying", "copyright", "notice", "unlicense")
    for entry in sorted(directory.iterdir()):
        if entry.name.lower().startswith(prefixes):
            if entry.is_file():
                yield entry
            elif entry.is_dir():
                yield from (item for item in sorted(entry.rglob("*")) if item.is_file())


def main():
    destination = pathlib.Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=True)
    host = next(line.removeprefix("host: ") for line in subprocess.check_output(
        ["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--offline", "--format-version", "1",
        "--filter-platform", host,
    ]))
    index = []
    without_notices = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        root = pathlib.Path(package["manifest_path"]).parent
        target = destination / f'{package["name"]}-{package["version"]}'
        target.mkdir(exist_ok=True)
        files = list(supplied_notices(root))
        if package.get("license_file"):
            files.append(root / package["license_file"])
        # Workspace members, including Rune, may keep the notice at the repository root.
        if not files and not (package.get("source") or "").startswith("registry+"):
            for parent in list(root.parents)[:3]:
                files = list(supplied_notices(parent))
                if files:
                    break
        copied = []
        for number, source in enumerate(dict.fromkeys(files)):
            if source.is_file():
                filename = f"{number:02}-{source.name}"
                shutil.copyfile(source, target / filename)
                copied.append(filename)
        if not copied:
            without_notices.append(f'{package["name"]} {package["version"]}')
            if (root / "README.md").is_file():
                shutil.copyfile(root / "README.md", target / "README.md")
        declaration = {
            "name": package["name"], "version": package["version"],
            "license": package.get("license"), "source": package.get("source"),
            "repository": package.get("repository"), "notices": copied,
            "authors": package.get("authors", []),
        }
        # A registry crate may contain neither a separate notice nor a README.
        # Keep its provenance as an owned file instead of emitting an empty directory.
        (target / "package.json").write_text(json.dumps(declaration, indent=2) + "\n")
        index.append(declaration)
        # Rune embeds these fonts; retain their notices alongside the crate notice.
        if package["name"] == "rune" and (root / "assets").is_dir():
            for notice in (root / "assets").glob("*-LICENSE.txt"):
                shutil.copyfile(notice, target / notice.name)
    (destination / "dependencies.json").write_text(json.dumps(index, indent=2) + "\n")
    (destination / "README.txt").write_text(
        "Dependency declarations and supplied notices for the locked Cargo graph.\n"
        "This includes development dependencies as well as runtime code for the build platform.\n"
        "See dependencies.json for versions, declared licenses, and source locations.\n"
        "Packages with no separate supplied notice (README included where available):\n"
        + "\n".join(without_notices) + "\n"
    )
    print(f"Included notices for {len(index)} locked packages; "
          f"{len(without_notices)} without a separate supplied notice")


if __name__ == "__main__":
    main()
