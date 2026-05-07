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

# Eigenius Symbolics Institution — End-to-End Demo
#
# Walks the full developer flow for the Symbolics institution against
# the local docker-compose stack:
#
#   1. Health-check kernel + orchestrator.
#   2. Load the Symbolics ontology (SymbolicExpression + SimplifiesTo).
#   3. Generate the Julia mirror (covers SymbolicExpression, SimplifiesTo,
#      and FormulaTerm via closure walk).
#   4. Build the env image with the EigeniusSymbolics handler package.
#   5. Commit the RuntimeEnvironment.
#   6. Install the institution declaration.
#   7. Commit a SimplifiesTo claim — kernel's AutoOnLoad fires
#      `validate_simplifies_to`, which re-runs Symbolics.simplify
#      and produces a Verdict.
#   8. Query the resulting Verdicts.
#
# Prerequisites:
#   docker compose up      (or: EIGENIUS_MOCK_LLM=true docker compose up)
#   docker daemon reachable on the host
#
# Usage:
#   ./demo/symbolics/run.sh
#   ./demo/symbolics/run.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTITUTION_DIR="$REPO_DIR/julia/institutions/symbolics"
ONTOLOGY_FILE="$INSTITUTION_DIR/declarations/symbolics-ontology.eigon.json"
INSTITUTION_FILE="$INSTITUTION_DIR/declarations/symbolics-institution.eigon.json"
HANDLER_PKG_DIR="$INSTITUTION_DIR/EigeniusSymbolics"

ENV_IRI="urn:eigenius:symbolics:env:v1"

# Always build the workspace CLI rather than picking up a possibly-stale
# `eigenius` from $PATH — the demo exercises lifecycle commands (mirror,
# env, institution) that lag the published binary.
echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius"

echo "=== Eigenius Symbolics Institution Demo ==="
echo "Kernel:        $ENDPOINT"
echo "Orchestrator:  $ORCHESTRATOR"
echo "Institution:   $INSTITUTION_DIR"
echo

# Step 0: Health check.
echo "--- Step 0: Health check ---"
if ! curl -sf "$ORCHESTRATOR/health" >/dev/null; then
    echo "ERROR: Orchestrator not reachable at $ORCHESTRATOR/health"
    echo "Start the stack first: docker compose up"
    exit 1
fi
$EIGENIUS --endpoint "$ENDPOINT" inspect "urn:eigenius:core:Class" >/dev/null
echo "Stack healthy."
echo

# Step 1: Load the Symbolics ontology classes.
echo "--- Step 1: Load Symbolics ontology ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$ONTOLOGY_FILE"
echo

# Step 2: Resolve the kernel's current head layer for the mirror anchor.
echo "--- Step 2: Resolve head layer ---"
HEAD_HEX=$($EIGENIUS --endpoint "$ENDPOINT" branch show main | awk '{print $2}')
LAYER_IRI="urn:eigenius:layer:$HEAD_HEX"
echo "Head layer: $LAYER_IRI"
echo

# Step 3: Generate the Julia mirror covering SymbolicExpression +
# SimplifiesTo. The closure walker pulls FormulaTerm in via the
# `term` property's class_types reference (D32 §3.5).
echo "--- Step 3: Generate + commit mirror ---"
MIRROR_OUTPUT_DIR=$(mktemp -d -t eigenius-symbolics-mirror-XXXXXX)
trap 'rm -rf "$MIRROR_OUTPUT_DIR"' EXIT

$EIGENIUS --endpoint "$ENDPOINT" mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) { "urn:eigenius:core:short_name": ?name } WHERE ?name IN ["SimplifiesTo", "SimplifyRequest", "EquivalenceCheck"] RETURN [] { iri: ?iri }' \
    --language julia \
    --output "$MIRROR_OUTPUT_DIR" \
    --json | tee /tmp/eigenius-symbolics-mirror.json
MIRROR_IRI=$(jq -r '.mirror_iri' < /tmp/eigenius-symbolics-mirror.json)
echo "Mirror IRI: $MIRROR_IRI"
echo

# Step 4: Build the env image with the EigeniusSymbolics handler baked
# in. Cold runs pull Julia + Pkg.precompile Symbolics — a few minutes
# the first time; cached from then on.
echo "--- Step 4: Build env image ---"
$EIGENIUS --endpoint "$ENDPOINT" env build \
    --language julia \
    --package-path "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json | tee /tmp/eigenius-symbolics-envbuild.json
IMAGE_DIGEST=$(jq -r '.image_digest' < /tmp/eigenius-symbolics-envbuild.json)
RUNTIME_VERSION=$(jq -r '.runtime_version' < /tmp/eigenius-symbolics-envbuild.json)
echo "Image digest:    $IMAGE_DIGEST"
echo "Runtime version: $RUNTIME_VERSION"
echo

# Step 5: Commit the RuntimeEnvironment Resource.
echo "--- Step 5: Create env Resource ---"
$EIGENIUS --endpoint "$ENDPOINT" env create \
    --language julia \
    --handler-package "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --as-iri "$ENV_IRI" \
    --image-digest "$IMAGE_DIGEST" \
    --runtime-version "$RUNTIME_VERSION"
echo

# Step 6: Install the Symbolics institution declaration.
echo "--- Step 6: Install institution ---"
$EIGENIUS --endpoint "$ENDPOINT" institution install --definition "$INSTITUTION_FILE"
echo

# Step 7: Commit a SimplifiesTo claim. We claim `x * 0` simplifies to
# `0` — a textbook Symbolics simplification. Kernel's AutoOnLoad
# fires `validate_simplifies_to`, which re-runs `Symbolics.simplify`
# and confirms the result.
echo "--- Step 7: Commit SimplifiesTo claim (x * 0 == 0; expect Holds) ---"
INSTANCE_FILE="$(mktemp -t eigenius-symbolics-instance-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE"' EXIT
cat >"$INSTANCE_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:symbolics:claim:x_times_0_simplifies_to_zero",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SimplifiesTo"],
    "urn:eigenius:core:short_name": "x_times_0_simplifies_to_zero",
    "urn:eigenius:symbolics:expr": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "x_times_0",
      "urn:eigenius:symbolics:term": {
        "ctor": "App",
        "args": [
          {
            "ctor": "App",
            "args": [
              {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
              {"ctor": "Var", "args": ["x"]}
            ]
          },
          {"ctor": "LitFloat", "args": [0.0]}
        ]
      }
    },
    "urn:eigenius:symbolics:simplified": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "zero",
      "urn:eigenius:symbolics:term": {
        "ctor": "LitFloat",
        "args": [0.0]
      }
    }
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$INSTANCE_FILE"
echo

# Step 8: Inspect the Verdict that just landed.
echo "--- Step 8: Query Verdicts ---"
$EIGENIUS --endpoint "$ENDPOINT" query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:core:ctor_name": ?ctor } RETURN [] { verdict: ?v, ctor: ?ctor }'
echo

# Step 9: OnDemand FIBER dispatch — `qc_symb_simplify`. Pre-commit a
# SymbolicExpression `(x + 0) * 1`, then ask the institution to
# simplify it explicitly via FIBER. The kernel's IRI-dereference pass
# embeds the chain-committed expr into the FIBER-synthesized
# SimplifyRequest input; the worker decodes, runs Symbolics.simplify,
# and re-encodes the result as a FormulaTerm-wrapped
# SymbolicExpression.
echo "--- Step 9: Commit input expression for FIBER simplify ---"
INPUT_EXPR_FILE="$(mktemp -t eigenius-symbolics-input-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE" "$INPUT_EXPR_FILE"' EXIT
cat >"$INPUT_EXPR_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:symbolics:expr:x_plus_0_times_1",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
    "urn:eigenius:core:short_name": "x_plus_0_times_1",
    "urn:eigenius:symbolics:term": {
      "ctor": "App",
      "args": [
        {
          "ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
            {
              "ctor": "App",
              "args": [
                {
                  "ctor": "App",
                  "args": [
                    {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                    {"ctor": "Var", "args": ["x"]}
                  ]
                },
                {"ctor": "LitFloat", "args": [0.0]}
              ]
            }
          ]
        },
        {"ctor": "LitFloat", "args": [1.0]}
      ]
    }
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$INPUT_EXPR_FILE"
echo

echo "--- Step 10: FIBER qc_symb_simplify (OnDemand) ---"
# The textual FIBER syntax: pass the chain-committed expr by IRI; the
# kernel's IRI-dereference pass embeds it into the SimplifyRequest the
# institution sees. Then project the simplified result's `term` (a
# FormulaTerm tree) and `short_name` for inspection.
$EIGENIUS --endpoint "$ENDPOINT" query \
    'USING INSTITUTION "urn:eigenius:institutions:symbolics" AS cap
     FIBER cap:qc_symb_simplify {
       expr: "urn:eigenius:demo:symbolics:expr:x_plus_0_times_1"
     } AS ?simplified
     RETURN [] { result: ?simplified, term: ?simplified.term }'
echo

echo "=== Demo complete ==="
