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
# Three structural points this demo exercises:
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
#   3. Large-data wrapped-R over off-chain inputs (D53 + multi-input D56,
#      Steps 3c/3d). The headline D-DIFF call — WRN is the top MSI-vs-MSS
#      differential dependency — runs limma moderated-t over the 187 MB
#      Achilles CERES matrix tracked as a content-addressed PinnedExternalFile
#      (D53), joined to MSI labels across two more pinned files via the
#      multi-input RunRuntimeScript path. Reproduces the paper's Q = 4.8e-24
#      (WRN rank 1) and lifts D-DIFF from linked-external to reproduced-external.
#      Step 3d exercises the same machinery for the transcriptional mechanism:
#      WRN-KO RNA-seq through limma-voom -> fgsea against the Hallmark .gmt,
#      pinned under the D53 Collection layout profile (ragged gene-set rows) and
#      carried as a second runtime:additional_inputs file.
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

# Step 1: ontology deps. reasoning + statistics + reference are seeded in the
# kernel image; bench-core + onco carry the domain vocabulary (bench:Measurement,
# onco:Gene / onco:InVivoDependence / ...) the WRN chain + the
# canonical_proposition ConstRefs resolve against. wrn-literature carries the
# bibliographic References + the imported-claim warrants (reference:Citation)
# that the structure-function rules [14] and the seed-control rule [16] compose
# as premises, so it must load before the phases that cite it.
echo "--- Step 1: Load ontology deps (bench-core, onco, wrn-literature) ---"
eig load "$REPO_DIR/experiments/benchmark/base-ontologies/bench-core.esl"
eig load "$WRN/chain/01-onco.esl"
eig load "$WRN/chain/02-literature.esl"
echo

# Step 2: recompute layers, then the narrative on top. Order matters and
# matches the validated stack (wrn_phase1_recompute.rs:
# onco -> recompute-plans -> recompute-conclusions -> wrn-phase1):
#   - PLANS (emitters) first — AutoOnLoad commits the StatisticalAnalysisResult
#     IsDerivedAs witnesses into this layer;
#   - CONCLUSIONS (consumers) next — the concl_*_recomputed sentences gate
#     against those now-ancestor witnesses;
#   - wrn-phase1 (narrative) LAST — it stacks ON TOP and its TaskOutput cites
#     the recomputed conclusions, so they must already be ancestors.
echo "--- Step 2: Load WRN recompute (plans -> conclusions) + narrative on top ---"
eig load "$WRN/chain/03-phase1-recompute-plans.esl"
eig load "$WRN/chain/04-phase1-recompute-conclusions.esl"
eig load "$WRN/chain/05-phase1-discovery.esl"
echo

# Step 3: the wrapped-R in-vivo warrant (D55/D56). Run the lme4 xenograft
# program; the substrate spawns the R worker container, runs the authors'
# random-slope LRT, and commits wrn:vivo_lme4:result (carrying the
# InVivoDependence proposition) under a ProgramTrace -> IsDerivedAs witness.
#
# Each program's runtime:image_digest must point at the R worker image (which
# bakes limma/fgsea/lme4/emmeans + the worker cdylib/driver). The baked digests
# are environment-specific AND go stale whenever the R worker crate changes (the
# boot cross-check, D26 §9.3, then refuses the old image). So rebuild with the
# committed build script (which declares the R packages explicitly) and pass the
# fresh digest to patch ALL programs for this run:
#   R_IMAGE_DIGEST="$(./demo/wrn-helicase/build-r-image.sh)" ./demo/wrn-helicase/run.sh
# run_r_program sed-patches runtime:image_digest from $R_IMAGE_DIGEST when set —
# covering xenograft, km12, and dd-achilles alike.
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
run_r_program "$PROGRAMS/invivo/xenograft-lme4-program.json" \
    "$PROGRAMS/invivo/xenograft-input.json" "lrt_p_value|InVivoDependence"
# 3b. Biological-level competition assay (finding F4): the pseudoreplication-
#     corrected mixed model lmer(value ~ is_WRN + (1|guide)) LRT on the KM12
#     competition data — the guide as biological unit. Commits
#     wrn:viab_KM12_bio_lme4:result -> ViabilityDependenceAtBiologicalUnit(WRN,KM12)
#     (P ~ 2.15e-6), the honest counterpart of the published nested-ANOVA warrant
#     (P = 2.74e-19, recomputed by wrn:viab_KM12_plan in the statistics layer).
echo "  3b. KM12 competition biological-unit lme4 -> ViabilityDependenceAtBiologicalUnit (F4)"
run_r_program "$PROGRAMS/invivo/km12-competition-lme4-program.json" \
    "$PROGRAMS/invivo/km12-competition-input.json" "lrt_p_value|ViabilityDependenceAtBiologicalUnit"

# 3c. D-DIFF (Achilles): the headline genome-wide differential dependency,
#     reproduced via limma moderated-t (D56 wrapped-R) over the 187 MB CERES
#     matrix pinned as a D53 PinnedExternalFile, joined to MSI labels across two
#     more pinned files through the MULTI-INPUT RunRuntimeScript path
#     (runtime:additional_inputs = sample_info bridge + Supp Table 1). Commits
#     wrn:dd_achilles:result -> TopDifferentialDependency(WRN,Achilles_MSI)
#     (WRN rank 1, Q = 4.81e-24, matching the paper's 4.8e-24) under a
#     ProgramTrace -> IsDerivedAs witness; lifts D-DIFF to reproduced-external.
#
#     Heavier than 3a/3b: the dependency matrices are gitignored (data/slices/,
#     ~235 MB). When present, content-address them, stage them into the depot's
#     extfile-cache (the DooD-shared mount the orchestrator + sibling R worker
#     both see), commit the two auxiliary PinnedExternalFile nodes, then run.
SLICES="$WRN/data/slices"
if [ -f "$SLICES/achilles_18Q4_gene_effect.csv" ]; then
    echo "  3c. D-DIFF limma (Achilles, 187 MB matrix) -> TopDifferentialDependency"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    # Stage each input into the content-addressed cache: <cache>/<sha256-hex>/<basename>.
    # Hashes are the pinned MANIFEST.md content addresses (== the PinnedExternalFile IRIs).
    for f in achilles_18Q4_gene_effect.csv:2186669de8ade17bfbf7f2bc3e67e8af59d644800bf793ef103c67a4692eb68b \
             achilles_18Q4_sample_info.csv:c5778e66e6c62c94386a39924be50f24086d5f0d5401117b065c3e6d7fbdb498 \
             wrn_supplementary_table_1.csv:eebd460257982a98cf6ce9f14e189ae0c4398a686f4181bc037c5591e87243f2; do
        name="${f%%:*}"; hex="${f##*:}"
        docker exec "$ORCH" mkdir -p "$CACHE/$hex"
        docker cp "$SLICES/$name" "$ORCH:$CACHE/$hex/$name"
    done
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # sample_info + supp1 (the additional_inputs)
    run_r_program "$PROGRAMS/differential-dependency/dd-achilles-limma-program.json" \
        "$PROGRAMS/differential-dependency/dd-achilles-input.json" "adj_p_value|differential_rank|TopDifferentialDependency"
else
    echo "  3c. D-DIFF limma -> SKIPPED (data/slices/ not vended; see data/MANIFEST.md)"
fi

# 3d. C-MECH (GSEA, ED Fig 3a): the transcriptional corroboration of cell-cycle
#     arrest. WRN-KO RNA-seq (GEO GSE126464 STAR counts, genes x 12 samples,
#     pinned + gzipped) is run through limma-voom -> moderated-t, ranked, and
#     fed to fgsea against the MSigDB Hallmark .gmt (a D53 Collection-profile
#     PinnedExternalFile carried as runtime:additional_inputs). Commits
#     wrn:gsea_mech:result -> CausesCellCycleArrest(WRN,MSI): G2M/E2F depleted
#     (NES -3.5 / -3.4, padj ~1e-49), p53 response activated (NES +2.9,
#     padj ~1e-20), apoptosis up -- matching the paper's Fig 3a panel.
if [ -f "$SLICES/GSE126464_STAR_Gene_Counts.csv.gz" ] && [ -f "$SLICES/h.all.v6.2.symbols.gmt" ]; then
    echo "  3d. C-MECH GSEA limma-voom + fgsea (GSE126464 vs Hallmark) -> CausesCellCycleArrest"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    for f in GSE126464_STAR_Gene_Counts.csv.gz:e66c70f32079bc137b6e3849fc71d5cf0d3e42caed3e8b8e7be4a3bc4876daa5 \
             h.all.v6.2.symbols.gmt:0ee07a4abda7bba49e6cf4ce773f1c5a57b24726cd6c6662f01cabe31e22146b; do
        name="${f%%:*}"; hex="${f##*:}"
        docker exec "$ORCH" mkdir -p "$CACHE/$hex"
        docker cp "$SLICES/$name" "$ORCH:$CACHE/$hex/$name"
    done
    eig load "$PROGRAMS/mechanism/gsea-mech-files.json"   # Hallmark .gmt + Collection schema (the additional_input)
    run_r_program "$PROGRAMS/mechanism/gsea-mech-program.json" \
        "$PROGRAMS/mechanism/gsea-mech-input.json" "nes_|padj_|CausesCellCycleArrest"
else
    echo "  3d. C-MECH GSEA -> SKIPPED (data/slices/ not vended; see data/MANIFEST.md)"
fi

# 3e. D-DIFF family — DRIVE (RNAi/DEMETER2, ED Fig 1b): the orthogonal-screen
#     replication. Same limma D-DIFF program shape as 3c, run over the 59 MB
#     DRIVE dependency matrix (genes x cell-lines; columns ARE CCLE_IDs, so the
#     MSI join is direct to Supp Table 1 — no sample_info bridge). Commits
#     wrn:dd_drive:result -> TopDifferentialDependency(WRN,DRIVE_MSI) (WRN rank 1,
#     Q = 1.46e-45, matching the paper's 1.5e-45) — WRN is #1 in BOTH the CRISPR
#     (Achilles) and RNAi (DRIVE) screens.
if [ -f "$SLICES/drive_D2_DRIVE_gene_dep_scores.csv" ]; then
    echo "  3e. D-DIFF DRIVE limma (RNAi, 59 MB matrix) -> TopDifferentialDependency(DRIVE)"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    for f in drive_D2_DRIVE_gene_dep_scores.csv:3f863c296188be1aa8a491ef5489b135a9bfd65266f05d0690225d20fc38254b \
             wrn_supplementary_table_1.csv:eebd460257982a98cf6ce9f14e189ae0c4398a686f4181bc037c5591e87243f2; do
        name="${f%%:*}"; hex="${f##*:}"
        docker exec "$ORCH" mkdir -p "$CACHE/$hex"
        docker cp "$SLICES/$name" "$ORCH:$CACHE/$hex/$name"
    done
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 (the additional_input; sample_info unused here)
    eig load "$PROGRAMS/differential-dependency/dd-drive-input.json"
    run_r_program "$PROGRAMS/differential-dependency/dd-drive-limma-program.json" \
        "$PROGRAMS/differential-dependency/dd-drive-input.json" "adj_p_value|differential_rank|TopDifferentialDependency"
else
    echo "  3e. D-DIFF DRIVE -> SKIPPED (data/slices/ not vended; see data/MANIFEST.md)"
fi

# 3f. D-DIFF family — GDSC PCR-MSI robustness (ED Fig 1b). Re-runs the Achilles
#     D-DIFF (3c) but groups by the orthogonal GDSC PCR MSI panel (MSI-H vs
#     MSS/MSI-L, only 19 MSI-H lines) instead of the NGS CCLE_MSI calls. Same
#     pinned matrix + join; only the label column differs. Commits
#     wrn:dd_gdsc:result -> TopDifferentialDependency(WRN,Achilles_GDSC_MSI)
#     (WRN STILL rank 1, Q = 4.66e-20) — the headline doesn't depend on the MSI
#     calling method. Reuses 3c's staged matrix + the supp1/sample_info nodes.
if [ -f "$SLICES/achilles_18Q4_gene_effect.csv" ]; then
    echo "  3f. D-DIFF GDSC PCR-MSI robustness (Achilles matrix) -> TopDifferentialDependency(GDSC)"
    run_r_program "$PROGRAMS/differential-dependency/dd-gdsc-limma-program.json" \
        "$PROGRAMS/differential-dependency/dd-achilles-input.json" "adj_p_value|differential_rank|TopDifferentialDependency"
else
    echo "  3f. D-DIFF GDSC robustness -> SKIPPED (data/slices/ not vended; see data/MANIFEST.md)"
fi

# 3g. C-MECH p53 activation (ED Fig 5b/d/f): the IF least-squares-means warrant.
#     Per-cell phospho-p53(S15)/p21 staining intensity (175,974 cells, a D53
#     file-backed SampleSet with a LongTable schema) through a D56 wrapped-R
#     emmeans contrast (WRN-KO vs control on log-intensity, adjusting for
#     cell_line). The genotype is joined from Supp Table 1 (additional input):
#     the contrast is recomputed over the MSI + TP53-proficient stratum, where
#     p-p53 (logFC +0.155) and p21 (+0.310) both rise. Commits
#     wrn:if_ed5:result -> ActivatesP53Response(WRN,MSI). The p53-null MSI line
#     (KM12) fails to induce p21 (p21_null_logfc < 0) — the p53-independence
#     control emitted as a measurement, NOT a failed warrant (finding F7).
#     The slice is derived from wrn_sourcedata_EDFig5_MOESM8.xlsx by
#     extract/if-ed5-extract.R (run once in data/slices/).
if [ -f "$SLICES/if_ed5_long.csv" ]; then
    echo "  3g. C-MECH p53 IF emmeans lsmeans (175k cells) -> ActivatesP53Response"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    HEX=8d26fbb8aafb610a4952c6281b4088b41bb1ffc6d3318b16c7f7ca164c86c519
    docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
    docker cp "$SLICES/if_ed5_long.csv" "$ORCH:$CACHE/$HEX/if_ed5_long.csv"
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 genotype (the additional_input)
    eig load "$PROGRAMS/mechanism/if-ed5-files.json"        # LongTable DatasetSchema
    eig load "$PROGRAMS/mechanism/if-ed5-input.json"        # IF PinnedExternalFile node
    run_r_program "$PROGRAMS/mechanism/if-ed5-lsmeans-program.json" \
        "$PROGRAMS/mechanism/if-ed5-input.json" "logfc|p_value|RaisesP53DamageMarkers"
else
    echo "  3g. C-MECH p53 IF emmeans -> SKIPPED (derived slice not present; see extract/if-ed5-extract.R)"
fi

# 3h. C-MECH DSB induction (ED Fig 6f/6h): the 53BP1 DSB-foci warrant. Per-cell
#     Apple-53BP1-trunc foci counts (39,249 cells across MSS SW620/ES2 + MSI
#     KM12/OVK18, a D53 file-backed SampleSet) through a D56 wrapped-R lm
#     foci ~ cell_line + condition*MSI. The condition×MSI INTERACTION is the
#     MSI-selective extra DSB induction: WRN-KO ~2.08x foci in MSI vs ~1.04x in
#     MSS (interaction +1.82, p ~ 2.6e-142). Commits wrn:foci_dsb:result ->
#     CausesDSBs(WRN,MSI); reproduced-external corroboration of concl_dsb
#     (concl_dsb_foci). The broader γH2AX/pATM/Chk2 panel stays linked (mech_dsb).
#     Slice derived from wrn_sourcedata_EDFig6_MOESM9.xlsx by foci-ed6-extract.R.
if [ -f "$SLICES/foci_53bp1_long.csv" ]; then
    echo "  3h. C-MECH 53BP1 DSB foci lm (39k cells, MSI-selective) -> CausesDSBs"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    HEX=1ba6dc6f78b10cee9ebc25287cc35170fffc11c357e6f371469395bfc14e9b83
    docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
    docker cp "$SLICES/foci_53bp1_long.csv" "$ORCH:$CACHE/$HEX/foci_53bp1_long.csv"
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 genotype (the additional_input)
    eig load "$PROGRAMS/mechanism/foci-ed6-files.json"      # LongTable DatasetSchema
    eig load "$PROGRAMS/mechanism/foci-ed6-input.json"      # foci PinnedExternalFile node
    run_r_program "$PROGRAMS/mechanism/foci-ed6-program.json" \
        "$PROGRAMS/mechanism/foci-ed6-input.json" "interaction|foci_fc|CausesDSBs"
else
    echo "  3h. C-MECH 53BP1 DSB foci -> SKIPPED (derived slice not present; see extract/foci-ed6-extract.R)"
fi

# 3i. Specificity (ED Fig 9a): paralogue co-loss control over the 1.6 GB DepMap
#     omics bundle — the LARGE multi-schema D53 container path. The authors' dat
#     list (DRIVE/CRISPR/GE/CN/MUT_*/RPPA matrices) is pinned as a single
#     PinnedExternalFile and read in-worker via readRDS; the warrant fits
#     lm(avg_WRN_dep ~ MSI + gene_loss) per RECQ paralogue and emits
#     wrn:paralog_ctrl:result -> NotExplainedByParalogLoss(WRN,MSI): the MSI
#     coefficient stays significant + same-signed (baseline β=-0.667 p=4.4e-60;
#     controlled β≈-0.67..-0.70, worst p≈1e-58) — WRN dependence is intrinsic to
#     MSI, not a paralogue-co-loss confound. Discharged by concl_paralog.
RDS="$WRN/data/large/DepMap_18Q4_data.rds"
if [ -f "$RDS" ]; then
    echo "  3i. Specificity paralogue co-loss lm (1.6 GB DepMap rds) -> NotExplainedByParalogLoss"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    HEX=14e82c398188b9f61ad2255301726551884354e90d1fd4ea612bfe6c709c85ed
    docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
    docker cp "$RDS" "$ORCH:$CACHE/$HEX/DepMap_18Q4_data.rds"
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 (avg_WRN_dep + CCLE_MSI; additional_input)
    eig load "$PROGRAMS/specificity/paralog-ed9a-files.json"  # rds container schema
    eig load "$PROGRAMS/specificity/paralog-ed9a-input.json"  # rds PinnedExternalFile node
    run_r_program "$PROGRAMS/specificity/paralog-ed9a-program.json" \
        "$PROGRAMS/specificity/paralog-ed9a-input.json" "paralog_|NotExplainedByParalogLoss"
else
    echo "  3i. Specificity paralogue co-loss -> SKIPPED (1.6 GB DepMap rds not vended; see data/MANIFEST.md)"
fi

# 3j. C-MECH γH2AX intensity (ED Fig 6c): the canonical DSB-lesion marker, the
#     authors' published quantification. Per-cell nuclear γH2AX staining intensity
#     (32,882 cells, ES2 MSS + OVK18 MSI) through a D56 wrapped-R emmeans
#     interaction on log10-intensity; emits wrn:gh2ax:result -> CausesDSBs(WRN,MSI).
#     Reproduces the paper exactly: mean log10 FC 0.055 ES2 / 0.144 OVK18,
#     MSI-vs-MSS contrast P < 2e-16 (concl_dsb_gh2ax). Slice from ED 6c via
#     extract/gh2ax-ed6c-extract.R.
if [ -f "$SLICES/gh2ax_intensity_long.csv" ]; then
    echo "  3j. C-MECH γH2AX intensity emmeans (32k cells, ED 6c) -> CausesDSBs"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    HEX=d8da9e9535f0f863e8ca541c63982804fa1e0fb9e9a9d3d721650c08ef625f95
    docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
    docker cp "$SLICES/gh2ax_intensity_long.csv" "$ORCH:$CACHE/$HEX/gh2ax_intensity_long.csv"
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 genotype (additional_input)
    eig load "$PROGRAMS/mechanism/gh2ax-intensity-files.json"   # LongTable DatasetSchema
    eig load "$PROGRAMS/mechanism/gh2ax-intensity-input.json"   # γH2AX intensity PinnedExternalFile
    run_r_program "$PROGRAMS/mechanism/gh2ax-intensity-program.json" \
        "$PROGRAMS/mechanism/gh2ax-intensity-input.json" "gh2ax_logfc|gh2ax_interaction|CausesDSBs"
else
    echo "  3j. C-MECH γH2AX intensity -> SKIPPED (derived slice not present; see extract/gh2ax-ed6c-extract.R)"
fi

# 3k. C-MECH γH2AX foci (ED Fig 6a/6d): the discrete-foci leg of the same marker.
#     Per-cell γH2AX foci (94,791 cells, colon + ovarian) through a D56 wrapped-R
#     interaction lm; saturated pan-nuclear cells (MSI-enriched) counted at a
#     ceiling, not dropped. Emits wrn:gh2ax_foci:result -> CausesDSBs(WRN,MSI)
#     (interaction +7.3, foci ×3.4 MSI vs ×1.0 MSS; concl_dsb_gh2ax_foci). Slice
#     from ED 6a/6d via extract/gh2ax-ed6ad-extract.R.
if [ -f "$SLICES/gh2ax_foci_long.csv" ]; then
    echo "  3k. C-MECH γH2AX foci lm (95k cells, ED 6a/6d) -> CausesDSBs"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    HEX=70abbad2f5319ae18ed840cd36bb3b75805ecd5c75f2adf940858e719e4b198c
    docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
    docker cp "$SLICES/gh2ax_foci_long.csv" "$ORCH:$CACHE/$HEX/gh2ax_foci_long.csv"
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 genotype
    eig load "$PROGRAMS/mechanism/gh2ax-foci-files.json"
    eig load "$PROGRAMS/mechanism/gh2ax-foci-input.json"
    run_r_program "$PROGRAMS/mechanism/gh2ax-foci-program.json" \
        "$PROGRAMS/mechanism/gh2ax-foci-input.json" "gh2ax_foci_interaction|gh2ax_foci_fc|CausesDSBs"
else
    echo "  3k. C-MECH γH2AX foci -> SKIPPED (derived slice not present; see extract/gh2ax-ed6ad-extract.R)"
fi

# 3l. C-MECH DDR signaling, pATM(S1981) foci (ED Fig 7b/7d): the DSB-response
#     kinase activation that bridges DSBs to p53. Per-cell pATM(S1981) foci
#     (191,241 cells, colon SW620/KM12/SW48 + ovarian ES2/OVK18) through a D56
#     wrapped-R interaction lm; emits wrn:patm:result -> ActivatesDSBResponse(WRN,
#     MSI) (foci ×1.74 MSI vs ×1.11 MSS, interaction p≈0; concl_ddr_signaling).
#     Slice from ED 7b/7d via extract/patm-ed7-extract.R.
if [ -f "$SLICES/patm_foci_long.csv" ]; then
    echo "  3l. C-MECH pATM(S1981) foci lm (191k cells, ED 7b/7d) -> ActivatesDSBResponse"
    ORCH="$(docker compose ps -q orchestrator 2>/dev/null || true)"
    ORCH="${ORCH:-eigenius-orchestrator-1}"
    CACHE=/var/lib/eigenius/substrate-depot/extfile-cache
    HEX=9a718df80087dece5ca9c36af2cb647f06913e1a4a54b7dd038a60ba2cc325de
    docker exec "$ORCH" mkdir -p "$CACHE/$HEX"
    docker cp "$SLICES/patm_foci_long.csv" "$ORCH:$CACHE/$HEX/patm_foci_long.csv"
    eig load "$PROGRAMS/differential-dependency/dd-achilles-files.json"   # supp1 genotype
    eig load "$PROGRAMS/mechanism/patm-foci-files.json"
    eig load "$PROGRAMS/mechanism/patm-foci-input.json"
    run_r_program "$PROGRAMS/mechanism/patm-foci-program.json" \
        "$PROGRAMS/mechanism/patm-foci-input.json" "patm_msi_interaction|patm_foci_fc|ActivatesDSBResponse"
else
    echo "  3l. C-MECH pATM foci -> SKIPPED (derived slice not present; see extract/patm-ed7-extract.R)"
fi
echo

# Step 4: the reasoning layers that cite the recomputed + wrapped-R warrants.
# wrn-phase1-biological-sap.esl cites the 3b warrant (concl_viab_KM12_biological)
# and records the F4 dual-SAP fact — loaded here, after 3b committed its witness.
echo "--- Step 4: Load WRN reasoning chain (biological-SAP, phase2, phase3, phase5) ---"
eig load "$WRN/chain/06-phase1-biological-sap.esl"
eig load "$WRN/chain/07-phase2-validation.esl"
eig load "$WRN/chain/08-phase3-invivo-mechanism.esl"
eig load "$WRN/chain/09-phase5-synthesis.esl"
echo

# Step 5: show every WRN verdict — all should be Holds.
echo "--- Step 5: WRN verdicts (expect all Holds) ---"
eig query 'MATCH "urn:eigenius:institution:Verdict"(?v) {
             "urn:eigenius:institution:verdict_subject": ?s,
             "urn:eigenius:core:ctor_name": ?c
           } RETURN [] { subject: ?s, verdict: ?c }'
echo
echo "=== Demo complete ==="
