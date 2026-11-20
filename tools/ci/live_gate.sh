#!/usr/bin/env bash
# Copyright (c) 2026 Ferroan contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The required check `live (ubuntu-latest)` fails unless every parallel live
# job succeeded. A documentation-only run succeeds only when those jobs were
# skipped. Cancelled, failure and skipped (on a code change) all fail.

set -euo pipefail

scope="${SCOPE:?}"
docs="${DOCS_ONLY:?}"
scenarios="${SCENARIOS:?}"
ferrobot="${FERROBOT:?}"
services="${SERVICES:?}"
images="${IMAGES:?}"

require_result() {
  local name="$1"
  local expected="$2"
  local actual="$3"
  if [ "$actual" != "$expected" ]; then
    echo "$name result: $actual (expected $expected)" >&2
    exit 1
  fi
}

if [ "$scope" != "success" ]; then
  echo "live scope result: $scope" >&2
  exit 1
fi

if [ "$docs" = "true" ]; then
  require_result scenarios skipped "$scenarios"
  require_result ferrobot skipped "$ferrobot"
  require_result services skipped "$services"
  require_result images skipped "$images"
  exit 0
fi

require_result scenarios success "$scenarios"
require_result ferrobot success "$ferrobot"
require_result services success "$services"
require_result images success "$images"
