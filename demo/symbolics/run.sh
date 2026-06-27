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
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) { "urn:eigenius:core:short_name": ?name } WHERE ?name IN ["SimplifiesTo", "SimplifyRequest", "EquivalenceCheck", "Substitutes", "SatisfiesEquation", "SymbolicallyReducesTo"] RETURN [] { iri: ?iri }' \
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
     USING NAMESPACE "urn:eigenius:symbolics:query_classes:"
     FIBER cap:qc_symb_simplify {
       expr: "urn:eigenius:demo:symbolics:expr:x_plus_0_times_1"
     } AS ?simplified
     RETURN [] { result: ?simplified, term: ?simplified.term }'
echo

# Step 11: Commit a Substitutes claim (Phase 19d.5). We claim that
# substituting `x` with `0` in `x + 1` yields `1`. AutoOnLoad fires
# `validate_substitutes`, which re-runs `Symbolics.substitute(target,
# x => 0)` and compares against the claimed result.
echo "--- Step 11: Commit Substitutes claim (x↦0 in x+1 == 1; expect Holds) ---"
SUBSTITUTES_FILE="$(mktemp -t eigenius-symbolics-substitutes-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE" "$INPUT_EXPR_FILE" "$SUBSTITUTES_FILE"' EXIT
cat >"$SUBSTITUTES_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:symbolics:claim:sub_x_with_0_in_x_plus_1",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:Substitutes"],
    "urn:eigenius:core:short_name": "sub_x_with_0_in_x_plus_1",
    "urn:eigenius:symbolics:target": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "x_plus_1",
      "urn:eigenius:symbolics:term": {
        "ctor": "App",
        "args": [
          {
            "ctor": "App",
            "args": [
              {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
              {"ctor": "Var", "args": ["x"]}
            ]
          },
          {"ctor": "LitFloat", "args": [1.0]}
        ]
      }
    },
    "urn:eigenius:symbolics:variable": "x",
    "urn:eigenius:symbolics:replacement": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "lit_zero",
      "urn:eigenius:symbolics:term": {
        "ctor": "LitFloat",
        "args": [0.0]
      }
    },
    "urn:eigenius:symbolics:result": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "lit_one",
      "urn:eigenius:symbolics:term": {
        "ctor": "LitFloat",
        "args": [1.0]
      }
    }
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$SUBSTITUTES_FILE"
echo

# Step 12: Commit a SatisfiesEquation claim (Phase 19d.4). We claim
# that the equation `x + 0 = x` holds unconditionally (empty bindings
# = pure algebraic equivalence between the two sides). AutoOnLoad
# fires `validate_satisfies_equation`, which simplifies the residue
# `lhs - rhs` and asserts it's zero.
echo "--- Step 12: Commit SatisfiesEquation claim (x+0 = x; expect Holds) ---"
SATISFIES_FILE="$(mktemp -t eigenius-symbolics-satisfies-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE" "$INPUT_EXPR_FILE" "$SUBSTITUTES_FILE" "$SATISFIES_FILE"' EXIT
cat >"$SATISFIES_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:symbolics:claim:x_plus_0_eq_x",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SatisfiesEquation"],
    "urn:eigenius:core:short_name": "x_plus_0_eq_x",
    "urn:eigenius:symbolics:equation": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicEquation"],
      "urn:eigenius:core:short_name": "x_plus_0_equation",
      "urn:eigenius:symbolics:lhs": {
        "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
        "urn:eigenius:core:short_name": "lhs_x_plus_0",
        "urn:eigenius:symbolics:term": {
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
      },
      "urn:eigenius:symbolics:rhs": {
        "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
        "urn:eigenius:core:short_name": "rhs_x",
        "urn:eigenius:symbolics:term": {
          "ctor": "Var",
          "args": ["x"]
        }
      }
    },
    "urn:eigenius:symbolics:bindings": []
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$SATISFIES_FILE"
echo

# Step 13: Commit a SymbolicallyReducesTo claim (Phase 19d.6) under
# the `Expand` strategy. Source: `2 * (x + 1)`; claimed result:
# `2*x + 2`. AutoOnLoad fires `validate_symbolically_reduces_to`,
# which dispatches on the strategy ctor (`Expand` →
# `Symbolics.expand`) and compares the resulting form against the
# claim.
echo "--- Step 13: Commit SymbolicallyReducesTo claim (Expand 2*(x+1) == 2*x+2; expect Holds) ---"
REDUCES_FILE="$(mktemp -t eigenius-symbolics-reduces-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE" "$INPUT_EXPR_FILE" "$SUBSTITUTES_FILE" "$SATISFIES_FILE" "$REDUCES_FILE"' EXIT
cat >"$REDUCES_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:symbolics:claim:expand_2_times_x_plus_1",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicallyReducesTo"],
    "urn:eigenius:core:short_name": "expand_2_times_x_plus_1",
    "urn:eigenius:symbolics:expr": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "two_times_x_plus_one",
      "urn:eigenius:symbolics:term": {
        "ctor": "App",
        "args": [
          {
            "ctor": "App",
            "args": [
              {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
              {"ctor": "LitFloat", "args": [2.0]}
            ]
          },
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
              {"ctor": "LitFloat", "args": [1.0]}
            ]
          }
        ]
      }
    },
    "urn:eigenius:symbolics:strategy": {
      "ctor": "Expand",
      "args": []
    },
    "urn:eigenius:symbolics:result": {
      "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "urn:eigenius:core:short_name": "two_x_plus_two",
      "urn:eigenius:symbolics:term": {
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
                      {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
                      {"ctor": "LitFloat", "args": [2.0]}
                    ]
                  },
                  {"ctor": "Var", "args": ["x"]}
                ]
              }
            ]
          },
          {"ctor": "LitFloat", "args": [2.0]}
        ]
      }
    }
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$REDUCES_FILE"
echo

# Step 14: Re-query Verdicts to surface the four new ones (Substitutes,
# SatisfiesEquation, SymbolicallyReducesTo, plus the original
# SimplifiesTo from Step 7).
echo "--- Step 14: Query all Verdicts (after AutoOnLoad-gated commits) ---"
$EIGENIUS --endpoint "$ENDPOINT" query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:core:ctor_name": ?ctor } RETURN [] { verdict: ?v, ctor: ?ctor }'
echo

# Step 15: Decidable EigenQL invocation — `qc_symb_check_equivalence`.
# Pre-commit two SymbolicExpressions (`x + 0` and `x`); pass them
# positionally to the Decidable predicate. The kernel's typed-property
# marshaling populates `EquivalenceCheck.lhs` / `EquivalenceCheck.rhs`
# from the IRI-string args (Phase 19d.7), the worker decodes the
# typed input and runs Symbolics' simplifier on both sides. The
# returned Verdict shows in the row.
echo "--- Step 15: Pre-commit lhs/rhs SymbolicExpressions for Decidable check ---"
LHS_RHS_FILE="$(mktemp -t eigenius-symbolics-lhs-rhs-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$INSTANCE_FILE" "$INPUT_EXPR_FILE" "$SUBSTITUTES_FILE" "$SATISFIES_FILE" "$REDUCES_FILE" "$LHS_RHS_FILE"' EXIT
cat >"$LHS_RHS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:symbolics:expr:lhs_x_plus_0",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
    "urn:eigenius:core:short_name": "lhs_x_plus_0",
    "urn:eigenius:symbolics:term": {
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
  },
  {
    "@id": "urn:eigenius:demo:symbolics:expr:rhs_x",
    "urn:eigenius:core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
    "urn:eigenius:core:short_name": "rhs_x",
    "urn:eigenius:symbolics:term": {
      "ctor": "Var",
      "args": ["x"]
    }
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$LHS_RHS_FILE"
echo

echo "--- Step 16: Decidable EigenQL invocation — qc_symb_check_equivalence ---"
# `cap:qc_symb_check_equivalence(?lhs_iri, ?rhs_iri)` is a Decidable
# predicate call returning a Verdict resource. Postfix `:Holds`
# projects to Boolean for use in WHERE / RETURN expressions.
$EIGENIUS --endpoint "$ENDPOINT" query \
    'USING INSTITUTION "urn:eigenius:institutions:symbolics" AS cap
     RETURN [] {
       verdict: cap:qc_symb_check_equivalence(
         "urn:eigenius:demo:symbolics:expr:lhs_x_plus_0",
         "urn:eigenius:demo:symbolics:expr:rhs_x"
       )
     }'
echo

echo "=== Demo complete ==="
