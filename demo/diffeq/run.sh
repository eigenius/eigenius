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

# Eigenius DiffEq Institution — End-to-End Demo
#
# Walks the full developer flow for the DiffEq institution against
# the local docker-compose stack. The worked example is exponential
# decay (`du/dt = -k*u, u(0) = 1`) with `k = 1`, integrated to
# `t = 1`. Closed-form solution: `u(1) = e^-1 ≈ 0.3679`.
#
# The RHS is authored as a `formulas:FormulaTerm` value — a typed
# expression tree on the chain — rather than as a Julia source
# string. That's the structural correctness fix landed in Phase
# 19g.5 / D32 §6: every numerical institution that consumes a
# formula speaks FormulaTerm, so cross-institution comorphisms
# (Symbolics → DiffEq, Catalyst → DiffEq, DiffEq →
# IntervalArithmetic) compose cleanly. The kinase reaction-network
# example you'd expect here is the *output* of the Catalyst →
# DiffEq comorphism — see `demo/catalyst-to-diffeq/run.sh` for that
# end-to-end story; this demo focuses on the institution itself.
#
# Steps:
#
#   1. Health-check kernel + orchestrator.
#   2. Load the DiffEq ontology.
#   3. Generate the Julia mirror covering OdeProblem, OdeSolution,
#      RhsComponent — closure walking pulls in FormulaTerm and the
#      operator catalog automatically.
#   4. Build the env image with EigeniusDiffEq + OrdinaryDiffEq.
#   5. Commit the RuntimeEnvironment.
#   6. Install the institution.
#   7. Commit the OdeProblem (exponential decay; RHS as FormulaTerm).
#   8. Commit a Holds-case OdeSolution (final_state = e^-1).
#   9. Try to commit a Fails-case OdeSolution (final_state = 0.5)
#      — the gate refutes; the load is rejected.
#  10. Query the resulting Verdicts.
#
# Prerequisites:
#   docker compose up      (or: EIGENIUS_MOCK_LLM=true docker compose up)
#   docker daemon reachable on the host
#
# Usage:
#   ./demo/diffeq/run.sh
#   ./demo/diffeq/run.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTITUTION_DIR="$REPO_DIR/julia/institutions/diffeq"
ONTOLOGY_FILE="$INSTITUTION_DIR/declarations/diffeq-ontology.eigon.json"
INSTITUTION_FILE="$INSTITUTION_DIR/declarations/diffeq-institution.eigon.json"
HANDLER_PKG_DIR="$INSTITUTION_DIR/EigeniusDiffEq"

ENV_IRI="urn:eigenius:diffeq:env:v1"

echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius"

echo "=== Eigenius DiffEq Institution Demo ==="
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

# Step 1: Load the DiffEq ontology.
echo "--- Step 1: Load DiffEq ontology ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$ONTOLOGY_FILE"
echo

# Step 2: Resolve head layer.
echo "--- Step 2: Resolve head layer ---"
HEAD_HEX=$($EIGENIUS --endpoint "$ENDPOINT" branch show main | awk '{print $2}')
LAYER_IRI="urn:eigenius:layer:$HEAD_HEX"
echo "Head layer: $LAYER_IRI"
echo

# Step 3: Generate + commit the Julia mirror. Seed three classes
# (OdeProblem, OdeSolution, RhsComponent); closure walking pulls in
# FormulaTerm + the operator catalog transitively.
echo "--- Step 3: Generate + commit mirror ---"
MIRROR_OUTPUT_DIR=$(mktemp -d -t eigenius-diffeq-mirror-XXXXXX)
trap 'rm -rf "$MIRROR_OUTPUT_DIR"' EXIT

$EIGENIUS --endpoint "$ENDPOINT" mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["OdeProblem", "OdeSolution", "RhsComponent"]
              RETURN [] { iri: ?iri }' \
    --language julia \
    --output "$MIRROR_OUTPUT_DIR" \
    --json | tee /tmp/eigenius-diffeq-mirror.json
MIRROR_IRI=$(jq -r '.mirror_iri' < /tmp/eigenius-diffeq-mirror.json)
echo "Mirror IRI: $MIRROR_IRI"
echo

# Step 4: Build env image. Cold runs ~5 minutes.
echo "--- Step 4: Build env image ---"
$EIGENIUS --endpoint "$ENDPOINT" env build \
    --language julia \
    --package-path "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json | tee /tmp/eigenius-diffeq-envbuild.json
IMAGE_DIGEST=$(jq -r '.image_digest' < /tmp/eigenius-diffeq-envbuild.json)
RUNTIME_VERSION=$(jq -r '.runtime_version' < /tmp/eigenius-diffeq-envbuild.json)
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

# Step 6: Install the DiffEq institution declaration.
echo "--- Step 6: Install institution ---"
$EIGENIUS --endpoint "$ENDPOINT" institution install --definition "$INSTITUTION_FILE"
echo

# Step 7: Commit the exponential-decay OdeProblem.
#
# RHS is `du/dt = -k*u`. As a FormulaTerm:
#   neg(mul(k, u))
#   = App(OpRef("neg"), App(App(OpRef("mul"), Var("k")), Var("u")))
#
# Authoring this by hand is a one-component case. For richer ODEs
# (e.g. the kinase mechanism) the natural authoring path is via
# the Catalyst → DiffEq comorphism (`demo/catalyst-to-diffeq/`),
# which produces FormulaTerm RHS components from a chain-committed
# ReactionNetwork.
echo "--- Step 7: Commit OdeProblem (exponential decay; RHS as FormulaTerm) ---"
PROBLEM_FILE="$(mktemp -t eigenius-diffeq-problem-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$PROBLEM_FILE"' EXIT
cat >"$PROBLEM_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:diffeq:problem:exp_decay",
    "urn:eigenius:core:is_a": ["urn:eigenius:diffeq:OdeProblem"],
    "urn:eigenius:core:short_name": "exp_decay",
    "urn:eigenius:diffeq:state_names": ["u"],
    "urn:eigenius:diffeq:parameter_names": ["k"],
    "urn:eigenius:diffeq:initial_conditions": [1.0],
    "urn:eigenius:diffeq:parameters": [1.0],
    "urn:eigenius:diffeq:time_span_start": 0.0,
    "urn:eigenius:diffeq:time_span_end": 1.0,
    "urn:eigenius:diffeq:rhs": [
      {
        "urn:eigenius:core:is_a": ["urn:eigenius:diffeq:RhsComponent"],
        "urn:eigenius:diffeq:term": {
          "ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:neg"]},
            {
              "ctor": "App",
              "args": [
                {
                  "ctor": "App",
                  "args": [
                    {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
                    {"ctor": "Var", "args": ["k"]}
                  ]
                },
                {"ctor": "Var", "args": ["u"]}
              ]
            }
          ]
        }
      }
    ]
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$PROBLEM_FILE"
echo

# Step 8: Commit the Holds-case OdeSolution. Closed-form at t=1
# with k=1, u(0)=1 is e^-1 ≈ 0.36787944117144233.
echo "--- Step 8: Commit Holds-case OdeSolution (final_state ≈ e^-1) ---"
HOLDS_FILE="$(mktemp -t eigenius-diffeq-holds-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$PROBLEM_FILE" "$HOLDS_FILE"' EXIT
cat >"$HOLDS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:diffeq:solution:exp_decay_at_1",
    "urn:eigenius:core:is_a": ["urn:eigenius:diffeq:OdeSolution"],
    "urn:eigenius:core:short_name": "exp_decay_at_1",
    "urn:eigenius:diffeq:problem": "urn:eigenius:demo:diffeq:problem:exp_decay",
    "urn:eigenius:diffeq:algorithm": "Tsit5",
    "urn:eigenius:diffeq:abstol": 1e-8,
    "urn:eigenius:diffeq:reltol": 1e-8,
    "urn:eigenius:diffeq:final_state": [0.36787944117144233]
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$HOLDS_FILE"
echo

# Step 9: Try to commit a Fails-case OdeSolution (final_state = 0.5).
# Wildly off from the actual e^-1 ≈ 0.368; the per-component
# tolerance check refutes; the kernel rejects the load.
echo "--- Step 9: Try to commit Fails-case OdeSolution (final_state = 0.5; expect rejection) ---"
FAILS_FILE="$(mktemp -t eigenius-diffeq-fails-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$PROBLEM_FILE" "$HOLDS_FILE" "$FAILS_FILE"' EXIT
cat >"$FAILS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:diffeq:solution:exp_decay_wrong",
    "urn:eigenius:core:is_a": ["urn:eigenius:diffeq:OdeSolution"],
    "urn:eigenius:core:short_name": "exp_decay_wrong",
    "urn:eigenius:diffeq:problem": "urn:eigenius:demo:diffeq:problem:exp_decay",
    "urn:eigenius:diffeq:algorithm": "Tsit5",
    "urn:eigenius:diffeq:abstol": 1e-8,
    "urn:eigenius:diffeq:reltol": 1e-8,
    "urn:eigenius:diffeq:final_state": [0.5]
  }
]
EOF
if $EIGENIUS --endpoint "$ENDPOINT" load "$FAILS_FILE"; then
    echo "WARNING: Fails-case load unexpectedly succeeded — Verdict shape may be off."
else
    echo "Load rejected as expected (Fails verdict from validate_solution)."
fi
echo

# Step 10: Inspect Verdicts.
echo "--- Step 10: Query Verdicts ---"
$EIGENIUS --endpoint "$ENDPOINT" query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:core:ctor_name": ?ctor } RETURN [] { verdict: ?v, ctor: ?ctor }'
echo

echo "=== Demo complete ==="
