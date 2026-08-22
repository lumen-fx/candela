#!/usr/bin/env python3
"""Set the version this source tree calls itself.

Usage:

    scripts/bump-version.py 0.0.5

The root `Cargo.toml` is where the version is decided, and three other places
copy it: `vm/Cargo.toml` carries the same number for `candela-vm`, the
`candela-vm` dependency in the root manifest asks for that exact version, and
`Cargo.lock` records it per crate. A copy that lags behind fails somewhere other
than where it was edited, so this moves all of them together and prints what it
touched.

Crates that do not publish keep versions of their own, on their own schedule:
`candela-lsp` and the Zed extension are numbered for the tools they are, not for
the release the toolchain is on, and this leaves them where they are.

It also leaves alone every version that names a release. The docs tell an
embedder to depend on a version that exists on crates.io, and the toolchain pins
in CI point at builds that exist. The version here is what the tree will be
next, which is a different thing.

`.github/workflows/release.yml` runs this after a release publishes, to move
`main` to the next patch version. Run it by hand to go somewhere other than the
next patch, or to repair a copy that drifted.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]

VERSION = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
SECTION = re.compile(r"^\s*\[(.+)\]\s*$")
# The lookbehind is what keeps `rust-version` out of this.
KEY_VERSION = re.compile(r'(?<![-\w.])(version\s*=\s*)"([^"]*)"')
NAME_VALUE = re.compile(r'^\s*name\s*=\s*"([^"]*)"\s*$')
PACKAGE_VERSION = re.compile(r'^\s*version\s*=\s*"([^"]+)"\s*$')
NO_PUBLISH = re.compile(r"^\s*publish\s*=\s*false\s*$")
HAS_PATH = re.compile(r"(?<![-\w.])path\s*=")
PACKAGE_RENAME = re.compile(r'(?<![-\w.])package\s*=\s*"([^"]*)"')
DEP_KEY = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*=")

# Directories with no manifest of ours in them. `target` holds vendored sources
# of other people's crates, which carry their own versions.
SKIP_DIRS = {".git", "target", "node_modules"}

# Files that write the version out in full, each for its own reason, and which
# therefore have to move every time. The rules below find them without being
# told to, so this is a tripwire rather than a work list: a bump that leaves one
# of them alone means a rule stopped matching, and stopping here is how that
# surfaces instead of becoming a version skew a merge queue finds later. Add a
# file here when a new one starts carrying the version.
REQUIRED = {
    # The version of `candela-lang`, and the pin on the `candela-vm` it builds
    # against.
    "Cargo.toml": "the compiler's version and its runtime pin",
    # The runtime ships in the same archive and reports the same number.
    "vm/Cargo.toml": "the runtime's version",
    # Every crate this repository builds is recorded here by version.
    "Cargo.lock": "the resolved versions",
}


class Edit(NamedTuple):
    """One file's rewritten text, held back until every file is accounted for."""

    path: Path
    text: str
    count: int


def fail(message: str) -> None:
    print(f"bump-version.py: {message}", file=sys.stderr)
    raise SystemExit(1)


def manifests() -> list[Path]:
    found = []
    for path in ROOT.rglob("Cargo.toml"):
        if SKIP_DIRS.isdisjoint(part for part in path.relative_to(ROOT).parts):
            found.append(path)
    return sorted(found)


def package_lines(text: str):
    """The lines of a manifest's `[package]` table.

    There is no `[workspace.package]` here to inherit from: the root manifest is
    both the workspace root and the `candela-lang` package, and every crate in
    the tree spells its own version out.
    """
    section = ""
    for line in text.split("\n"):
        header = SECTION.match(line)
        if header:
            section = header.group(1)
        elif section == "package":
            yield line


def package_key(text: str, pattern: re.Pattern[str]) -> str | None:
    """The value the first `[package]` line matching `pattern` captures."""
    matches = (pattern.match(line) for line in package_lines(text))
    return next((match.group(1) for match in matches if match), None)


def package_version(text: str) -> str | None:
    """The `[package] version`, which for the root manifest is the tree's."""
    return package_key(text, PACKAGE_VERSION)


def package_name(text: str) -> str | None:
    """The `[package] name` a manifest declares, if it declares one."""
    return package_key(text, NAME_VALUE)


def publishes(text: str) -> bool:
    """Whether this manifest's package is one the release publishes."""
    return not any(NO_PUBLISH.match(line) for line in package_lines(text))


def entries(lines: list[str]):
    """Walk a manifest as (section, logical line, line numbers).

    A dependency written as an inline table can wrap across lines, so the
    braces are counted rather than assuming one entry is one line.
    """
    section = ""
    index = 0
    while index < len(lines):
        line = lines[index]
        header = SECTION.match(line)
        if header:
            section = header.group(1)
            index += 1
            continue
        span = [index]
        depth = line.count("{") - line.count("}")
        while depth > 0 and span[-1] + 1 < len(lines):
            span.append(span[-1] + 1)
            depth += lines[span[-1]].count("{") - lines[span[-1]].count("}")
        yield section, "\n".join(lines[i] for i in span), span
        index = span[-1] + 1


def is_dependency_table(section: str) -> bool:
    """`[dependencies]`, and its dev, build, target and workspace spellings."""
    return section.split(".")[-1].endswith("dependencies")


def carries_tree_version(section: str, entry: str, ships: bool) -> bool:
    """Whether a `version` in this entry would mean "the version of this tree".

    A `[package] version` is the tree's when the crate is one the release ships,
    which is `candela-lang` and `candela-vm`. The language server and the Zed
    extension are numbered as the tools they are, so they keep what they carry.

    A `version` beside a `path` in a dependency table is one crate here asking
    for another: cargo takes the path when building from this checkout and the
    version when the crate is published, so the two have to agree.
    """
    if section == "package":
        return ships
    if not is_dependency_table(section):
        return False
    # A dependency with no `path` is somebody else's crate.
    return bool(re.search(HAS_PATH, entry))


def dependency_crate(entry: str) -> str | None:
    """The crate a dependency entry names, which `package` can rename."""
    rename = PACKAGE_RENAME.search(entry)
    if rename:
        return rename.group(1)
    key = DEP_KEY.match(entry)
    return key.group(1) if key else None


def skew(text: str, shipping: set[str], new: str) -> list[str]:
    """Every place in one manifest that should say `new` and does not.

    The tripwire below asks whether a file moved at all. This asks the question
    the file exists to answer, which is what catches the second literal in a
    file whose first one moved: a pin rewritten as `version = "=0.0.4"` stops
    matching the rule that moves it, and the file still counts as moved because
    the package version above it went.
    """
    found = []
    name = package_name(text)
    if name in shipping and package_version(text) != new:
        carried = package_version(text)
        found.append(f"{name} is {carried}" if carried else f"{name} declares no version")
    for section, entry, _ in entries(text.split("\n")):
        if not is_dependency_table(section) or not re.search(HAS_PATH, entry):
            continue
        crate = dependency_crate(entry)
        pin = KEY_VERSION.search(entry)
        if crate in shipping and pin and pin.group(2) != new:
            found.append(f"the {crate} dependency asks for {pin.group(2)}")
    return found


def rewrite_manifest(path: Path, shipping: set[str], old: str, new: str) -> Edit:
    """Move every version in one manifest that means "this tree".

    Only a value equal to the version the tree carries today is touched, which
    is what keeps an unrelated dependency that happens to sit near a version
    key out of it.
    """
    text = path.read_text(encoding="utf-8")
    lines = text.split("\n")
    changed = 0
    ships = package_name(text) in shipping

    for section, entry, span in entries(lines):
        if not carries_tree_version(section, entry, ships):
            continue

        def swap(match: re.Match[str]) -> str:
            nonlocal changed
            if match.group(2) != old:
                return match.group(0)
            changed += 1
            return f'{match.group(1)}"{new}"'

        replaced = KEY_VERSION.sub(swap, entry)
        if replaced != entry:
            for offset, line in enumerate(replaced.split("\n")):
                lines[span[offset]] = line

    return Edit(path, "\n".join(lines), changed)


def lock_blocks(lines: list[str]):
    """Walk `Cargo.lock` as (name, is_local, line numbers of the block).

    A `[[package]]` block with no `source` was resolved from a path in this
    checkout, which makes the lockfile the answer to which crates this
    repository builds. A crate that merely sits in the tree without being part
    of that graph, such as the Zed extension, is not in here.
    """
    starts = [i for i, line in enumerate(lines) if line == "[[package]]"]
    for index, start in enumerate(starts):
        end = starts[index + 1] if index + 1 < len(starts) else len(lines)
        block = lines[start:end]
        names = (NAME_VALUE.match(line) for line in block)
        name = next((match.group(1) for match in names if match), None)
        if name is None:
            continue
        local = not any(line.startswith("source = ") for line in block)
        yield name, local, range(start, end)


def lock_skew(text: str, shipping: set[str], new: str) -> list[str]:
    """Every crate the release ships whose lockfile block does not say `new`.

    Same question as `skew`, asked of the lockfile. Its blocks are per crate
    while the tripwire is per file, so one block lagging is invisible to a check
    that only asks whether the file moved.
    """
    lines = text.split("\n")
    recorded = {
        name: any(lines[index] == f'version = "{new}"' for index in span)
        for name, local, span in lock_blocks(lines)
        if local
    }
    return [
        f"{name} is {'not recorded' if name not in recorded else 'not at ' + new}"
        for name in sorted(shipping)
        if not recorded.get(name)
    ]


def rewrite_lock(path: Path, shipping: set[str], old: str, new: str) -> Edit:
    """Move the lockfile's record of every crate the release ships.

    The name decides, not the value: `candela-lsp` is on a version of its own
    and stays there even in the release where the two numbers happen to be the
    same, which is the case a value check alone would get wrong.

    The `dependencies` lists name crates without versions while the name is
    unambiguous, which it is for all three of these, so nothing else in the file
    moves with them.
    """
    lines = path.read_text(encoding="utf-8").split("\n")
    changed = 0
    for name, local, span in lock_blocks(lines):
        if not local or name not in shipping:
            continue
        for index in span:
            if lines[index] == f'version = "{old}"':
                lines[index] = f'version = "{new}"'
                changed += 1
    return Edit(path, "\n".join(lines), changed)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: bump-version.py <version>", file=sys.stderr)
        return 2
    new = argv[1].lstrip("v")
    if not VERSION.match(new):
        fail(f"'{argv[1]}' is not a version number")

    root_manifest = ROOT / "Cargo.toml"
    old = package_version(root_manifest.read_text(encoding="utf-8"))
    if old is None:
        fail("no [package] version in Cargo.toml")
    if old == new:
        print(f"the tree is already at {new}")
        return 0

    lock = ROOT / "Cargo.lock"
    local = {
        name
        for name, is_local, _ in lock_blocks(lock.read_text(encoding="utf-8").split("\n"))
        if is_local
    }
    # The crates the release ships: in the build graph this checkout resolves,
    # and not held back from the registry. That is `candela-lang` and
    # `candela-vm`, and it is what the version being set here belongs to.
    paths = manifests()
    shipping = set()
    for path in paths:
        text = path.read_text(encoding="utf-8")
        name = package_name(text)
        if name in local and publishes(text):
            shipping.add(name)

    manifest_edits = [rewrite_manifest(path, shipping, old, new) for path in paths]
    lock_edit = rewrite_lock(lock, shipping, old, new)
    edits = [*manifest_edits, lock_edit]

    total = 0
    moved = set()
    for edit in edits:
        total += edit.count
        if edit.count:
            moved.add(str(edit.path.relative_to(ROOT)))

    # Two questions, asked of the rewritten text and answered before any of it
    # reaches the disk. Whether each file that always carries the version moved,
    # and whether every literal naming a crate the release ships now reads as
    # the version being set. Either answer coming out wrong means a rule stopped
    # matching something, and stopping here is how that surfaces instead of
    # becoming a skew found later.
    problems = [
        f"  {name} still carries {old}, and holds {REQUIRED[name]}"
        for name in sorted(set(REQUIRED) - moved)
    ]
    for edit in manifest_edits:
        problems += [
            f"  {edit.path.relative_to(ROOT)}: {line}" for line in skew(edit.text, shipping, new)
        ]
    problems += [f"  Cargo.lock: {line}" for line in lock_skew(lock_edit.text, shipping, new)]
    if problems:
        print("\n".join(problems), file=sys.stderr)
        fail(f"the tree would not come out at {new}, so nothing was written")

    for edit in edits:
        if edit.count:
            edit.path.write_text(edit.text, encoding="utf-8")
            print(f"  {edit.path.relative_to(ROOT)}: {edit.count}")

    print(f"{old} -> {new}, {total} lines")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
