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

# Eigenius JuMP-HiGHS Institution — End-to-End Demo
#
# Walks the full developer flow for the JuMP-HiGHS institution against
# the local docker-compose stack. Worked examples:
#
#   LP:  min x + 2y  s.t.  x + y ≤ 10,  0 ≤ x,y ≤ 10
#        Optimum (x,y) = (0,0); objective value 0.
#
#   QP:  min (x-1)² + (y-2)²  s.t.  x + y == 2
#        Optimum (x,y) = (0.5, 1.5); objective value 0.5.
#
# Objective and constraints are authored as `formulas:FormulaTerm`
# values — the same chain-typed expression tree used by Symbolics,
# DiffEq, IntervalArithmetic, and Catalyst (D32 §6). The institution's
# walker translates each FormulaTerm to a JuMP expression at dispatch
# time. The smart-pow rule (integer-valued LitFloat exponent on `pow`
# unrolls to repeated multiplication) is what makes the QP land at
# all — HiGHS rejects ScalarNonlinearFunction objectives, so without
# the unrolling `(x-1)^2.0` would promote to NonlinearExpr and the
# solver would refuse the model.
#
# Steps:
#
#   1. Health-check kernel + orchestrator.
#   2. Load the JuMP ontology.
#   3. Generate the Julia mirror covering OptimisationProblem,
#      OptimisesTo — closure walking pulls in Constraint,
#      VariableBound, ConstraintRelation, FormulaTerm, and the
#      operator catalog automatically.
#   4. Build the env image with EigeniusJuMPHiGHS + JuMP + HiGHS.
#   5. Commit the RuntimeEnvironment.
#   6. Install the institution.
#   7. Commit the LP OptimisationProblem.
#   8. Commit a Holds-case OptimisesTo (objective_value = 0).
#   9. Commit the QP OptimisationProblem (smart-pow path).
#  10. Commit a Holds-case OptimisesTo (objective_value = 0.5).
#  11. Try to commit a Fails-case OptimisesTo (claimed objective
#      = -1) — the gate refutes; the load is rejected.
#  12. Query the resulting Verdicts.
#
# Prerequisites:
#   docker compose up      (or: EIGENIUS_MOCK_LLM=true docker compose up)
#   docker daemon reachable on the host
#
# Usage:
#   ./demo/jump-highs/run.sh
#   ./demo/jump-highs/run.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTITUTION_DIR="$REPO_DIR/julia/institutions/jump"
ONTOLOGY_FILE="$INSTITUTION_DIR/declarations/jump-ontology.eigon.json"
INSTITUTION_FILE="$INSTITUTION_DIR/declarations/jump-highs-institution.eigon.json"
HANDLER_PKG_DIR="$INSTITUTION_DIR/EigeniusJuMPHiGHS"

ENV_IRI="urn:eigenius:jump_highs:env:v1"

echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius"

echo "=== Eigenius JuMP-HiGHS Institution Demo ==="
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

# Step 1: Load the JuMP ontology.
echo "--- Step 1: Load JuMP ontology ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$ONTOLOGY_FILE"
echo

# Step 2: Resolve head layer.
echo "--- Step 2: Resolve head layer ---"
HEAD_HEX=$($EIGENIUS --endpoint "$ENDPOINT" branch show main | awk '{print $2}')
LAYER_IRI="urn:eigenius:layer:$HEAD_HEX"
echo "Head layer: $LAYER_IRI"
echo

# Step 3: Generate + commit the Julia mirror.
echo "--- Step 3: Generate + commit mirror ---"
MIRROR_OUTPUT_DIR=$(mktemp -d -t eigenius-jump-mirror-XXXXXX)
trap 'rm -rf "$MIRROR_OUTPUT_DIR"' EXIT

$EIGENIUS --endpoint "$ENDPOINT" mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["OptimisationProblem", "OptimisesTo"]
              RETURN [] { iri: ?iri }' \
    --language julia \
    --output "$MIRROR_OUTPUT_DIR" \
    --json | tee /tmp/eigenius-jump-mirror.json
MIRROR_IRI=$(jq -r '.mirror_iri' < /tmp/eigenius-jump-mirror.json)
echo "Mirror IRI: $MIRROR_IRI"
echo

# Step 4: Build env image. Cold runs ~2-3 minutes (JuMP + HiGHS).
echo "--- Step 4: Build env image ---"
$EIGENIUS --endpoint "$ENDPOINT" env build \
    --language julia \
    --package-path "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json | tee /tmp/eigenius-jump-envbuild.json
IMAGE_DIGEST=$(jq -r '.image_digest' < /tmp/eigenius-jump-envbuild.json)
RUNTIME_VERSION=$(jq -r '.runtime_version' < /tmp/eigenius-jump-envbuild.json)
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

# Step 6: Install the JuMP-HiGHS institution declaration.
echo "--- Step 6: Install institution ---"
$EIGENIUS --endpoint "$ENDPOINT" institution install --definition "$INSTITUTION_FILE"
echo

# Step 7: Commit the LP OptimisationProblem.
#
# Authoring shape:
#   variable_names: ["x", "y"]
#   variable_bounds: x ∈ [0, 10], y ∈ [0, 10]
#   objective: x + 2*y  (FormulaTerm)
#   sense: "Min"
#   constraints: x + y <= 10
echo "--- Step 7: Commit LP OptimisationProblem ---"
LP_FILE="$(mktemp -t eigenius-jump-lp-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$LP_FILE"' EXIT
cat >"$LP_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:jump:bound:lp:x",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:VariableBound"],
    "urn:eigenius:jump:variable_name": "x",
    "urn:eigenius:jump:lower": 0.0,
    "urn:eigenius:jump:upper": 10.0
  },
  {
    "@id": "urn:eigenius:demo:jump:bound:lp:y",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:VariableBound"],
    "urn:eigenius:jump:variable_name": "y",
    "urn:eigenius:jump:lower": 0.0,
    "urn:eigenius:jump:upper": 10.0
  },
  {
    "@id": "urn:eigenius:demo:jump:cstr:lp:sum_le",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:Constraint"],
    "urn:eigenius:jump:lhs": {
      "ctor": "App",
      "args": [
        {
          "ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
            {"ctor": "Var", "args": ["x"]}
          ]
        },
        {"ctor": "Var", "args": ["y"]}
      ]
    },
    "urn:eigenius:jump:relation": {"ctor": "LE"},
    "urn:eigenius:jump:rhs": 10.0
  },
  {
    "@id": "urn:eigenius:demo:jump:problem:lp",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:OptimisationProblem"],
    "urn:eigenius:core:short_name": "lp_demo",
    "urn:eigenius:jump:variable_names": ["x", "y"],
    "urn:eigenius:jump:variable_bounds": [
      "urn:eigenius:demo:jump:bound:lp:x",
      "urn:eigenius:demo:jump:bound:lp:y"
    ],
    "urn:eigenius:jump:objective": {
      "ctor": "App",
      "args": [
        {
          "ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
            {"ctor": "Var", "args": ["x"]}
          ]
        },
        {
          "ctor": "App",
          "args": [
            {
              "ctor": "App",
              "args": [
                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
                {"ctor": "LitFloat", "args": [2.0]}
              ]
            },
            {"ctor": "Var", "args": ["y"]}
          ]
        }
      ]
    },
    "urn:eigenius:jump:sense": "Min",
    "urn:eigenius:jump:constraints": ["urn:eigenius:demo:jump:cstr:lp:sum_le"]
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$LP_FILE"
echo

# Step 8: Commit the Holds-case OptimisesTo for the LP.
echo "--- Step 8: Commit Holds-case OptimisesTo (LP, objective_value = 0.0) ---"
LP_HOLDS_FILE="$(mktemp -t eigenius-jump-lp-holds-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$LP_FILE" "$LP_HOLDS_FILE"' EXIT
cat >"$LP_HOLDS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:jump:optimum:lp",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:OptimisesTo"],
    "urn:eigenius:core:short_name": "lp_demo_optimum",
    "urn:eigenius:jump:problem": "urn:eigenius:demo:jump:problem:lp",
    "urn:eigenius:jump:termination_status": "OPTIMAL",
    "urn:eigenius:jump:objective_value": 0.0,
    "urn:eigenius:jump:variable_values": [0.0, 0.0],
    "urn:eigenius:jump:abstol": 1e-6,
    "urn:eigenius:jump:reltol": 1e-6
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$LP_HOLDS_FILE"
echo

# Step 9: Commit the QP OptimisationProblem.
#
# (x-1)² + (y-2)² as a FormulaTerm:
#   add(pow(sub(x, 1.0), 2.0), pow(sub(y, 2.0), 2.0))
# The walker's smart-pow rule unrolls each `pow(•, LitFloat(2.0))` to
# repeated multiplication so the result is QuadExpr (HiGHS-acceptable).
echo "--- Step 9: Commit QP OptimisationProblem (smart-pow path) ---"
QP_FILE="$(mktemp -t eigenius-jump-qp-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$LP_FILE" "$LP_HOLDS_FILE" "$QP_FILE"' EXIT
cat >"$QP_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:jump:cstr:qp:sum_eq",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:Constraint"],
    "urn:eigenius:jump:lhs": {
      "ctor": "App",
      "args": [
        {
          "ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
            {"ctor": "Var", "args": ["x"]}
          ]
        },
        {"ctor": "Var", "args": ["y"]}
      ]
    },
    "urn:eigenius:jump:relation": {"ctor": "EQ"},
    "urn:eigenius:jump:rhs": 2.0
  },
  {
    "@id": "urn:eigenius:demo:jump:problem:qp",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:OptimisationProblem"],
    "urn:eigenius:core:short_name": "qp_demo",
    "urn:eigenius:jump:variable_names": ["x", "y"],
    "urn:eigenius:jump:objective": {
      "ctor": "App",
      "args": [
        {
          "ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
            {
              "ctor": "App",
              "args": [
                {
                  "ctor": "App",
                  "args": [
                    {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:pow"]},
                    {
                      "ctor": "App",
                      "args": [
                        {
                          "ctor": "App",
                          "args": [
                            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:sub"]},
                            {"ctor": "Var", "args": ["x"]}
                          ]
                        },
                        {"ctor": "LitFloat", "args": [1.0]}
                      ]
                    }
                  ]
                },
                {"ctor": "LitFloat", "args": [2.0]}
              ]
            }
          ]
        },
        {
          "ctor": "App",
          "args": [
            {
              "ctor": "App",
              "args": [
                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:pow"]},
                {
                  "ctor": "App",
                  "args": [
                    {
                      "ctor": "App",
                      "args": [
                        {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:sub"]},
                        {"ctor": "Var", "args": ["y"]}
                      ]
                    },
                    {"ctor": "LitFloat", "args": [2.0]}
                  ]
                }
              ]
            },
            {"ctor": "LitFloat", "args": [2.0]}
          ]
        }
      ]
    },
    "urn:eigenius:jump:sense": "Min",
    "urn:eigenius:jump:constraints": ["urn:eigenius:demo:jump:cstr:qp:sum_eq"]
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$QP_FILE"
echo

# Step 10: Commit the Holds-case OptimisesTo for the QP.
echo "--- Step 10: Commit Holds-case OptimisesTo (QP, objective_value = 0.5) ---"
QP_HOLDS_FILE="$(mktemp -t eigenius-jump-qp-holds-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$LP_FILE" "$LP_HOLDS_FILE" "$QP_FILE" "$QP_HOLDS_FILE"' EXIT
cat >"$QP_HOLDS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:jump:optimum:qp",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:OptimisesTo"],
    "urn:eigenius:core:short_name": "qp_demo_optimum",
    "urn:eigenius:jump:problem": "urn:eigenius:demo:jump:problem:qp",
    "urn:eigenius:jump:termination_status": "OPTIMAL",
    "urn:eigenius:jump:objective_value": 0.5,
    "urn:eigenius:jump:variable_values": [0.5, 1.5],
    "urn:eigenius:jump:abstol": 1e-4,
    "urn:eigenius:jump:reltol": 1e-4
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$QP_HOLDS_FILE"
echo

# Step 11: Try to commit a Fails-case OptimisesTo for the LP
# (claim objective = -1, which is unreachable: the LP is bounded
# below by 0). The gate refutes; the kernel rejects the load.
echo "--- Step 11: Try to commit Fails-case OptimisesTo (LP, claimed objective = -1; expect rejection) ---"
FAILS_FILE="$(mktemp -t eigenius-jump-fails-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$LP_FILE" "$LP_HOLDS_FILE" "$QP_FILE" "$QP_HOLDS_FILE" "$FAILS_FILE"' EXIT
cat >"$FAILS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:jump:optimum:lp_wrong",
    "urn:eigenius:core:is_a": ["urn:eigenius:jump:OptimisesTo"],
    "urn:eigenius:core:short_name": "lp_wrong_optimum",
    "urn:eigenius:jump:problem": "urn:eigenius:demo:jump:problem:lp",
    "urn:eigenius:jump:termination_status": "OPTIMAL",
    "urn:eigenius:jump:objective_value": -1.0,
    "urn:eigenius:jump:variable_values": [0.0, 0.0],
    "urn:eigenius:jump:abstol": 1e-6,
    "urn:eigenius:jump:reltol": 1e-6
  }
]
EOF
if $EIGENIUS --endpoint "$ENDPOINT" load "$FAILS_FILE"; then
    echo "WARNING: Fails-case load unexpectedly succeeded — Verdict shape may be off."
else
    echo "Load rejected as expected (Fails verdict from validate_optimum)."
fi
echo

# Step 12: Inspect Verdicts.
echo "--- Step 12: Query Verdicts ---"
$EIGENIUS --endpoint "$ENDPOINT" query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:core:ctor_name": ?ctor } RETURN [] { verdict: ?v, ctor: ?ctor }'
echo

echo "=== Demo complete ==="
