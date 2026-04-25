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

# WASM Extensibility Demo
#
# Exercises the Phase 8 WASM hosting path end-to-end:
#   1. Installs a pure WASM component (doc validator) into the kernel
#   2. Installs an IO WASM component (http shout) into the orchestrator
#   3. Installs a WASM institution (ordering) into the kernel
#   4. Invokes each via `capability test` and shows typed results
#
# Prerequisites:
#   - The kernel and orchestrator are running (Docker Compose or three terminals)
#   - The WASM binaries are built (see BUILD_WASM below if missing)
#
# Usage:
#   ./demo/wasm/run.sh                          # against Docker Compose stack
#   ./demo/wasm/run.sh http://localhost:50051   # custom kernel endpoint

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

if command -v eigenius &>/dev/null; then
  EIGENIUS="eigenius"
else
  EIGENIUS="cargo run -q -p eigenius-cli --"
fi

DOC_VALIDATOR_WASM="$REPO_DIR/examples/wasm-doc-validator/target/wasm32-unknown-unknown/debug/eigenius_wasm_doc_validator.wasm"
HTTP_SHOUT_WASM="$REPO_DIR/examples/wasm-http-shout/target/wasm32-unknown-unknown/debug/eigenius_wasm_http_shout.wasm"
ORDERING_WASM="$REPO_DIR/examples/wasm-ordering-institution/target/wasm32-unknown-unknown/debug/eigenius_wasm_ordering_institution.wasm"

build_if_missing() {
  local wasm_path="$1"
  local example_dir="$2"
  if [ ! -f "$wasm_path" ]; then
    echo "Building $example_dir (not found at $wasm_path)..."
    (cd "$example_dir" && cargo component build)
  fi
}

echo "=== WASM Extensibility Demo ==="
echo "Kernel:       $ENDPOINT"
echo "Orchestrator: $ORCHESTRATOR"
echo

# Pre-flight: make sure all three WASM binaries exist
build_if_missing "$DOC_VALIDATOR_WASM" "$REPO_DIR/examples/wasm-doc-validator"
build_if_missing "$HTTP_SHOUT_WASM" "$REPO_DIR/examples/wasm-http-shout"
build_if_missing "$ORDERING_WASM" "$REPO_DIR/examples/wasm-ordering-institution"
echo

# Health check the orchestrator
echo "--- Health check ---"
if curl -sf "$ORCHESTRATOR/health" >/dev/null; then
  echo "Orchestrator is healthy."
else
  echo "WARNING: Orchestrator not reachable at $ORCHESTRATOR/health"
  echo "  (IO WASM install will fail without orchestrator)"
fi
echo

# =============================================================================
# Pure WASM component: doc validator (hosted in the kernel)
# =============================================================================

echo "--- Step 1: Install pure WASM component (doc validator, kernel-hosted) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability install \
  "$DOC_VALIDATOR_WASM" \
  --as-iri urn:example:components:DocValidator \
  --kind component \
  --capability pure \
  --input-type urn:example:doc:Document \
  --output-type urn:example:doc:ValidationResult
echo

cat > /tmp/doc-valid.json <<'EOF'
{
  "@id": "urn:example:doc:valid1",
  "urn:example:doc:title": "Hello World",
  "urn:example:doc:body": "This document has enough body text to pass the minimum 100-character requirement that the validator enforces. Padding to reach the threshold.",
  "urn:example:doc:section_count": 3
}
EOF

echo "--- Step 2: Test doc validator (valid input) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability test \
  urn:example:components:DocValidator --input /tmp/doc-valid.json
echo

cat > /tmp/doc-invalid.json <<'EOF'
{
  "@id": "urn:example:doc:bad",
  "urn:example:doc:title": "",
  "urn:example:doc:body": "too short",
  "urn:example:doc:section_count": 0
}
EOF

echo "--- Step 3: Test doc validator (multiple validation failures) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability test \
  urn:example:components:DocValidator --input /tmp/doc-invalid.json
echo

# =============================================================================
# IO WASM component: http shout (hosted in the orchestrator, dispatches to
# CompleteText via the io-access host import)
# =============================================================================

echo "--- Step 4: Install IO WASM component (http shout, orchestrator-hosted) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability install \
  "$HTTP_SHOUT_WASM" \
  --as-iri urn:example:components:HttpShout \
  --kind component \
  --capability io \
  --input-type urn:example:shout:TextInput \
  --output-type urn:example:shout:ShoutedText
echo

cat > /tmp/shout-input.json <<'EOF'
{
  "@id": "urn:example:shout:input1",
  "urn:example:shout:text": "hello from a wasm component"
}
EOF

echo "--- Step 5: Test http shout (WASM → dispatch-component → CompleteText → LLM) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability test \
  urn:example:components:HttpShout --input /tmp/shout-input.json
echo

# =============================================================================
# WASM institution: ordering (hosted in the kernel)
# =============================================================================

echo "--- Step 6: Install WASM institution (ordering, kernel-hosted) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability install \
  "$ORDERING_WASM" \
  --as-iri urn:eigenius:test:wasm:ordering \
  --kind institution
echo

cat > /tmp/conv-query.json <<'EOF'
{
  "@id": "urn:example:query:conv1",
  "urn:eigenius:core:is_a": ["urn:eigenius:test:wasm:ConvergenceQuery"],
  "urn:eigenius:test:wasm:tolerance": 0.01,
  "urn:eigenius:test:wasm:latest_delta": 0.001
}
EOF

echo "--- Step 7: Test convergence query (parameterized fiber query) ---"
$EIGENIUS --endpoint "$ENDPOINT" capability test \
  urn:eigenius:test:wasm:ordering --input /tmp/conv-query.json
echo

# =============================================================================
# Summary
# =============================================================================

echo "--- Step 8: List all registered capabilities ---"
$EIGENIUS --endpoint "$ENDPOINT" capability list
echo

echo "=== Demo complete ==="
echo "What was demonstrated:"
echo "  - Pure WASM component hosted in the kernel (doc validator)"
echo "  - IO WASM component hosted in the orchestrator (http shout)"
echo "    dispatching to CompleteText via the io-access host import"
echo "  - WASM institution with parameterized fiber queries (ordering)"
