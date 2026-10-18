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

# D41 commit-pipeline e2e: exercises CommitPolicy + --explicit-tombstone
# end-to-end through the CLI against the Docker stack.
#
# Scenarios:
#   1. Default Reject policy accepts a valid layer.
#   2. Default Reject rejects a layer with an undefined IRI reference.
#   3. --commit-policy cascade tombstones lower-layer resources that the
#      new layer's class redefinition retroactively invalidates.
#   4. --explicit-tombstone suppresses an IRI in a follow-up commit;
#      `inspect` afterwards reports "not found".
#
# Prerequisites:
#   docker compose up
#
# Usage:
#   ./demo/d41-commit-pipeline/run.sh                         # default endpoints
#   ./demo/d41-commit-pipeline/run.sh http://localhost:50051  # custom kernel
#
# Each run uses a unique IRI namespace (`urn:eigenius:test:d41-<ts>-<pid>:`)
# so reruns against the same long-lived stack don't collide.

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Prefer the workspace `cargo run` so this test always exercises
# bleeding-edge CLI flags (--commit-policy / --explicit-tombstone /
# --max-violations are D41 additions; a stale installed `eigenius`
# binary on $PATH would fail with "unexpected argument"). Fall back
# to the installed binary only when run from outside the workspace.
if [ -f "$SCRIPT_DIR/../../Cargo.toml" ] && command -v cargo &>/dev/null; then
  EIGENIUS=(cargo run -q -p eigenius-cli --)
elif command -v eigenius &>/dev/null; then
  EIGENIUS=(eigenius)
else
  echo "Neither workspace cargo nor installed eigenius binary found." >&2
  exit 1
fi

NS="urn:eigenius:test:d41-$(date +%s)-$$:"
TMP="$(mktemp -d -t d41-XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

ok()   { printf '    \033[32m✓\033[0m %s\n' "$*"; }
fail() { printf '    \033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

# Run the CLI with retries on transient transport errors. gRPC connections
# from short-lived CLI processes to the dockerized kernel occasionally
# drop with "transport error" / "Failed to connect" / "connection refused"
# during handshake — most often when many fresh processes connect in
# rapid succession. We retry up to 3 times with linear backoff; real
# failures (validation rejections, parse errors, "not found", etc.)
# return on the first attempt because their output doesn't match the
# transport-error patterns.
run_cli() {
  local max=3 attempt=1 out rc
  while :; do
    set +e
    out=$("${EIGENIUS[@]}" "$@" 2>&1)
    rc=$?
    set -e
    if [ $rc -eq 0 ] \
       || ! printf '%s' "$out" | grep -qE 'transport error|connection refused|Failed to connect'; then
      printf '%s' "$out"
      return $rc
    fi
    if [ $attempt -ge $max ]; then
      printf '%s' "$out"
      return $rc
    fi
    sleep "$attempt"
    attempt=$((attempt + 1))
  done
}

# Verification helpers — query the chain to confirm post-commit state.
# `inspect` against the chain is the simplest IRI-presence probe; its
# output shape carries either the resource (success) or a "not found"
# marker (success-with-found=false, or non-zero exit with text). Both
# shapes are accepted so this script is forward-compatible with output
# format changes.
inspect_out() {
  # Echo (stdout, exit-code) for the given IRI. Uses `run_cli` so
  # transient gRPC handshake failures don't surface as false negatives.
  local iri="$1"
  set +e
  local out
  out=$(run_cli --endpoint "$ENDPOINT" --json inspect "$iri")
  local rc=$?
  set -e
  printf '%s\n%d\n' "$out" "$rc"
}

assert_resolves() {
  local iri="$1" desc="${2:-$iri}"
  local raw out rc
  raw=$(inspect_out "$iri")
  out=$(printf '%s' "$raw" | head -n -1)
  rc=$(printf '%s' "$raw" | tail -n 1)
  if [ "$rc" -ne 0 ]; then
    fail "expected $desc to resolve; inspect exit=$rc: $out"
  fi
  if printf '%s' "$out" | grep -qiE 'found"?: *false|not[ _]found'; then
    fail "expected $desc to resolve; inspect reports not-found: $out"
  fi
}

assert_not_resolved() {
  local iri="$1" desc="${2:-$iri}"
  local raw out rc
  raw=$(inspect_out "$iri")
  out=$(printf '%s' "$raw" | head -n -1)
  rc=$(printf '%s' "$raw" | tail -n 1)
  # Either non-zero exit OR found-false / not-found marker in output.
  if [ "$rc" -eq 0 ] && ! printf '%s' "$out" | grep -qiE 'found"?: *false|not[ _]found'; then
    fail "expected $desc NOT to resolve; inspect succeeded with: $out"
  fi
}

# Fixtures use `urn:eigenius:test:d41-template:` as a stand-in for the
# per-run namespace so the static files contain only valid URN @id
# values. At runtime we substitute the template namespace for $NS
# (which has the timestamp + pid suffix) so reruns don't collide.
TEMPLATE_NS="urn:eigenius:test:d41-template:"

fixture() {
  local out="$TMP/$1.json"
  sed "s|${TEMPLATE_NS}|${NS}|g" "$SCRIPT_DIR/$1.json" > "$out"
  printf '%s' "$out"
}

echo "=== D41 commit-pipeline e2e ==="
echo "Kernel:       $ENDPOINT"
echo "Orchestrator: $ORCHESTRATOR"
echo "Namespace:    $NS"
echo

# Step 0: orchestrator health check
echo "Step 0: health check"
if ! curl -sf "$ORCHESTRATOR/health" >/dev/null; then
  fail "orchestrator not reachable at $ORCHESTRATOR; docker compose up first"
fi
ok "orchestrator healthy"
echo

# Step 1: default Reject accepts a valid layer.
echo "Step 1: default Reject policy accepts valid layer"
out=$("${EIGENIUS[@]}" --endpoint "$ENDPOINT" --json load "$(fixture base)" 2>&1) \
  || fail "base load (expected ok): $out"
echo "$out" | grep -q '"success":true' || fail "expected success=true: $out"
ok "base layer loaded (Animal + Cat)"
# Verify chain state — both resources should now resolve.
assert_resolves "${NS}Animal" "Animal class"
assert_resolves "${NS}Cat"    "Cat instance"
ok "chain state verified: Animal + Cat resolvable"
echo

# Step 2: default Reject rejects an invalid layer.
echo "Step 2: default Reject policy rejects invalid layer"
set +e
out=$("${EIGENIUS[@]}" --endpoint "$ENDPOINT" --json load "$(fixture invalid)" 2>&1)
rc=$?
set -e
if [ $rc -eq 0 ]; then
  fail "expected non-zero exit for invalid layer; got: $out"
fi
ok "invalid layer rejected (exit=$rc)"
# Verify chain unchanged — Animal should still resolve, BadClass should not.
assert_resolves     "${NS}Animal"   "Animal still resolves (chain unchanged)"
assert_not_resolved "${NS}BadClass" "rejected BadClass not committed"
ok "chain state verified: rejection left chain unchanged"
echo

# Step 3: --commit-policy cascade tombstones lower-layer violators.
# Subcommand flags (--commit-policy, --explicit-tombstone, --max-violations)
# belong to the `load` subcommand and must appear after the subcommand name.
echo "Step 3: --commit-policy cascade tombstones violators"
out=$("${EIGENIUS[@]}" --endpoint "$ENDPOINT" --json load \
        --commit-policy cascade "$(fixture redef)" 2>&1) \
  || fail "cascade load (expected ok): $out"
echo "$out" | grep -q '"success":true' || fail "expected success=true: $out"
# The kernel JSON output shape carries cascade_tombstones as a count.
# We accept either a numeric ">=1" or a non-empty array form for forward
# compatibility with output-shape evolution.
if ! echo "$out" | grep -qE '"cascade_tombstones":[ ]*[1-9]|"cascade_tombstones":[ ]*\[[^]]*"'; then
  fail "expected cascade_tombstones > 0 (Cat should have been tombstoned): $out"
fi
ok "cascade applied tombstones to fixpoint"
# Verify chain state — Animal still resolves (redef landed), Cat does not (cascade-tombstoned).
assert_resolves     "${NS}Animal" "Animal still resolves (now InductiveType)"
assert_not_resolved "${NS}Cat"    "Cat cascade-tombstoned by Animal redefinition"
ok "chain state verified: Animal kept, Cat suppressed"
echo

# Step 4: --explicit-tombstone suppresses an IRI.
echo "Step 4: --explicit-tombstone suppresses an IRI"
out=$("${EIGENIUS[@]}" --endpoint "$ENDPOINT" --json load "$(fixture tomb_target)" 2>&1) \
  || fail "tomb_target load: $out"
echo "$out" | grep -q '"success":true' || fail "expected tomb_target ok: $out"
# Verify Mouse landed before we tombstone it.
assert_resolves "${NS}Mouse" "Mouse class committed"
ok "Mouse landed in tomb_target commit"

out=$("${EIGENIUS[@]}" --endpoint "$ENDPOINT" --json load \
        --explicit-tombstone "${NS}Mouse" \
        "$(fixture tomb_marker)" 2>&1) \
  || fail "tomb_marker load: $out"
echo "$out" | grep -q '"success":true' || fail "expected tomb_marker ok: $out"

# Verify Mouse suppressed AND the unrelated Marker resource (committed
# alongside the explicit tombstone) is reachable.
assert_not_resolved "${NS}Mouse"  "Mouse suppressed by --explicit-tombstone"
assert_resolves     "${NS}Marker" "Marker committed alongside tombstone"
ok "chain state verified: Mouse suppressed, Marker present"
echo

echo "=== D41 e2e: PASS ==="
