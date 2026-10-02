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

# Build an HPO↔UMLS-aligned snapshot from a snapshot that carries HPO (obograph-import) on top of the
# WordNet↔UMLS alignment, in ONE step:
#
#   MRCONSO (SAB=HPO) + merges.json ──emit(reads the base chain)──▶  hpo-alignment.esl  ──load──▶  snapshot
#
# The ESL is a build artefact of this run and is never a hand-carried input — for the reason
# scripts/build-alignment-snapshot.sh gives (a stale layer loads cleanly and nothing fails).
#
# Usage:
#   scripts/build-hpo-alignment-snapshot.sh --base <snapshot-dir> --out <new-snapshot-dir> \
#       [--mrconso references/umls/2026AA/META/MRCONSO.RRF] [--merges experiments/lexicon-align/merges.json]
#
# The docker side runs through scripts/add-layer-to-snapshot.sh; set COMPOSE_PROJECT_NAME and
# EIGENIUS_KERNEL_TAG to build in isolation from other checkouts.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BASE=""
OUT=""
MRCONSO="references/umls/2026AA/META/MRCONSO.RRF"
MERGES="experiments/lexicon-align/merges.json"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)    BASE="$2";    shift 2 ;;
    --out)     OUT="$2";     shift 2 ;;
    --mrconso) MRCONSO="$2"; shift 2 ;;
    --merges)  MERGES="$2";  shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$BASE" && -d "$BASE" && -f "$BASE/CURRENT" ]] || {
  echo "error: --base must be a RocksDB snapshot dir (got: '$BASE')" >&2; exit 2; }
[[ -n "$OUT" ]] || { echo "error: --out is required" >&2; exit 2; }
[[ -f "$MRCONSO" ]] || { echo "error: no such file: $MRCONSO" >&2; exit 2; }
[[ -f "$MERGES" ]] || { echo "error: no such merge set: $MERGES" >&2; exit 2; }

say() { echo; echo "=== $* ==="; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ESL="$WORK/hpo-alignment.esl"

# The emitter opens RocksDB read-write, so it reads a COPY; the base is never opened in place.
say "copying the base chain for the emitter to read ($(du -sh "$BASE" | cut -f1))"
cp -a "$BASE" "$WORK/chain"

say "emitting the HPO↔UMLS layer from $(basename "$MRCONSO") (SAB=HPO), leaving $(basename "$MERGES")'s concepts to WordNet"
cargo run --release --features chain --bin lexicon-align-hpo-emit -- \
  --snapshot "$WORK/chain" --mrconso "$MRCONSO" --merges "$MERGES" --out "$ESL"
rm -rf "$WORK/chain"

say "loading the layer onto a fresh copy of the base → $OUT"
scripts/add-layer-to-snapshot.sh --base "$BASE" --out "$OUT" "$ESL"

if [[ -f "$OUT/PROVENANCE" ]]; then
    {
        echo "hpo_umls_alignment : $(basename "$MRCONSO") SAB=HPO, excluding $(basename "$MERGES") (layered by scripts/build-hpo-alignment-snapshot.sh)"
        echo "hpo_aligned_from   : $(basename "$BASE")"
        echo "hpo_aligned_at     : $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    } >> "$OUT/PROVENANCE"
fi

# Next to the merge set for inspection; gitignored, regenerated on every run.
cp "$ESL" experiments/lexicon-align/hpo-alignment.esl
echo "(a copy of the emitted layer is at experiments/lexicon-align/hpo-alignment.esl — inspection only)"
