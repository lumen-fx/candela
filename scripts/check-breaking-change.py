#!/usr/bin/env python3
"""Check a pull request's breaking-change label against its migration notes.

Usage:

    gh pr view 123 --json labels,files > pr.json
    scripts/check-breaking-change.py pr.json

A pull request that breaks something carries the `C-Breaking-Change` label and
adds a note under `docs/migration/unreleased/` saying what a user has to do
about it. This fails when one of the two is there without the other, and when
an added note does not start with a `# ` heading or holds anything but ASCII.
The heading is what the release notes list, so it has to be there.

The input is the JSON `gh pr view --json labels,files` prints: `labels` holds
objects with a `name`, `files` holds objects with a `path` and a `changeType`,
and a note counts when its `changeType` is `ADDED`. `gh pr view` lists at most
100 files, so `.github/workflows/breaking-change.yml` builds the same document
from the paginated REST listing instead. Notes are read from the working tree,
so run it from a checkout of the pull request.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LABEL = "C-Breaking-Change"
UNRELEASED = "docs/migration/unreleased/"


def added_notes(files: list[dict]) -> list[str]:
    notes = []
    for entry in files:
        path = entry.get("path", "")
        if entry.get("changeType") != "ADDED" or not path.startswith(UNRELEASED):
            continue
        if Path(path).name == ".gitkeep":
            continue
        notes.append(path)
    return sorted(notes)


def note_problems(path: str) -> list[str]:
    data = (ROOT / path).read_bytes()
    problems = []
    if not data.startswith(b"# "):
        problems.append(f"{path} does not start with a '# ' heading")
    for number, line in enumerate(data.split(b"\n"), start=1):
        if any(byte > 0x7F for byte in line):
            problems.append(f"{path}:{number} holds a character outside ASCII")
    return problems


def check(pr: dict) -> list[str]:
    labelled = any(label.get("name") == LABEL for label in pr.get("labels", []))
    notes = added_notes(pr.get("files", []))

    problems = []
    if labelled and not notes:
        problems.append(
            f"the pull request is labelled {LABEL} but adds no note under "
            f"{UNRELEASED}; add one saying what changed and what a user does about it"
        )
    if notes and not labelled:
        problems.append(
            f"the pull request adds {', '.join(notes)} but is not labelled "
            f"{LABEL}; add the label, or take the note out if nothing breaks"
        )
    for note in notes:
        problems.extend(note_problems(note))
    return problems


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: scripts/check-breaking-change.py <pr.json | ->", file=sys.stderr)
        return 2
    source = sys.argv[1]
    text = sys.stdin.read() if source == "-" else Path(source).read_text()
    problems = check(json.loads(text))
    for problem in problems:
        print(f"check-breaking-change.py: {problem}", file=sys.stderr)
    if problems:
        return 1
    print("the label and the migration notes agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
