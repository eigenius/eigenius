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

# End-to-end smoke test for the wasm-ordering-institution example.
#
# Spins up a local Eigenius kernel, installs this institution, runs the
# convergence fiber query twice (a converged case and a not-converged
# case), and prints the institution's metadata via `capability inspect`.
# Kills the kernel on exit.
#
# Prerequisite: `cargo build --release -p eigenius-cli` (or run
# `cargo build -p eigenius-cli` and point KERNEL_BIN at target/debug/).
#
# Overrides:
#   PORT          — gRPC port to use (default: 50099)
#   KERNEL_BIN    — path to the eigenius CLI binary
#   FIXTURE       — path to the pre-built institution .wasm

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
REPO_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

PORT="${PORT:-50099}"
KERNEL_BIN="${KERNEL_BIN:-$REPO_ROOT/target/release/eigenius}"
FIXTURE="${FIXTURE:-$REPO_ROOT/kernel/tests/fixtures/eigenius_wasm_ordering_institution.wasm}"

if [[ ! -x "$KERNEL_BIN" ]]; then
  echo "error: kernel binary not found at $KERNEL_BIN" >&2
  echo "hint:  cargo build --release -p eigenius-cli" >&2
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
TMPDIR="$(mktemp -d)"
LOG="$TMPDIR/kernel.log"

cleanup() {
  if [[ -n "${KERNEL_PID:-}" ]]; then
    kill "$KERNEL_PID" 2>/dev/null || true
    wait "$KERNEL_PID" 2>/dev/null || true
  fi
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

echo "=== starting kernel on port $PORT ==="
"$KERNEL_BIN" serve --port "$PORT" >"$LOG" 2>&1 &
KERNEL_PID=$!

# Poll the kernel's Health RPC until it's up.
for _ in $(seq 1 50); do
  if "$KERNEL_BIN" --endpoint "$ENDPOINT" inspect urn:eigenius:core:Class >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

echo
echo "=== installing institution ==="
"$KERNEL_BIN" --endpoint "$ENDPOINT" \
  capability install "$FIXTURE" \
  --as-iri "$INSTITUTION_IRI" \
  --kind institution

echo
echo "=== capability list ==="
"$KERNEL_BIN" --endpoint "$ENDPOINT" capability list

echo
echo "=== converged case (|delta|=0.005 <= tolerance=0.01) ==="
cat >"$TMPDIR/conv-query.json" <<EOF
{
  "@id": "urn:eigenius:test:demo:query-1",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.005
}
EOF
"$KERNEL_BIN" --endpoint "$ENDPOINT" \
  capability test "$INSTITUTION_IRI" --input "$TMPDIR/conv-query.json"

echo
echo "=== not-converged case (|delta|=0.2 > tolerance=0.01) ==="
cat >"$TMPDIR/conv-query-no.json" <<EOF
{
  "@id": "urn:eigenius:test:demo:query-2",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.2
}
EOF
"$KERNEL_BIN" --endpoint "$ENDPOINT" \
  capability test "$INSTITUTION_IRI" --input "$TMPDIR/conv-query-no.json"

echo
echo "=== capability inspect ==="
"$KERNEL_BIN" --endpoint "$ENDPOINT" capability inspect "$INSTITUTION_IRI"

# ---------------------------------------------------------------------------
# EigenQL FIBER clause — dispatch to this institution from a query (#10).
#
# Load a handful of Refinement instances plus the Property definitions the
# type checker needs to resolve short names (tolerance, latest_delta, delta),
# then run a query that asks the institution per-refinement and projects
# the ones it says have converged.
# ---------------------------------------------------------------------------
echo
echo "=== FIBER in EigenQL: load refinements + supporting Property defs ==="
cat >"$TMPDIR/demo-data.json" <<'EOF'
[
  {
    "@id": "urn:eigenius:test:wasm:Refinement",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:short_name": "Refinement",
    "urn:eigenius:core:description": "A refinement morphism between two results.",
    "urn:eigenius:core:requires": [
      "urn:eigenius:test:wasm:source",
      "urn:eigenius:test:wasm:target",
      "urn:eigenius:test:wasm:delta"
    ]
  },
  {
    "@id": "urn:eigenius:test:wasm:ConvergenceQuery",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:short_name": "ConvergenceQuery",
    "urn:eigenius:core:description": "Has the latest refinement step converged below tolerance?",
    "urn:eigenius:core:requires": [
      "urn:eigenius:test:wasm:tolerance",
      "urn:eigenius:test:wasm:latest_delta"
    ]
  },
  {
    "@id": "urn:eigenius:test:wasm:tolerance",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "tolerance",
    "urn:eigenius:core:description": "Convergence tolerance threshold.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:float"
  },
  {
    "@id": "urn:eigenius:test:wasm:latest_delta",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "latest_delta",
    "urn:eigenius:core:description": "Most recently measured refinement step size.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:float"
  },
  {
    "@id": "urn:eigenius:test:wasm:delta",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "delta",
    "urn:eigenius:core:description": "Refinement step size on a Refinement morphism.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:float"
  },
  {
    "@id": "urn:eigenius:test:wasm:source",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "source",
    "urn:eigenius:core:description": "Source result of a Refinement.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:resource"
  },
  {
    "@id": "urn:eigenius:test:wasm:target",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Property"],
    "urn:eigenius:core:short_name": "target",
    "urn:eigenius:core:description": "Target result of a Refinement.",
    "urn:eigenius:core:data_type": "urn:eigenius:core:resource"
  },
  {
    "@id": "urn:demo:refinement:converged",
    "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:Refinement"],
    "urn:eigenius:test:wasm:source": "urn:demo:result:x",
    "urn:eigenius:test:wasm:target": "urn:demo:result:y",
    "urn:eigenius:test:wasm:delta": 0.005
  },
  {
    "@id": "urn:demo:refinement:far",
    "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:Refinement"],
    "urn:eigenius:test:wasm:source": "urn:demo:result:y",
    "urn:eigenius:test:wasm:target": "urn:demo:result:z",
    "urn:eigenius:test:wasm:delta": 0.5
  }
]
EOF
"$KERNEL_BIN" --endpoint "$ENDPOINT" load "$TMPDIR/demo-data.json"

echo
echo "=== EigenQL query with FIBER clause ==="
echo "Asking the institution 'converged?' for each refinement,"
echo "keeping only those it reports as converged (tolerance=0.01):"
echo
"$KERNEL_BIN" --endpoint "$ENDPOINT" query '
  USING INSTITUTION "urn:eigenius:test:wasm:ordering" AS ord
  USING "urn:eigenius:test:wasm:Refinement"
  MATCH Refinement(?m) { delta: ?d }
  FIBER ord:ConvergenceQuery { tolerance: 0.01, latest_delta: ?d } AS ?conv
  MATCH ?conv { "urn:eigenius:test:wasm:converged": ?c }
  WHERE ?c = true
  RETURN [] { refinement: ?m, delta: ?d }
'

echo
echo "=== OK ==="
