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

# Eigenius WRN-Helicase paper — End-to-End Recompute Demo (Chan et al., Nature 2019)
#
# Loads the full WRN synthetic-lethality encoding against the local
# docker-compose stack and shows every headline warrant land as a
# kernel-checked `Holds` verdict — the statistics institution recomputing
# the wet-lab/dependency statistics from chain-resident data, and the R
# language runtime (D55/D56) wrapping the authors' own lme4 mixed model
# for the in-vivo claim.
#
# Two structural points this demo exercises:
#
#   1. Two-phase recompute load (D54 lemma citation). The recompute layer
#      is split into PLANS (SampleSets + StatisticalAnalysisPlans + bridges
#      — AutoOnLoad emits the StatisticalAnalysisResult IsDerivedAs
#      witnesses) and CONCLUSIONS (the ReasoningSentences that cite those
#      witnesses). Plans load first so the witnesses are in an ancestor
#      layer before the conclusions gate.
#
#   2. Wrapped-R component (D56). `concl_vivo` is NOT linked-external: the
#      KM12 xenograft tumour-volume table is dispatched through a
#      `RunRuntimeScript` program that runs the authors' lme4 random-slope
#      LRT (in_vivo_KM12_analysis.R) inside a substrate-spawned R container,
#      commits the LRT-p DerivedResource carrying
#      onco:InVivoDependence("WRN","MSI") under a ProgramTrace, and the
#      witness lifts concl_vivo.
#
# Prerequisites:
#   EIGENIUS_MOCK_LLM=true docker compose up -d   (or `just up-mock`)
#   docker daemon reachable on the host (the substrate spawns the R worker
#   as a sibling container via DooD).
#
# Usage:
#   ./demo/wrn-helicase/run.sh
#   ./demo/wrn-helicase/run.sh http://localhost:50051 http://localhost:8080

set -euo pipefail

ENDPOINT="${1:-http://localhost:50051}"
ORCHESTRATOR="${2:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
WRN="$REPO_DIR/experiments/publications/wrn-helicase"
PROGRAMS="$WRN/programs"

echo "Building eigenius CLI (one-time)..."
(cd "$REPO_DIR" && cargo build -q -p eigenius-cli)
EIGENIUS="$REPO_DIR/target/debug/eigenius"
eig() { "$EIGENIUS" --endpoint "$ENDPOINT" "$@"; }

echo "=== Eigenius WRN-Helicase Recompute Demo ==="
echo "Kernel:        $ENDPOINT"
echo "Orchestrator:  $ORCHESTRATOR"
echo

# Step 0: health check.
echo "--- Step 0: Health check ---"
if ! curl -sf "$ORCHESTRATOR/health" >/dev/null; then
    echo "ERROR: Orchestrator not reachable at $ORCHESTRATOR/health"
    echo "Start the stack first: EIGENIUS_MOCK_LLM=true docker compose up -d"
    exit 1
fi
eig inspect "urn:eigenius:core:Class" >/dev/null
echo "Stack healthy."
echo

# Step 1: ontology deps. reasoning + statistics are seeded in the kernel
# image; bench-core + onco carry the domain vocabulary (bench:Measurement,
# onco:Gene / onco:InVivoDependence / ...) the WRN chain + the
# canonical_proposition ConstRefs resolve against.
echo "--- Step 1: Load ontology deps (bench-core, onco) ---"
eig load "$REPO_DIR/experiments/benchmark/base-ontologies/bench-core.esl"
eig load "$WRN/onco.esl"
echo

# Step 2: narrative + recompute layers. The recompute is two-phase:
# plans (emitters) before conclusions (consumers).
echo "--- Step 2: Load WRN narrative + recompute (plans -> conclusions) ---"
eig load "$WRN/wrn-phase1.esl"
eig load "$WRN/wrn-phase1-recompute-plans.esl"
eig load "$WRN/wrn-phase1-recompute-conclusions.esl"
echo

# Step 3: the wrapped-R in-vivo warrant (D55/D56). Run the lme4 xenograft
# program; the substrate spawns the R worker container, runs the authors'
# random-slope LRT, and commits wrn:vivo_lme4:result (carrying the
# InVivoDependence proposition) under a ProgramTrace -> IsDerivedAs witness.
#
# The program's runtime:image_digest must point at the R+lme4 worker image.
# Build it once with:  eig env build --language r   (prints the digest), and
# paste it into programs/xenograft-lme4-program.json (or pass it here via
# $R_IMAGE_DIGEST to patch a temp copy).
# run_r_program <program.json> <input.json> <grep-pattern>: runs a wrapped-R
# program, patching runtime:image_digest from $R_IMAGE_DIGEST when set.
run_r_program() {
    local src="$1" input="$2" pat="$3" prog="$1"
    if [ -n "${R_IMAGE_DIGEST:-}" ]; then
        prog="$(mktemp -t wrn-r-prog-XXXXXX.json)"
        sed "s|sha256:[0-9a-f]\{64\}|$R_IMAGE_DIGEST|" "$src" > "$prog"
    fi
    eig run "$prog" "$input" | grep -iE "$pat" || true
    if [ "$prog" != "$src" ]; then rm -f "$prog"; fi
}

echo "--- Step 3: Run the wrapped-R warrants (lme4, D55/D56) ---"
# 3a. In-vivo: the authors' own random-slope LRT on the xenograft volumes,
#     committing wrn:vivo_lme4:result -> InVivoDependence(WRN,MSI) (concl_vivo).
echo "  3a. xenograft in-vivo lme4 -> InVivoDependence"
run_r_program "$PROGRAMS/xenograft-lme4-program.json" \
    "$PROGRAMS/xenograft-input.json" "lrt_p_value|InVivoDependence"
# 3b. Biological-level competition assay (finding F4): the pseudoreplication-
#     corrected mixed model lmer(value ~ is_WRN + (1|guide)) LRT on the KM12
#     competition data — the guide as biological unit. Commits
#     wrn:viab_KM12_bio_lme4:result -> ViabilityDependenceAtBiologicalUnit(WRN,KM12)
#     (P ~ 2.15e-6), the honest counterpart of the published nested-ANOVA warrant
#     (P = 2.74e-19, recomputed by wrn:viab_KM12_plan in the statistics layer).
echo "  3b. KM12 competition biological-unit lme4 -> ViabilityDependenceAtBiologicalUnit (F4)"
run_r_program "$PROGRAMS/km12-competition-lme4-program.json" \
    "$PROGRAMS/km12-competition-input.json" "lrt_p_value|ViabilityDependenceAtBiologicalUnit"
echo

# Step 4: the reasoning layers that cite the recomputed + wrapped-R warrants.
# wrn-phase1-biological-sap.esl cites the 3b warrant (concl_viab_KM12_biological)
# and records the F4 dual-SAP fact — loaded here, after 3b committed its witness.
echo "--- Step 4: Load WRN reasoning chain (biological-SAP, phase2, phase3, phase5) ---"
eig load "$WRN/wrn-phase1-biological-sap.esl"
eig load "$WRN/wrn-phase2.esl"
eig load "$WRN/wrn-phase3.esl"
eig load "$WRN/wrn-phase5.esl"
echo

# Step 5: show every WRN verdict — all should be Holds.
echo "--- Step 5: WRN verdicts (expect all Holds) ---"
eig query 'MATCH "urn:eigenius:institution:Verdict"(?v) {
             "urn:eigenius:institution:verdict_subject": ?s,
             "urn:eigenius:core:ctor_name": ?c
           } RETURN [] { subject: ?s, verdict: ?c }'
echo
echo "=== Demo complete ==="
