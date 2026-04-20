#!/usr/bin/env bash
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

echo
echo "=== OK ==="
