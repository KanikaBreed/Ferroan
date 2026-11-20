#!/usr/bin/env bash
# Copyright (c) 2026 Ferroan contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Write docs_only=true or docs_only=false to $GITHUB_OUTPUT.
# A push, a schedule and a manual run are never documentation-only.
# A fetch or diff failure fails closed, so the full suite still runs.

set -euo pipefail

write_result() {
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    echo "docs_only=$1" >>"$GITHUB_OUTPUT"
  else
    echo "docs_only=$1"
  fi
}

if [ "${EVENT_NAME:-}" != "pull_request" ]; then
  write_result false
  exit 0
fi

base="${BASE_SHA:-}"
if [ -z "$base" ]; then
  write_result false
  exit 0
fi

if ! git fetch --no-tags --depth=1 origin "$base"; then
  write_result false
  exit 0
fi

# --no-renames lists a renamed source file under its old name, so a rename
# from code to Markdown does not look like a documentation-only change.
if ! files="$(git diff --name-only --no-renames "$base" HEAD)"; then
  write_result false
  exit 0
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if printf '%s\n' "$files" | bash "$script_dir/docs_only.sh"; then
  write_result true
else
  write_result false
fi
