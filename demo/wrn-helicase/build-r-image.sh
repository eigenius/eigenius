#!/usr/bin/env bash
#
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
#
# Build the R / Bioconductor worker image the WRN demo dispatches its wrapped-R
# warrants to (D55/D56). This is the committed, reviewable counterpart of the
# ad-hoc `eigenius env build --language r` step: the R package set is declared
# EXPLICITLY here (below) and passed through `--r-package`, rather than left
# implicit in the compiled `RImagePlan::default` in crates/eigenius-r/src/dockerfile.rs.
#
# The packages each back a specific WRN warrant:
#   limma    — moderated-t differential dependency (D-DIFF: Achilles/DRIVE/GDSC)
#              and the limma-voom DE step feeding GSEA
#   fgsea    — Hallmark gene-set enrichment (mechanism, ED Fig 3a)
#   lme4     — mixed-model LRTs (xenograft random-slope; KM12 competition
#              biological-unit, finding F4)
#   emmeans  — least-squares-means contrasts (p53 / p21 IF, ED Fig 5)
# (The 53BP1-foci interaction lm and the paralogue co-loss lm use base R `stats`,
#  so they need no extra package.)
#
# IMPORTANT — cross-check constraint (D26 §9.3): the running orchestrator computes
# the worker image's expected manifest from ITS compiled-in `RImagePlan::default`.
# So this explicit list MUST equal that default, or the worker's boot cross-check
# rejects the freshly-built image. The list below is the default, stated openly;
# changing it here also means changing dockerfile.rs and rebuilding the
# orchestrator image. This script's `--verify` step re-reads the built image and
# fails loudly if any declared package is missing.
#
# Usage:
#   ./demo/wrn-helicase/build-r-image.sh            # build, print the digest
#   ./demo/wrn-helicase/build-r-image.sh --verify   # build, then verify packages
#
# Feed the printed digest to the demo:
#   R_IMAGE_DIGEST="$(./demo/wrn-helicase/build-r-image.sh | tail -1)" \
#       ./demo/wrn-helicase/run.sh

set -euo pipefail

# ── The explicit R environment specification ─────────────────────────────────
BIOC_VERSION="3.18"
R_PACKAGES=(limma fgsea lme4 emmeans)
# ─────────────────────────────────────────────────────────────────────────────

VERIFY=0
[ "${1:-}" = "--verify" ] && VERIFY=1

# The R image build runs locally (buildah on the host) and does not talk to the
# kernel, but the `env` command group still requires an endpoint flag. Default to
# the local stack; override with EIGENIUS_ENDPOINT.
ENDPOINT="${EIGENIUS_ENDPOINT:-http://localhost:50051}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_DIR"

echo "Building eigenius CLI + R worker cdylib (one-time)..." >&2
cargo build -q -p eigenius-cli -p eigenius-r-worker --release
EIGENIUS="$REPO_DIR/target/release/eigenius"
CDYLIB="$REPO_DIR/target/release/libeigenius_r_worker.so"
DRIVER="$REPO_DIR/crates/eigenius-r-worker/r/EigeniusRWorker.R"

# Assemble the explicit --r-package flags.
pkg_flags=()
for p in "${R_PACKAGES[@]}"; do pkg_flags+=(--r-package "$p"); done

echo "Building R env image: Bioconductor $BIOC_VERSION + ${R_PACKAGES[*]}" >&2
OUT="$("$EIGENIUS" --endpoint "$ENDPOINT" env build --language r \
    --bioc-version "$BIOC_VERSION" \
    "${pkg_flags[@]}" \
    --r-driver "$DRIVER" \
    --r-cdylib "$CDYLIB" \
    --json)"

DIGEST="$(printf '%s' "$OUT" | sed -n 's/.*"image_digest":"\([^"]*\)".*/\1/p')"
if [ -z "$DIGEST" ]; then
    echo "ERROR: could not parse image_digest from env build output:" >&2
    echo "$OUT" >&2
    exit 1
fi
echo "Built R image: $DIGEST" >&2

if [ "$VERIFY" = 1 ]; then
    echo "Verifying the built image contains the declared packages..." >&2
    rexpr="ok<-suppressWarnings(sapply(c($(printf '"%s",' "${R_PACKAGES[@]}" | sed 's/,$//')), requireNamespace, quietly=TRUE)); if(!all(ok)) { cat('MISSING:', names(ok)[!ok], '\n'); quit(status=1) }; cat('all packages present\n')"
    if docker run --rm --entrypoint Rscript "$DIGEST" -e "$rexpr" >&2; then
        echo "Verified." >&2
    else
        echo "ERROR: built image is missing declared packages." >&2
        exit 1
    fi
fi

# stdout = just the digest, so it can be captured into R_IMAGE_DIGEST.
echo "$DIGEST"
