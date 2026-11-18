#!/usr/bin/env python3
"""Regenerate and check the mechanical parts of docs/GPUI-MIGRATION-LEDGER.md.

usage:
  gpui_ledger_tables.py scenes     print the Scenes table: every Rune scene name of
                                   crates/xenadmin-ui/src/scene.rs with the area that registers it
  gpui_ledger_tables.py tests      print every #[test] of the Rune crate with its kind and the
                                   tests of the new crates found by its name (needs the Rune crate)
  gpui_ledger_tables.py keys       print every element key of the Rune crate that is no string
                                   literal of the new crates (needs the Rune crate)
  gpui_ledger_tables.py check [--commands FILE] [LEDGER]
                                   compare the ledger with the repository; exit 1 on a difference

What `check` compares, and what it needs:

  always                     every Markdown table of the ledger has one cell count per table;
                             the Scenes table is what `scenes` prints;
                             every successor of the Tests table is a test function of the file
                             it names; the "Tests by disposition" table counts the Tests table
  with --commands FILE       the Commands table is what scripts/gpui_commands_table.py prints
                             from FILE (the output of `xenadmin-scenes --commands`)
  while crates/xenadmin-desktop exists
                             the Tests table has exactly the tests of the Rune crate with their
                             kind, and every test found by name is among the row's successors;
                             the appendix of Rune controls has exactly the absent keys

Once the Rune crate is deleted the last group cannot be checked any more: the Rune columns of
the Tests table and the appendix are then a record, generated while the crate existed.

Places are abbreviated as in the ledger: `ui:x` is crates/xenadmin-ui/src/x and `gpui:x` is
crates/xenadmin-gpui/x. A successor is written `place::function`.
"""

import collections
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
RUNE = REPO / "crates/xenadmin-desktop/src"
UI = REPO / "crates/xenadmin-ui/src"
GPUI = REPO / "crates/xenadmin-gpui"
LEDGER = REPO / "docs/GPUI-MIGRATION-LEDGER.md"

TEST = re.compile(
    r"#\[(?:gpui_kit::|tokio::)?test[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub(?:\([a-z]+\))?\s+)?(?:async\s+)?fn\s+([a-zA-Z0-9_]+)",
    re.S,
)
SCENE = re.compile(r'SceneSpec::new\(\s*"([^"]+)"\s*,\s*("[^"]+"|[A-Z_]+)')
RUNTIME_HELPER = re.compile(r"fn\s+([a-z0-9_]+)\s*\([^)]*\)\s*->\s*(?:rune::)?Runtime<")
KEY = re.compile(r'\.key\(\s*"([^"]+)"\s*\)')
FORMATTED_KEY = re.compile(r'\.key\(\s*format!\(\s*"([^"]+)"')
SCENE_STATUS = "registered, renders headlessly"


def place(path):
    """The ledger's short form of a path of the new crates."""
    relative = path.relative_to(REPO).as_posix()
    if relative.startswith("crates/xenadmin-ui/src/"):
        return "ui:" + relative[len("crates/xenadmin-ui/src/") :]
    if relative.startswith("crates/xenadmin-gpui/"):
        return "gpui:" + relative[len("crates/xenadmin-gpui/") :]
    return relative


def path_of(short):
    if short.startswith("ui:"):
        return UI / short[3:]
    if short.startswith("gpui:"):
        return GPUI / short[5:]
    return REPO / short


def new_sources():
    for root in (UI, GPUI / "src", GPUI / "tests"):
        yield from sorted(root.rglob("*.rs"))


def new_tests():
    """Test function name -> places, over the new crates."""
    found = collections.defaultdict(list)
    for path in new_sources():
        for name in TEST.findall(path.read_text(encoding="utf-8")):
            found[name].append(place(path))
    return found


# ---------------------------------------------------------------- scenes


def rune_scene_names():
    text = (UI / "scene.rs").read_text(encoding="utf-8")
    listed = re.search(r"RUNE_APP_SCENES: \[&str; \d+\] = \[(.*?)\];", text, re.S)
    names = re.findall(r'"([^"]+)"', listed.group(1))
    window = re.search(r'CONSOLE_WINDOW_SCENE: &str = "([^"]+)"', text)
    return names + [window.group(1)]


def registered_scenes():
    """Scene name -> (area, place of the registration), from the SceneSpec::new calls."""
    found = {}
    for path in sorted(UI.rglob("*.rs")):
        if path.name.endswith("tests.rs"):
            continue
        short = place(path)
        for name, area in SCENE.findall(path.read_text(encoding="utf-8")):
            if area.startswith('"'):
                area = area.strip('"')
            elif short.startswith("ui:areas/"):
                area = short.split("/")[1]
            else:
                area = "kernel"
            found[name] = (area, short)
    return found


def scene_rows():
    registered = registered_scenes()
    rows = []
    for name in rune_scene_names():
        area, where = registered.get(name, ("", ""))
        status = SCENE_STATUS if area else "not registered"
        rows.append([f"`{name}`", f"`{area}`" if area else "", f"`{where}`" if where else "", status])
    return rows


def other_scenes():
    rune = set(rune_scene_names())
    return sorted((name, area) for name, (area, _) in registered_scenes().items() if name not in rune)


SCENE_HEADER = ["scene", "area", "registered in", "status"]

# ---------------------------------------------------------------- tests


def body_after(text, start):
    """The braces block that follows `start` (a function's body), or the rest of the text."""
    opening = text.find("{", start)
    if opening < 0:
        return ""
    depth = 0
    for index in range(opening, len(text)):
        character = text[index]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return text[opening : index + 1]
    return text[opening:]


def rune_tests():
    """(file, test, kind) for every #[test] of the Rune crate, in file order.

    A test is `runtime` when its body names `Runtime` or calls a function of its file (or the
    usual `runtime` helper of a sibling file) that returns a `rune::Runtime`; else `pure`.
    """
    rows = []
    for path in sorted(RUNE.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        helpers = set(RUNTIME_HELPER.findall(text)) | {"runtime", "runtime_of"}
        # a helper that takes the runtime (click, settle, ...) marks a runtime test as well
        helpers |= set(re.findall(r"fn\s+([a-z0-9_]+)\s*\([^)]*Runtime<", text))
        call = re.compile(r"\b(?:" + "|".join(sorted(helpers)) + r")\s*\(")
        for match in TEST.finditer(text):
            body = body_after(text, match.end())
            runtime = "Runtime" in body or call.search(body)
            rows.append((path.name, match.group(1), "runtime" if runtime else "pure"))
    return rows


def by_name(name, tests):
    """The tests of the new crates that carry a Rune test's name, or `rune_` and its name."""
    found = []
    for candidate in (name, "rune_" + name):
        found += [f"{where}::{candidate}" for where in tests.get(candidate, [])]
    return found


TEST_HEADER = ["Rune file", "Rune test", "kind", "found by name"]


def test_rows():
    tests = new_tests()
    return [
        [f"`{file}`", f"`{name}`", kind, "; ".join(f"`{s}`" for s in by_name(name, tests))]
        for file, name, kind in rune_tests()
    ]


# ---------------------------------------------------------------- keys


def absent_keys():
    """(file, key) for every element key of the Rune crate that the new crates do not have.

    A literal key is absent when it is no string literal of the new crates; a formatted key
    (`format!("prefix-{..}")`) when no string literal there starts with its prefix.
    """
    new = "".join(path.read_text(encoding="utf-8") for path in new_sources())
    rows = []
    for path in sorted(RUNE.glob("*.rs")):
        if path.name.endswith("_tests.rs"):
            continue
        body = path.read_text(encoding="utf-8").split("#[cfg(test)]")[0]
        for key in sorted(set(KEY.findall(body))):
            if f'"{key}"' not in new:
                rows.append((path.name, key))
        for pattern in sorted(set(FORMATTED_KEY.findall(body))):
            prefix = pattern.split("{")[0]
            if not prefix or f'"{prefix}' not in new:
                rows.append((path.name, pattern))
    return rows


# ---------------------------------------------------------------- Markdown


def cells(line):
    """The cells of one table row; `\\|` is a bar inside a cell."""
    inner = line.strip()
    inner = inner[1:] if inner.startswith("|") else inner
    inner = inner[:-1] if inner.endswith("|") and not inner.endswith("\\|") else inner
    return [cell.strip().replace("\\|", "|") for cell in re.split(r"(?<!\\)\|", inner)]


def tables(text):
    """[(heading, first line number, header cells, rows)] for every table of a document."""
    found, heading, lines = [], "", text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        if line.startswith("#"):
            heading = line.lstrip("#").strip()
        if line.startswith("|") and index + 1 < len(lines) and re.match(r"^\|[ :|-]+\|$", lines[index + 1]):
            header, start, rows = cells(line), index + 1, []
            index += 2
            while index < len(lines) and lines[index].startswith("|"):
                rows.append((index + 1, cells(lines[index])))
                index += 1
            found.append((heading, start, header, rows))
            continue
        index += 1
    return found


def malformed(text):
    """Complaints about rows whose cell count is not their header's."""
    problems = []
    for heading, start, header, rows in tables(text):
        for number, row in rows:
            if len(row) != len(header):
                problems.append(f"line {number} ({heading}): {len(row)} cells, the header has {len(header)}")
    return problems


def table_under(text, heading):
    for name, _, header, rows in tables(text):
        if name == heading:
            return header, [row for _, row in rows]
    return None, []


def markdown(header, rows):
    out = ["| " + " | ".join(header) + " |", "| " + " | ".join("---" for _ in header) + " |"]
    out += ["| " + " | ".join(cell.replace("|", "\\|") for cell in row) + " |" for row in rows]
    return "\n".join(out) + "\n"


def bare(cell):
    return cell.strip().strip("`")


def successors(cell):
    """The `place::function` items of a successor cell."""
    return re.findall(r"`([a-z]+:[^`]+?)::([A-Za-z0-9_]+)`", cell)


# ---------------------------------------------------------------- check


def check(ledger, commands):
    text = ledger.read_text(encoding="utf-8")
    problems = malformed(text)
    notes = []

    # Scenes
    _, rows = table_under(text, "Scenes")
    expected = scene_rows()
    if rows != expected:
        problems.append(f"Scenes: the table differs from `scenes` ({len(rows)} rows, expected {len(expected)})")
        for have, want in zip(rows, expected):
            if have != want:
                problems.append(f"  first difference: {have} / expected {want}")
                break
    registered = sum(1 for row in expected if row[3] == SCENE_STATUS)
    notes.append(f"scenes: {len(expected)} Rune names, {registered} registered")

    # Tests
    header, rows = table_under(text, "Tests")
    tests = new_tests()
    counts = collections.Counter()
    for row in rows:
        counts[row[3].split()[0].rstrip(":,;") if row[3] else "(none)"] += 1
        for where, function in successors(row[4]):
            if where not in tests.get(function, []):
                problems.append(f"Tests: {row[1]}: no test `{function}` in {where}")
        if not successors(row[4]) and not row[3].startswith("retired"):
            problems.append(f"Tests: {row[1]}: no successor and not retired")
    notes.append(f"tests: {len(rows)} rows; " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    _, summary = table_under(text, "Tests by disposition")
    stated = {row[0]: int(row[1]) for row in summary if row[1].isdigit()}
    wanted = dict(counts)
    wanted["total"] = len(rows)
    if stated != wanted:
        problems.append(f"Tests by disposition: the table says {stated}, the Tests table has {wanted}")

    if RUNE.is_dir():
        rune = rune_tests()
        have = [(bare(row[0]), bare(row[1]), row[2]) for row in rows]
        if have != rune:
            missing = [r for r in rune if r not in have]
            extra = [r for r in have if r not in rune]
            problems.append(f"Tests: rows differ from the Rune crate ({len(have)} rows, {len(rune)} tests); missing {missing[:5]}, extra {extra[:5]}")
        for row in rows:
            named = set(successors(row[4]))
            for found in by_name(bare(row[1]), tests):
                where, function = found.rsplit("::", 1)
                if (where, function) not in named:
                    problems.append(f"Tests: {row[1]}: `{found}` carries its name and is not a successor")
        _, appendix = table_under(text, "Appendix: Rune controls without a key in the new crates")
        have = sorted((bare(row[0]), bare(row[1])) for row in appendix)
        want = sorted(absent_keys())
        if have != want:
            missing = [k for k in want if k not in have]
            extra = [k for k in have if k not in want]
            problems.append(f"Appendix: rows differ from the absent keys; missing {missing[:8]}, extra {extra[:8]}")
        notes.append(f"appendix: {len(want)} absent keys of the Rune crate")
    else:
        notes.append("the Rune crate is gone: the Rune columns of Tests and the appendix were not compared")

    # Commands
    if commands:
        sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
        import gpui_commands_table

        generated = gpui_commands_table.table(pathlib.Path(commands).read_text(encoding="utf-8"))
        _, rows = table_under(text, "Commands")
        _, want = table_under("## Commands\n\n" + generated, "Commands")
        if rows != want:
            problems.append(f"Commands: the table differs from gpui_commands_table.py ({len(rows)} rows, expected {len(want)})")
        notes.append(f"commands: {len(want)} registered")
    else:
        _, rows = table_under(text, "Commands")
        notes.append(f"commands: {len(rows)} rows, not compared (no --commands FILE)")

    for note in notes:
        print(note)
    for problem in problems:
        print("DIFFERENCE:", problem)
    print("ledger matches the repository" if not problems else f"{len(problems)} difference(s)")
    return 1 if problems else 0


def main(arguments):
    if not arguments or arguments[0] in ("-h", "--help"):
        raise SystemExit(__doc__)
    mode, rest = arguments[0], arguments[1:]
    if mode == "scenes":
        sys.stdout.write(markdown(SCENE_HEADER, scene_rows()))
    elif mode == "tests":
        if not RUNE.is_dir():
            raise SystemExit("crates/xenadmin-desktop is gone: the Rune tests cannot be listed any more")
        sys.stdout.write(markdown(TEST_HEADER, test_rows()))
    elif mode == "keys":
        if not RUNE.is_dir():
            raise SystemExit("crates/xenadmin-desktop is gone: the Rune keys cannot be listed any more")
        sys.stdout.write(markdown(["Rune file", "key"], [[f"`{f}`", f"`{k}`"] for f, k in absent_keys()]))
    elif mode == "check":
        commands = None
        if "--commands" in rest:
            position = rest.index("--commands")
            commands = rest[position + 1]
            del rest[position : position + 2]
        raise SystemExit(check(pathlib.Path(rest[0]) if rest else LEDGER, commands))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
