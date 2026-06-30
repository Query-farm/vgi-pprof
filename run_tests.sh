#!/usr/bin/env bash
# Copyright 2026 Query Farm LLC - https://query.farm
#
# Local end-to-end runner: build the release worker and run the haybarn
# SQLLogic suite (test/sql/*.test) against it over the stdio (subprocess)
# transport. CI runs the same suite across subprocess/http/unix via
# ci/run-integration.sh; this is the one-command local convenience wrapper.
#
# Usage:
#   ./run_tests.sh                       # whole suite, subprocess transport
#   ./run_tests.sh test/sql/stacks.test  # a single file (TEST_PATTERN)
#   TRANSPORT=unix ./run_tests.sh        # pick a transport
#
# Prereqs (one-time):
#   uv tool install haybarn-unittest
#   echo "INSTALL vgi FROM community;" | uvx haybarn-cli
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

echo "Building release worker ..."
cargo build --release --bin pprof-worker

# Resolve the haybarn-unittest runner (installed as a uv tool).
HAYBARN_UNITTEST="${HAYBARN_UNITTEST:-$(command -v haybarn-unittest || true)}"
if [[ -z "$HAYBARN_UNITTEST" ]]; then
  echo "ERROR: haybarn-unittest not found. Install it with:" >&2
  echo "       uv tool install haybarn-unittest" >&2
  exit 1
fi

export HAYBARN_UNITTEST
export WORKER_BIN="$HERE/target/release/pprof-worker"
export TRANSPORT="${TRANSPORT:-subprocess}"
# The runner stages preprocessed tests into a scratch dir and runs DuckDB there,
# so fixtures are referenced by absolute path via ${VGI_PPROF_DATA} (the .test
# files `require-env VGI_PPROF_DATA`).
export VGI_PPROF_DATA="$HERE/data"
[[ $# -ge 1 ]] && export TEST_PATTERN="$1"

exec ci/run-integration.sh
