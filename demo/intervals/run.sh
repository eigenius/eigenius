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

# Eigenius IntervalArithmetic Institution — End-to-End Demo
#
# Walks the full developer flow for an external Julia institution
# against the local docker-compose stack:
#
#   1. Health-check kernel + orchestrator.
#   2. Load the intervals ontology classes (BoundedBy + props).
#   3. Generate the Julia mirror against the kernel's head layer and
#      commit the RuntimePackageMirror Resource.
#   4. Build the env image (host-side via `eigenius env build`, which
#      drives buildah + the substrate engine — prints sha256:digest).
#   5. Commit the RuntimeEnvironment Resource carrying that digest.
#   6. Install the Institution declaration (Institution + QueryClass +
#      RuntimeMethodSignature).
#   7. Commit a BoundedBy instance — kernel's AutoOnLoad gate fires,
#      orchestrator dispatches to the Julia worker, Verdict commits.
#   8. Query the resulting Verdict + RuntimeInvocation provenance.
#
# Prerequisites:
#   docker compose up      (or: EIGENIUS_MOCK_LLM=true docker compose up)
#   docker daemon reachable on the host
#
# Usage:
#   ./demo/intervals/run.sh
#   ./demo/intervals/run.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTITUTION_DIR="$REPO_DIR/julia/institutions/intervals"
ONTOLOGY_FILE="$INSTITUTION_DIR/declarations/intervals-ontology.eigon.json"
INSTITUTION_FILE="$INSTITUTION_DIR/declarations/intervals-institution.eigon.json"
HANDLER_PKG_DIR="$INSTITUTION_DIR/EigeniusIntervals"

ENV_IRI="urn:eigenius:intervals:env:v1"
BOUNDED_BY_CLASS="urn:eigenius:intervals:BoundedBy"

# Always build the workspace CLI rather than picking up a possibly-stale
# `eigenius` from $PATH — the demo exercises lifecycle commands (mirror,
# env, institution) that lag the published binary.
echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius"

echo "=== Eigenius IntervalArithmetic Institution Demo ==="
echo "Kernel:        $ENDPOINT"
echo "Orchestrator:  $ORCHESTRATOR"
echo "Institution:   $INSTITUTION_DIR"
echo

# Step 0: Health check
echo "--- Step 0: Health check ---"
if ! curl -sf "$ORCHESTRATOR/health" >/dev/null; then
    echo "ERROR: Orchestrator not reachable at $ORCHESTRATOR/health"
    echo "Start the stack first: docker compose up"
    exit 1
fi
$EIGENIUS --endpoint "$ENDPOINT" inspect "urn:eigenius:core:Class" >/dev/null
echo "Stack healthy."
echo

# Step 1: Load the intervals ontology classes.
echo "--- Step 1: Load intervals ontology ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$ONTOLOGY_FILE"
echo

# Step 2: Resolve the kernel's current head layer — the mirror
# generator anchors against this layer so closure-walked classes are
# read consistently.
echo "--- Step 2: Resolve head layer ---"
HEAD_HEX=$($EIGENIUS --endpoint "$ENDPOINT" branch show main | awk '{print $2}')
LAYER_IRI="urn:eigenius:layer:$HEAD_HEX"
echo "Head layer: $LAYER_IRI"
echo

# Step 3: Generate the Julia mirror for BoundedBy and commit it.
# Filter selects exactly the BoundedBy class — the generator's
# closure walk pulls in its property classes automatically.
echo "--- Step 3: Generate + commit mirror ---"
MIRROR_OUTPUT_DIR=$(mktemp -d -t eigenius-intervals-mirror-XXXXXX)
trap 'rm -rf "$MIRROR_OUTPUT_DIR"' EXIT

$EIGENIUS --endpoint "$ENDPOINT" mirror create \
    --layer "$LAYER_IRI" \
    --filter "MATCH \"urn:eigenius:core:Class\"(?iri) { \"urn:eigenius:core:short_name\": \"BoundedBy\" } RETURN [] { iri: ?iri }" \
    --language julia \
    --output "$MIRROR_OUTPUT_DIR" \
    --json | tee /tmp/eigenius-intervals-mirror.json
MIRROR_IRI=$(jq -r '.mirror_iri' < /tmp/eigenius-intervals-mirror.json)
echo "Mirror IRI: $MIRROR_IRI"
echo

# Step 4: Build the env image. Slow on cold runs — pulls the Julia
# base image, runs `Pkg.instantiate` for handler + EigeniusJuliaCommon
# + the generated mirror.
echo "--- Step 4: Build env image ---"
$EIGENIUS --endpoint "$ENDPOINT" env build \
    --language julia \
    --package-path "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json | tee /tmp/eigenius-intervals-envbuild.json
IMAGE_DIGEST=$(jq -r '.image_digest' < /tmp/eigenius-intervals-envbuild.json)
RUNTIME_VERSION=$(jq -r '.runtime_version' < /tmp/eigenius-intervals-envbuild.json)
echo "Image digest:    $IMAGE_DIGEST"
echo "Runtime version: $RUNTIME_VERSION"
echo

# Step 5: Commit the RuntimeEnvironment Resource referencing the digest.
echo "--- Step 5: Create env Resource ---"
$EIGENIUS --endpoint "$ENDPOINT" env create \
    --language julia \
    --handler-package "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --as-iri "$ENV_IRI" \
    --image-digest "$IMAGE_DIGEST" \
    --runtime-version "$RUNTIME_VERSION"
echo

# Step 6: Install the institution declaration. The kernel's commit
# pipeline indexes the AutoOnLoad QueryClass so future BoundedBy
# loads dispatch automatically.
echo "--- Step 6: Install institution ---"
$EIGENIUS --endpoint "$ENDPOINT" institution install --definition "$INSTITUTION_FILE"
echo

# Step 7: Commit a BoundedBy instance. The kernel's AutoOnLoad gate
# fires DispatchExternal → orchestrator → substrate → Julia worker →
# Verdict committed back to the chain. `2 ∈ [1, 3]` should yield Holds.
echo "--- Step 7: Commit BoundedBy instance (expect Holds) ---"
INSTANCE_FILE="$(mktemp -t eigenius-intervals-instance-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE"' EXIT
cat >"$INSTANCE_FILE" <<EOF
[
  {
    "@id": "urn:eigenius:demo:intervals:obs:1",
    "urn:eigenius:core:is_a": ["$BOUNDED_BY_CLASS"],
    "urn:eigenius:core:short_name": "obs1",
    "urn:eigenius:intervals:value": 2.0,
    "urn:eigenius:intervals:lower": 1.0,
    "urn:eigenius:intervals:upper": 3.0
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$INSTANCE_FILE"
echo

# Step 8: Inspect the Verdict that just landed. The institution's
# AutoOnLoad gate produces one Verdict per BoundedBy commit; query
# them so the user can see Holds + the RuntimeInvocation provenance.
echo "--- Step 8: Query Verdicts ---"
$EIGENIUS --endpoint "$ENDPOINT" query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:core:ctor_name": ?ctor } RETURN [] { verdict: ?v, ctor: ?ctor }'
echo

echo "=== Demo complete ==="
