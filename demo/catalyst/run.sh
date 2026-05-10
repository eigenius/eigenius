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

# Eigenius Catalyst Institution — End-to-End Demo
#
# Walks the full developer flow for the Catalyst institution against
# the local docker-compose stack. The worked example is the classical
# kinase-inhibition reaction network — the mechanism underneath the
# IC50 measurements you'd find in the Phase 5 kinase-screening
# notebook (see notebooks/examples/kinase-institutions.json):
#
#   E + S  ⇌  ES  →  E + P    (Michaelis-Menten catalysis)
#   E + I  ⇌  EI              (competitive inhibition)
#
# Three conservation laws follow from the network's stoichiometry:
#
#   - Enzyme:   [E] + [ES] + [EI]      = E_total
#   - Substrate-or-product:
#               [S] + [ES] + [P]       = S_total
#   - Inhibitor: [I] + [EI]            = I_total
#
# The demo commits the network plus all three claims and asserts the
# AutoOnLoad gate produces three Holds verdicts.
#
# Steps:
#
#   1. Health-check kernel + orchestrator.
#   2. Load the Catalyst ontology (ReactionNetwork + ConservationLaw).
#   3. Generate the Julia mirror.
#   4. Build the env image with the EigeniusCatalyst handler package.
#      First-time build is slow (~10 min cold) — Catalyst.jl pulls
#      MTK + SymbolicUtils + DiffEqBase + a long tail of SciML deps.
#   5. Commit the RuntimeEnvironment.
#   6. Install the institution declaration.
#   7. Commit the kinase ReactionNetwork.
#   8. Commit three ConservationLaw claims — kernel's AutoOnLoad
#      fires `validate_conservation_law` on each, three Holds verdicts
#      land on the chain.
#   9. Query the Verdicts.
#
# Prerequisites:
#   docker compose up      (or: EIGENIUS_MOCK_LLM=true docker compose up)
#   docker daemon reachable on the host
#
# Usage:
#   ./demo/catalyst/run.sh
#   ./demo/catalyst/run.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTITUTION_DIR="$REPO_DIR/julia/institutions/catalyst"
ONTOLOGY_FILE="$INSTITUTION_DIR/declarations/catalyst-ontology.eigon.json"
INSTITUTION_FILE="$INSTITUTION_DIR/declarations/catalyst-institution.eigon.json"
HANDLER_PKG_DIR="$INSTITUTION_DIR/EigeniusCatalyst"

ENV_IRI="urn:eigenius:catalyst:env:v1"

echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius"

echo "=== Eigenius Catalyst Institution Demo ==="
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

# Step 1: Load the Catalyst ontology classes.
echo "--- Step 1: Load Catalyst ontology ---"
$EIGENIUS --endpoint "$ENDPOINT" load "$ONTOLOGY_FILE"
echo

# Step 2: Resolve head layer for the mirror anchor.
echo "--- Step 2: Resolve head layer ---"
HEAD_HEX=$($EIGENIUS --endpoint "$ENDPOINT" branch show main | awk '{print $2}')
LAYER_IRI="urn:eigenius:layer:$HEAD_HEX"
echo "Head layer: $LAYER_IRI"
echo

# Step 3: Generate + commit the Julia mirror covering ReactionNetwork
# and ConservationLaw. The closure walker pulls in the property
# definitions automatically.
echo "--- Step 3: Generate + commit mirror ---"
MIRROR_OUTPUT_DIR=$(mktemp -d -t eigenius-catalyst-mirror-XXXXXX)
trap 'rm -rf "$MIRROR_OUTPUT_DIR"' EXIT

$EIGENIUS --endpoint "$ENDPOINT" mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["ReactionNetwork", "ConservationLaw"]
              RETURN [] { iri: ?iri }' \
    --language julia \
    --output "$MIRROR_OUTPUT_DIR" \
    --json | tee /tmp/eigenius-catalyst-mirror.json
MIRROR_IRI=$(jq -r '.mirror_iri' < /tmp/eigenius-catalyst-mirror.json)
echo "Mirror IRI: $MIRROR_IRI"
echo

# Step 4: Build the env image with the EigeniusCatalyst handler baked
# in. Cold runs pull Julia + Pkg.precompile Catalyst — substantially
# slower than IntervalArithmetic (~10 min vs ~1 min) because Catalyst
# pulls MTK + SymbolicUtils + DiffEqBase + a long SciML dep tail.
# Cached after the first build.
echo "--- Step 4: Build env image ---"
$EIGENIUS --endpoint "$ENDPOINT" env build \
    --language julia \
    --package-path "$HANDLER_PKG_DIR" \
    --mirror "$MIRROR_IRI" \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json | tee /tmp/eigenius-catalyst-envbuild.json
IMAGE_DIGEST=$(jq -r '.image_digest' < /tmp/eigenius-catalyst-envbuild.json)
RUNTIME_VERSION=$(jq -r '.runtime_version' < /tmp/eigenius-catalyst-envbuild.json)
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

# Step 6: Install the Catalyst institution declaration.
echo "--- Step 6: Install institution ---"
$EIGENIUS --endpoint "$ENDPOINT" institution install --definition "$INSTITUTION_FILE"
echo

# Step 7: Commit the kinase reaction network.
#
# Note on species ordering: Catalyst's `species(rn)` returns species
# in first-appearance order across the reactions. Reading the macro:
#   R1: E + S --> ES        introduces E, S, ES
#   R2: ES --> E + S        (no new species)
#   R3: ES --> E + P        introduces P
#   R4: E + I --> EI        introduces I, EI
#   R5: EI --> E + I        (no new species)
# So the canonical order is [E, S, ES, P, I, EI] — six species. The
# ConservationLaw coefficients in step 8 use this ordering positionally.
echo "--- Step 7: Commit kinase ReactionNetwork ---"
NETWORK_FILE="$(mktemp -t eigenius-catalyst-network-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$NETWORK_FILE"' EXIT
cat >"$NETWORK_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:catalyst:network:kinase",
    "urn:eigenius:core:is_a": ["urn:eigenius:catalyst:ReactionNetwork"],
    "urn:eigenius:core:short_name": "kinase_with_competitive_inhibition",
    "urn:eigenius:catalyst:network_source": "@reaction_network begin\n    k_on_S, E + S --> ES\n    k_off_S, ES --> E + S\n    k_cat, ES --> E + P\n    k_on_I, E + I --> EI\n    k_off_I, EI --> E + I\nend",
    "urn:eigenius:catalyst:species_declared": ["E", "S", "ES", "P", "I", "EI"],
    "urn:eigenius:catalyst:parameters_declared": ["k_on_S", "k_off_S", "k_cat", "k_on_I", "k_off_I"]
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$NETWORK_FILE"
echo

# Step 8: Commit three ConservationLaw claims. Each triggers the
# AutoOnLoad gate; each should yield Holds.
#
# Coefficients align positionally with species_declared = [E, S, ES, P, I, EI]:
#   Enzyme:               [1, 0, 1, 0, 0, 1]   ([E] + [ES] + [EI])
#   Substrate-or-product: [0, 1, 1, 1, 0, 0]   ([S] + [ES] + [P])
#   Inhibitor:            [0, 0, 0, 0, 1, 1]   ([I] + [EI])
echo "--- Step 8: Commit three ConservationLaw claims ---"
LAWS_FILE="$(mktemp -t eigenius-catalyst-laws-XXXXXX.json)"
trap 'rm -rf "$MIRROR_OUTPUT_DIR" "$NETWORK_FILE" "$LAWS_FILE"' EXIT
cat >"$LAWS_FILE" <<'EOF'
[
  {
    "@id": "urn:eigenius:demo:catalyst:law:enzyme_conservation",
    "urn:eigenius:core:is_a": ["urn:eigenius:catalyst:ConservationLaw"],
    "urn:eigenius:core:short_name": "enzyme_conservation",
    "urn:eigenius:catalyst:network": "urn:eigenius:demo:catalyst:network:kinase",
    "urn:eigenius:catalyst:coefficients": [1, 0, 1, 0, 0, 1]
  },
  {
    "@id": "urn:eigenius:demo:catalyst:law:substrate_product_conservation",
    "urn:eigenius:core:is_a": ["urn:eigenius:catalyst:ConservationLaw"],
    "urn:eigenius:core:short_name": "substrate_product_conservation",
    "urn:eigenius:catalyst:network": "urn:eigenius:demo:catalyst:network:kinase",
    "urn:eigenius:catalyst:coefficients": [0, 1, 1, 1, 0, 0]
  },
  {
    "@id": "urn:eigenius:demo:catalyst:law:inhibitor_conservation",
    "urn:eigenius:core:is_a": ["urn:eigenius:catalyst:ConservationLaw"],
    "urn:eigenius:core:short_name": "inhibitor_conservation",
    "urn:eigenius:catalyst:network": "urn:eigenius:demo:catalyst:network:kinase",
    "urn:eigenius:catalyst:coefficients": [0, 0, 0, 0, 1, 1]
  }
]
EOF
$EIGENIUS --endpoint "$ENDPOINT" load "$LAWS_FILE"
echo

# Step 9: Inspect Verdicts. Three should land — one per claim, all
# Holds.
echo "--- Step 9: Query Verdicts ---"
$EIGENIUS --endpoint "$ENDPOINT" query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) { "urn:eigenius:core:ctor_name": ?ctor } RETURN [] { verdict: ?v, ctor: ?ctor }'
echo

echo "=== Demo complete ==="
