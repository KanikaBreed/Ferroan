#!/usr/bin/env python3
# Copyright (c) 2026 Ferroan contributors
#
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
"""Check Markdown links to local files, then format, lint and test the workspace."""

import argparse
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {".git", "target", ".scratch", "__pycache__"}
MD_LINK = re.compile(r"\[[^\]\n]*\]\(([^)\s]+)\)")

CARGO_STEPS = [
    ["cargo", "fmt", "--all", "--", "--check"],
    ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
    ["cargo", "test", "--workspace", "--locked"],
]


def markdown_files():
    for path in sorted(ROOT.rglob("*.md")):
        if not SKIP_DIRS.intersection(path.relative_to(ROOT).parts):
            yield path


def broken_links(path):
    fenced = False
    for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            continue
        if fenced:
            continue
        for match in MD_LINK.finditer(line):
            target = urlsplit(match.group(1).strip("<>"))
            if target.scheme or target.netloc or not target.path:
                continue  # remote URL or in-page anchor
            resolved = (path.parent / unquote(target.path)).resolve()
            if not resolved.is_relative_to(ROOT) or not resolved.exists():
                yield f"{path.relative_to(ROOT)}:{line_no}: missing local target {match.group(1)}"


def check_docs():
    files = list(markdown_files())
    problems = [problem for path in files for problem in broken_links(path)]
    for problem in problems:
        print(problem, file=sys.stderr)
    if not problems:
        print(f"Markdown: local links resolve in {len(files)} files.", flush=True)
    return not problems


def run_cargo():
    for step in CARGO_STEPS:
        print("$ " + " ".join(step), flush=True)
        try:
            subprocess.run(step, cwd=ROOT, check=True)
        except FileNotFoundError:
            print("cargo not found: install Rust through rustup, then reopen the terminal.", file=sys.stderr)
            return 1
        except subprocess.CalledProcessError as failure:
            return failure.returncode
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--docs-only", action="store_true", help="check Markdown links only")
    args = parser.parse_args()
    if not check_docs():
        return 1
    return 0 if args.docs_only else run_cargo()


if __name__ == "__main__":
    sys.exit(main())
