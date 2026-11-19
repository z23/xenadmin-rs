#!/usr/bin/env python3
"""Include dependency declarations and supplied license notices in binary archives."""

import json
import pathlib
import re
import shutil
import subprocess
import sys

# The standard text of a licence, by SPDX identifier, for a package that declares the
# licence and ships no licence file. See packaging/licenses/README.md.
STANDARD_TEXTS = pathlib.Path(__file__).resolve().parent.parent / "packaging" / "licenses"
# Words every copy of a licence has, in lower case with single spaces: how a supplied notice
# is recognised as the text of the licence a package declares.
SIGNATURES = {
    "Apache-2.0": "apache license",
    "MIT": "permission is hereby granted, free of charge",
    "Zlib": "this software is provided 'as-is'",
}


def license_ids(expression):
    """The licence identifiers an SPDX expression names; `/` is the older way to write OR."""
    ids = []
    words = iter(re.split(r"[\s()/]+", expression or ""))
    for word in words:
        if word == "WITH":
            # What follows names an exception to the licence before it, not a licence.
            next(words, None)
        elif word and word not in ("OR", "AND"):
            ids.append(word)
    return list(dict.fromkeys(ids))


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
    without_text = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        root = pathlib.Path(package["manifest_path"]).parent
        target = destination / f'{package["name"]}-{package["version"]}'
        target.mkdir(exist_ok=True)
        files = list(supplied_notices(root))
        if package.get("license_file"):
            files.append(root / package["license_file"])
        # A workspace member or a vendored crate may keep the notice at the repository root.
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
        # The standard text of every licence a package names is kept once, beside the
        # packages, for a package that declares its licence and ships no text of it.
        ids = license_ids(package.get("license"))
        wanted = []
        if not copied:
            without_notices.append(f'{package["name"]} {package["version"]}')
            if (root / "README.md").is_file():
                shutil.copyfile(root / "README.md", target / "README.md")
            unknown = [name for name in ids if not (STANDARD_TEXTS / f"{name}.txt").is_file()]
            if unknown or not ids:
                without_text.append(
                    f'{package["name"]} {package["version"]}: {package.get("license") or "no licence declared"}')
            else:
                wanted = ids
        elif ids and all(name in SIGNATURES for name in ids):
            # What it ships may be the notice of something it embeds and not its own
            # licence (gpui-kit-assets: Apache-2.0, and only the notice of the icons).
            supplied = " ".join(
                " ".join((target / name).read_text(errors="replace").split()) for name in copied).lower()
            if not any(SIGNATURES[name] in supplied for name in ids):
                wanted = ids
        standard = []
        for name in wanted:
            (destination / "licenses").mkdir(exist_ok=True)
            shutil.copyfile(STANDARD_TEXTS / f"{name}.txt", destination / "licenses" / f"{name}.txt")
            standard.append(f"licenses/{name}.txt")
        declaration = {
            "name": package["name"], "version": package["version"],
            "license": package.get("license"), "source": package.get("source"),
            "repository": package.get("repository"), "notices": copied,
            "standard_texts": standard,
            "authors": package.get("authors", []),
        }
        # A registry crate may contain neither a separate notice nor a README.
        # Keep its provenance as an owned file instead of emitting an empty directory.
        (target / "package.json").write_text(json.dumps(declaration, indent=2) + "\n")
        index.append(declaration)
    if without_text:
        # Not a list to extend here: see packaging/licenses/README.md.
        sys.exit("No licence text, supplied or standard, for:\n" + "\n".join(without_text))
    (destination / "dependencies.json").write_text(json.dumps(index, indent=2) + "\n")
    (destination / "README.txt").write_text(
        "Dependency declarations and supplied notices for the locked Cargo graph.\n"
        "This includes development dependencies as well as runtime code for the build platform.\n"
        "See dependencies.json for versions, declared licenses, and source locations.\n"
        "Packages with no separate supplied notice (README included where available; the\n"
        "standard text of each licence they declare is in licenses/, named in their package.json):\n"
        + "\n".join(without_notices) + "\n"
    )
    print(f"Included notices for {len(index)} locked packages; "
          f"{len(without_notices)} without a separate supplied notice")


if __name__ == "__main__":
    main()
