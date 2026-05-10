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

# Setup for the kinase-institutions notebook
# (`notebooks/examples/kinase-institutions.json`).
#
# Loads every ontology, builds every Julia institution's env image,
# commits every institution declaration, and registers the cross-
# institution comorphisms — everything the notebook needs to run.
#
# Intended flow:
#
#   1. `just up` — start the docker compose stack.
#   2. `./notebooks/examples/kinase-institutions-setup.sh` — this script.
#      Cold first run takes ~30–60 minutes (five env images each pull
#      Julia + per-institution dep tail). Subsequent runs reuse the
#      buildah cache; minutes.
#   3. Open <http://localhost:8080/notebooks/> in a browser.
#   4. Import `notebooks/examples/kinase-institutions.json`.
#   5. Click "Run All".
#
# What's installed
# ----------------
#
# Five institutions (D27 §4):
#
#   - `urn:eigenius:institutions:symbolics`   — Symbolics.jl (D27 §4.1)
#   - `urn:eigenius:institutions:intervals`   — IntervalArithmetic.jl (D27 §4.3)
#   - `urn:eigenius:institutions:catalyst`    — Catalyst.jl (D27 §4.4)
#   - `urn:eigenius:institutions:diffeq`      — OrdinaryDiffEq.jl (D27 §4.5)
#   - `urn:eigenius:institutions:jump_highs`  — JuMP+HiGHS (D27 §4.2; LP/QP)
#
# Three comorphisms over the shared `formulas:FormulaTerm` payload:
#
#   - `urn:eigenius:comorphisms:symbolics_to_intervals`  (Phase 19d.2 / D32 §6.2)
#   - `urn:eigenius:comorphisms:catalyst_to_diffeq`      (Phase 19h.1 / D27 §4.4.4)
#   - `urn:eigenius:comorphisms:symbolics_to_jump`       (Phase 19f.1 / D27 §4.2)
#
# Idempotence
# -----------
#
# The script is *not* fully idempotent: `eigenius load` rejects a re-
# committed identical resource (same content hash → same IRI = no-op
# ok; modified content with the same IRI = error). If a partial run
# left intermediate state, the cleanest reset is `docker compose down -v`
# followed by `docker compose up -d` and re-running this script.
#
# Usage
# -----
#
#   ./notebooks/examples/kinase-institutions-setup.sh
#   ./notebooks/examples/kinase-institutions-setup.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

INSTITUTIONS_DIR="$REPO_DIR/julia/institutions"
COMORPHISMS_DIR="$REPO_DIR/julia/comorphisms"

BASE_IMAGE="docker.io/library/julia:1.12-bookworm"

MIRROR_BASE_DIR="$(mktemp -d -t eigenius-kinase-inst-mirrors-XXXXXX)"
trap 'rm -rf "$MIRROR_BASE_DIR"' EXIT

echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius --endpoint $ENDPOINT"

echo "=== Eigenius kinase-institutions notebook setup ==="
echo "Kernel:        $ENDPOINT"
echo "Orchestrator:  $ORCHESTRATOR"
echo "Mirrors:       $MIRROR_BASE_DIR"
echo

# Step 0: Health check.
#
# `docker compose up -d` returns when containers are *started*, not
# when their listeners accept connections. The orchestrator has a
# healthcheck and reaches "Healthy" reliably, but the kernel
# container doesn't (yet) — so the kernel's gRPC listener may still
# be coming up when the orchestrator is already routable. Retry the
# kernel probe a few times before giving up.
echo "--- Step 0: Health check ---"
if ! curl -sf "$ORCHESTRATOR/health" >/dev/null; then
    echo "ERROR: Orchestrator not reachable at $ORCHESTRATOR/health"
    echo "Start the stack first: docker compose up -d (or: just up)"
    exit 1
fi

KERNEL_READY=0
for attempt in $(seq 1 30); do
    if $EIGENIUS inspect "urn:eigenius:core:Class" >/dev/null 2>&1; then
        KERNEL_READY=1
        break
    fi
    if [ "$attempt" -eq 1 ]; then
        echo -n "Waiting for kernel listener at $ENDPOINT"
    fi
    echo -n "."
    sleep 1
done
echo

if [ "$KERNEL_READY" -ne 1 ]; then
    echo "ERROR: Kernel not reachable at $ENDPOINT after 30 attempts"
    echo "Check 'docker compose logs kernel' for startup errors."
    exit 1
fi

echo "Stack healthy."
echo

# ─── Helpers ────────────────────────────────────────────────────────────

# Resolve the current head layer IRI so mirror generation anchors
# against a stable layer.
head_layer() {
    local hex
    hex=$($EIGENIUS branch show main | awk '{print $2}')
    echo "urn:eigenius:layer:$hex"
}

# Per-institution lifecycle: load ontology (no-op if already loaded),
# generate mirror, build env image, commit RuntimeEnvironment, install
# institution declaration. The ontology is loaded *before* this is
# called for cross-referencing institutions; pass `none` for
# `ontology_file` to skip the load step.
setup_institution() {
    local label="$1"             # e.g. "DiffEq"
    local handler_pkg="$2"       # e.g. "EigeniusDiffEq"
    local handler_pkg_dir="$3"   # absolute path to handler package directory
    local institution_file="$4"  # absolute path to {name}-institution.eigon.json
    local env_iri="$5"           # e.g. "urn:eigenius:diffeq:env:v1"
    local mirror_seed_filter="$6"  # MATCH … RETURN clause selecting mirror seed classes

    echo "--- Setting up institution: $label ---"

    local layer
    layer=$(head_layer)
    echo "Head layer: $layer"

    local mirror_dir="$MIRROR_BASE_DIR/$handler_pkg"
    mkdir -p "$mirror_dir"

    # `--institution-file` augments the seed with cross-institution
    # return classes by reading the institution declaration file's
    # `RuntimeMethodSignature` resources directly. The file is the
    # source of truth at this stage of the pipeline; the institution
    # itself is only committed to the chain at `institution install`
    # time later in this function.
    local mirror_json="$MIRROR_BASE_DIR/$handler_pkg-mirror.json"
    $EIGENIUS mirror create \
        --layer "$layer" \
        --filter "$mirror_seed_filter" \
        --institution-file "$institution_file" \
        --language julia \
        --output "$mirror_dir" \
        --json | tee "$mirror_json" >/dev/null
    local mirror_iri
    mirror_iri=$(jq -r '.mirror_iri' < "$mirror_json")
    echo "  mirror IRI: $mirror_iri"

    local envbuild_json="$MIRROR_BASE_DIR/$handler_pkg-envbuild.json"
    $EIGENIUS env build \
        --language julia \
        --package-path "$handler_pkg_dir" \
        --mirror "$mirror_iri" \
        --base-image "$BASE_IMAGE" \
        --json | tee "$envbuild_json" >/dev/null
    local image_digest runtime_version
    image_digest=$(jq -r '.image_digest' < "$envbuild_json")
    runtime_version=$(jq -r '.runtime_version' < "$envbuild_json")
    echo "  image digest:    $image_digest"
    echo "  runtime version: $runtime_version"

    $EIGENIUS env create \
        --language julia \
        --handler-package "$handler_pkg_dir" \
        --mirror "$mirror_iri" \
        --as-iri "$env_iri" \
        --image-digest "$image_digest" \
        --runtime-version "$runtime_version" >/dev/null
    echo "  env Resource committed: $env_iri"

    $EIGENIUS institution install --definition "$institution_file" >/dev/null
    echo "  institution installed: $(basename "$institution_file")"
    echo
}

# ─── Step 1: Ontology layer ─────────────────────────────────────────────
#
# Order matters — properties' `class_types` references must resolve at
# commit time. JuMP first because Symbolics' SymbolicsToJuMPInput
# references jump:VariableBound and jump:Constraint. Symbolics next
# because intervals' BoundsRequest references SymbolicExpression and
# catalyst's CatalystToOdeInput references diffeq:OdeProblem.

echo "--- Step 1: Load ontologies ---"
$EIGENIUS load "$INSTITUTIONS_DIR/jump/declarations/jump-ontology.eigon.json"
$EIGENIUS load "$INSTITUTIONS_DIR/symbolics/declarations/symbolics-ontology.eigon.json"
$EIGENIUS load "$INSTITUTIONS_DIR/intervals/declarations/intervals-ontology.eigon.json"
$EIGENIUS load "$INSTITUTIONS_DIR/diffeq/declarations/diffeq-ontology.eigon.json"
$EIGENIUS load "$INSTITUTIONS_DIR/catalyst/declarations/catalyst-ontology.eigon.json"
echo "All ontologies loaded."
echo

# ─── Step 2: Per-institution mirror + env + install ─────────────────────

# Each institution's seed filter selects only its *own* declared
# classes. Cross-institution return classes (e.g. `OptimisationProblem`
# returned from `Symbolics → JuMP`'s `frame_as_optimisation_problem`)
# are discovered automatically by `mirror create --institution-file`,
# which parses the institution declaration and folds every
# `RuntimeMethodSignature.input_types` / `output_type` class into the
# seed before closure expansion.

# JuMP-HiGHS — LP/QP solver (D27 §4.2).
setup_institution \
    "JuMP-HiGHS" \
    "EigeniusJuMPHiGHS" \
    "$INSTITUTIONS_DIR/jump/EigeniusJuMPHiGHS" \
    "$INSTITUTIONS_DIR/jump/declarations/jump-highs-institution.eigon.json" \
    "urn:eigenius:jump_highs:env:v1" \
    'MATCH "urn:eigenius:core:Class"(?iri) {
        "urn:eigenius:core:short_name": ?name
     }
     WHERE ?name IN ["OptimisationProblem", "OptimisesTo"]
     RETURN [] { iri: ?iri }'

# Symbolics — symbolic algebra over FormulaTerm (D27 §4.1).
setup_institution \
    "Symbolics" \
    "EigeniusSymbolics" \
    "$INSTITUTIONS_DIR/symbolics/EigeniusSymbolics" \
    "$INSTITUTIONS_DIR/symbolics/declarations/symbolics-institution.eigon.json" \
    "urn:eigenius:symbolics:env:v1" \
    'MATCH "urn:eigenius:core:Class"(?iri) {
        "urn:eigenius:core:short_name": ?name
     }
     WHERE ?name IN ["SymbolicExpression", "SimplifiesTo", "SimplifyRequest",
                     "EquivalenceCheck", "SatisfiesEquation", "Substitutes",
                     "SymbolicallyReducesTo", "SymbolicsToJuMPInput"]
     RETURN [] { iri: ?iri }'

# IntervalArithmetic — rigorous bounds (D27 §4.3).
setup_institution \
    "IntervalArithmetic" \
    "EigeniusIntervals" \
    "$INSTITUTIONS_DIR/intervals/EigeniusIntervals" \
    "$INSTITUTIONS_DIR/intervals/declarations/intervals-institution.eigon.json" \
    "urn:eigenius:intervals:env:v1" \
    'MATCH "urn:eigenius:core:Class"(?iri) {
        "urn:eigenius:core:short_name": ?name
     }
     WHERE ?name IN ["BoundedBy", "BoundsRequest", "IntervalFunction"]
     RETURN [] { iri: ?iri }'

# DiffEq — ODE integration (D27 §4.5).
setup_institution \
    "DiffEq" \
    "EigeniusDiffEq" \
    "$INSTITUTIONS_DIR/diffeq/EigeniusDiffEq" \
    "$INSTITUTIONS_DIR/diffeq/declarations/diffeq-institution.eigon.json" \
    "urn:eigenius:diffeq:env:v1" \
    'MATCH "urn:eigenius:core:Class"(?iri) {
        "urn:eigenius:core:short_name": ?name
     }
     WHERE ?name IN ["OdeProblem", "OdeSolution", "RhsComponent"]
     RETURN [] { iri: ?iri }'

# Catalyst — chemical-reaction networks (D27 §4.4).
setup_institution \
    "Catalyst" \
    "EigeniusCatalyst" \
    "$INSTITUTIONS_DIR/catalyst/EigeniusCatalyst" \
    "$INSTITUTIONS_DIR/catalyst/declarations/catalyst-institution.eigon.json" \
    "urn:eigenius:catalyst:env:v1" \
    'MATCH "urn:eigenius:core:Class"(?iri) {
        "urn:eigenius:core:short_name": ?name
     }
     WHERE ?name IN ["ReactionNetwork", "ConservationLaw", "CatalystToOdeInput"]
     RETURN [] { iri: ?iri }'

# ─── Step 3: Comorphisms ────────────────────────────────────────────────
#
# Loaded last, after all institutions are installed — comorphism
# triples reference both source-side ExportFormats and target-side
# ImportFormats, all of which now resolve.

echo "--- Step 3: Load comorphisms ---"
$EIGENIUS load "$COMORPHISMS_DIR/symbolics-to-intervals.eigon.json"
$EIGENIUS load "$COMORPHISMS_DIR/catalyst-to-diffeq.eigon.json"
$EIGENIUS load "$COMORPHISMS_DIR/symbolics-to-jump.eigon.json"
echo "All comorphisms loaded."
echo

# ─── Smoke check ────────────────────────────────────────────────────────

echo "--- Smoke check: query installed institutions ---"
$EIGENIUS query \
    'MATCH "urn:eigenius:institution:Institution"(?inst) {
        "urn:eigenius:core:short_name": ?name
     }
     RETURN [] { iri: ?inst, name: ?name }
     ORDER BY ?name'
echo

echo "=== Setup complete ==="
echo
echo "Next steps:"
echo "  1. Open <$ORCHESTRATOR/notebooks/> in a browser."
echo "  2. Click \"Import…\" and select"
echo "       notebooks/examples/kinase-institutions.json"
echo "  3. Click \"Run All\"."
