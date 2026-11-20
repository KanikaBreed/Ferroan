#!/usr/bin/env bash
# Copyright (c) 2026 Ferroan contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Exit 0 when every path on stdin is Markdown and none is a file a program
# reads. Exit 1 otherwise, including an empty list (fail closed).
#
# Program-read Markdown, from a search of xtask/src on 20 November 2026:
#   docs/REFERENCES.md  pin and the Tranquillity cache key
#   docs/LIMITS.md      limit_ids::check, and crates/ferroan-diag
#   README.md           named with those two; the link checker addresses it
# No other Markdown path is an input.

set -euo pipefail

found=0
while IFS= read -r path || [ -n "$path" ]; do
  [ -z "$path" ] && continue
  found=1
  case "$path" in
    *.md) ;;
    *) exit 1 ;;
  esac
  case "$path" in
    README.md | docs/REFERENCES.md | docs/LIMITS.md) exit 1 ;;
  esac
done

[ "$found" -eq 1 ]
