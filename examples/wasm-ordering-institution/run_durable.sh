#!/usr/bin/env bash

# Copyright 2026 The Eigenius Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Durability smoke test for the wasm-ordering-institution example.
#
# Exercises the Phase 9a `--db` path end-to-end through the CLI surface:
#
#   Round 1: start kernel with a fresh RocksDB (SEED), install the
#            institution, `capability list`, inspect a declared class,
#            kill the kernel.
#   Round 2: restart the kernel pointed at the *same* RocksDB (RESUME),
#            verify `capability list` still shows the institution, the
#            declared class is still inspectable, and a FIBER dispatch
#            still returns the expected result — all without re-installing.
#
# This is the CLI-surface counterpart to the in-process Rust test at
# `storage/rocksdb/tests/durability_test.rs`. Both must stay green.
# See D13 — Durable Kernel State.
#
# Prerequisite: `cargo build -p eigenius-cli` (or --release; override
# KERNEL_BIN below).
#
# Overrides:
#   PORT          — gRPC port (default: 50099)
#   KERNEL_BIN    — path to the eigenius CLI binary
#   FIXTURE       — path to the pre-built institution .wasm

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

PORT="${PORT:-50099}"
KERNEL_BIN="${KERNEL_BIN:-$REPO_ROOT/target/debug/eigenius}"
FIXTURE="${FIXTURE:-$REPO_ROOT/kernel/tests/fixtures/eigenius_wasm_ordering_institution.wasm}"

if [[ ! -x "$KERNEL_BIN" ]]; then
  echo "error: kernel binary not found at $KERNEL_BIN" >&2
  echo "hint:  cargo build -p eigenius-cli" >&2
  exit 1
fi
if [[ ! -f "$FIXTURE" ]]; then
  echo "error: institution fixture not found at $FIXTURE" >&2
  echo "hint:  cd $SCRIPT_DIR && cargo component build --release" >&2
  echo "       cp target/wasm32-unknown-unknown/release/eigenius_wasm_ordering_institution.wasm $FIXTURE" >&2
  exit 1
fi

ENDPOINT="http://localhost:$PORT"
INSTITUTION_IRI="urn:eigenius:test:wasm:ordering"
REFINEMENT_CLASS="urn:eigenius:test:wasm:Refinement"
TMPDIR="$(mktemp -d)"
DB="$TMPDIR/kernel.db"
KERNEL_PID=""

cleanup() {
  if [[ -n "$KERNEL_PID" ]]; then
    kill "$KERNEL_PID" 2>/dev/null || true
    wait "$KERNEL_PID" 2>/dev/null || true
    KERNEL_PID=""
  fi
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

start_kernel() {
  local log="$1"
  "$KERNEL_BIN" serve --port "$PORT" --db "$DB" >"$log" 2>&1 &
  KERNEL_PID=$!
  for _ in $(seq 1 50); do
    if "$KERNEL_BIN" --endpoint "$ENDPOINT" inspect urn:eigenius:core:Class >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "error: kernel did not become ready" >&2
  cat "$log" >&2
  exit 1
}

stop_kernel() {
  if [[ -n "$KERNEL_PID" ]]; then
    kill "$KERNEL_PID" 2>/dev/null || true
    wait "$KERNEL_PID" 2>/dev/null || true
    KERNEL_PID=""
  fi
}

assert_contains() {
  local label="$1"
  local haystack="$2"
  local needle="$3"
  if [[ "$haystack" != *"$needle"* ]]; then
    echo "FAIL: $label — missing '$needle' in output:" >&2
    echo "$haystack" >&2
    exit 1
  fi
}

# ---- Round 1: SEED + install -------------------------------------------
echo "=== Round 1: fresh DB at $DB ==="
start_kernel "$TMPDIR/kernel1.log"

echo "--- install institution ---"
"$KERNEL_BIN" --endpoint "$ENDPOINT" capability install "$FIXTURE" \
  --as-iri "$INSTITUTION_IRI" --kind institution

CAPS1=$("$KERNEL_BIN" --endpoint "$ENDPOINT" capability list)
assert_contains "round1 capability list" "$CAPS1" "$INSTITUTION_IRI"

"$KERNEL_BIN" --endpoint "$ENDPOINT" inspect "$REFINEMENT_CLASS" >/dev/null
echo "    $REFINEMENT_CLASS found (post-install, #15)"

stop_kernel

# ---- Round 2: RESUME + dispatch ----------------------------------------
echo
echo "=== Round 2: restart against the same DB ==="
start_kernel "$TMPDIR/kernel2.log"

if ! grep -q "Rehydrated WASM institution: $INSTITUTION_IRI" "$TMPDIR/kernel2.log"; then
  echo "FAIL: kernel2.log missing rehydration log line" >&2
  tail -40 "$TMPDIR/kernel2.log" >&2
  exit 1
fi
echo "    kernel2 log confirms rehydration"

CAPS2=$("$KERNEL_BIN" --endpoint "$ENDPOINT" capability list)
assert_contains "round2 capability list" "$CAPS2" "$INSTITUTION_IRI"

"$KERNEL_BIN" --endpoint "$ENDPOINT" inspect "$REFINEMENT_CLASS" >/dev/null
echo "    $REFINEMENT_CLASS still found after restart"

cat >"$TMPDIR/conv-query.json" <<EOF
{
  "@id": "urn:eigenius:test:dur:query-1",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.005
}
EOF
DISPATCH=$("$KERNEL_BIN" --endpoint "$ENDPOINT" \
  capability test "$INSTITUTION_IRI" --input "$TMPDIR/conv-query.json")
assert_contains "post-restart dispatch" "$DISPATCH" '"urn:eigenius:test:wasm:converged": true'
echo "    dispatch returns converged=true after restart"

stop_kernel

echo
echo "=== OK — durability loop survived kernel restart ==="
