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

# Reseed the lexicon database FROM SCRATCH against the CURRENT bootstrap, then snapshot it.
#
# Why this exists: a persisted chain is rooted at the bootstrap it was seeded with (content
# hashes). After editing a bootstrap ontology (ontologies/logic, ontologies/lexicon/closed-class,
# …) the old store can no longer be resumed (ManifestDrift, fail-closed). Pre-production posture is
# drop-and-reseed. This script does exactly that — deterministically, no LLM — and leaves a
# read-only snapshot the in-process harnesses (e.g. the D62 (d) measurement,
# crates/eigenius-wordnet/tests/db_backed_encoding.rs) open via EIGENIUS_DB_SNAPSHOT.
#
# Steps: build release importers/CLI → build the kernel image from HEAD → clean volume →
# bring up kernel (no orchestrator; ingest is deterministic) → convert+load WordNet + UMLS →
# take down → copy the volume to a dated out-of-git snapshot.
#
# Usage:
#   scripts/reseed-lexicon-db.sh                 # WordNet --all + UMLS WRN-relevant subset
#   scripts/reseed-lexicon-db.sh --umls-all      # UMLS all semantic types (large; ~prior 1.9 GB store)
#   scripts/reseed-lexicon-db.sh --no-build      # skip the kernel image rebuild (image already matches HEAD)
#   scripts/reseed-lexicon-db.sh --snapshot-dir /path/to/dir
#
# Env overrides:
#   ENDPOINT       kernel gRPC endpoint to load into (default: 127.0.0.1:50051)
#   VOLUME         docker volume name (default: eigenius_eigenius_db — compose project "eigenius")
#   SNAPSHOT_ROOT  parent dir for snapshots (default: ../db-snapshot relative to repo root)
#   CARGO_PROFILE_IMG  kernel image build profile (default: ci — functionally identical, faster than release)
#
# Prerequisites (NOT provisioned here; both are gitignored, licensed/large):
#   - WordNet 3.0 dict at references/WordNet-3.0/dict   (scripts/provision-wordnet.sh downloads it)
#   - UMLS Level-0 META at references/umls/<release>/META (your own UMLS license; see provision-umls.sh)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENDPOINT="${ENDPOINT:-127.0.0.1:50051}"
VOLUME="${VOLUME:-eigenius_eigenius_db}"
SNAPSHOT_ROOT="${SNAPSHOT_ROOT:-$ROOT/../db-snapshot}"
CARGO_PROFILE_IMG="${CARGO_PROFILE_IMG:-ci}"
UMLS_RELEASE="${RELEASE:-2026AA}"
UMLS_META="references/umls/${UMLS_RELEASE}/META"
DICT="references/WordNet-3.0/dict"

UMLS_ALL=0
BUILD_IMAGE=1
SNAPSHOT_DIR=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --umls-all)     UMLS_ALL=1; shift ;;
    --no-build)     BUILD_IMAGE=0; shift ;;
    --snapshot-dir) SNAPSHOT_DIR="$2"; shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$SNAPSHOT_DIR" ]] || SNAPSHOT_DIR="$SNAPSHOT_ROOT/wordnet-umls-$(date +%Y-%m-%d)"

# WRN-relevant UMLS semantic types (mirrors scripts/provision-umls.sh DEFAULT_TUIS): Disease,
# Cell/Molecular Dysfunction, Neoplastic Process, Gene or Genome, Diagnostic Procedure,
# Pharmacologic Substance, Enzyme, Amino Acid/Peptide/Protein.
UMLS_TUIS=(T047 T049 T191 T028 T060 T121 T126 T116)

say() { echo -e "\n=== $* ==="; }

# ── prerequisites ─────────────────────────────────────────────────────
[[ -f "$DICT/data.noun" ]] || { echo "error: WordNet dict missing at $DICT (run scripts/provision-wordnet.sh)" >&2; exit 1; }
[[ -f "$UMLS_META/MRCONSO.RRF" ]] || { echo "error: UMLS META missing at $UMLS_META (see scripts/provision-umls.sh)" >&2; exit 1; }

# ── 1. fresh release binaries (importers + CLI) ───────────────────────
# Build EXPLICITLY in release: a `cargo run` can silently reuse a stale binary after a branch
# switch, and the UMLS importer's semantic-type-class emission was such a stale-binary trap.
say "building release importers + CLI"
cargo build --release -p eigenius-cli -p eigenius-wordnet -p eigenius-umls
CLI="$ROOT/target/release/eigenius"

# ── 2. kernel image from HEAD (so the seeded bootstrap matches the code that runs the test) ──
if [[ "$BUILD_IMAGE" == "1" ]]; then
  say "building kernel docker image from HEAD ($(git rev-parse --short HEAD), profile=$CARGO_PROFILE_IMG)"
  docker compose build --build-arg CARGO_PROFILE="$CARGO_PROFILE_IMG" kernel
fi

# ── 3. clean volume + bring up kernel alone (orchestrator not needed for deterministic ingest) ──
say "tearing down + dropping the volume for a clean seed"
docker compose down 2>/dev/null || true
docker volume rm "$VOLUME" 2>/dev/null || echo "(volume $VOLUME already absent)"

say "bringing up kernel on a clean volume"
docker compose up -d --no-deps kernel

say "waiting for kernel health"
until [[ "$(docker inspect -f '{{.State.Health.Status}}' eigenius-kernel-1 2>/dev/null)" == "healthy" ]]; do
  [[ "$(docker inspect -f '{{.State.Status}}' eigenius-kernel-1 2>/dev/null)" == "exited" ]] && {
    echo "error: kernel exited before becoming healthy" >&2; docker logs --tail 30 eigenius-kernel-1; exit 1; }
  sleep 3
done
echo "kernel healthy @ $ENDPOINT"

# ── 4. convert (release importers) ────────────────────────────────────
# Countability lexicon (D62 bare-mass args): if present, the importer mass-marks uncountable
# nouns so bare singulars ("lethality matters") shift to NP arguments. Provisioned separately
# (scripts/provision-countability.sh); absent ⇒ count-only nouns (non-fatal).
COUNTABILITY="${COUNTABILITY:-references/wiktionary/uncountable-nouns.txt}"
[[ -f "$COUNTABILITY" ]] || say "note: $COUNTABILITY absent — WordNet nouns will be count-only (run scripts/provision-countability.sh)"
say "converting WordNet (--all) → wordnet-chain/"
rm -rf wordnet-chain
cargo run --release -q -p eigenius-wordnet --bin wordnet-import -- --all --dict "$DICT" --countability "$COUNTABILITY" --out-dir wordnet-chain

say "converting UMLS → umls-chain/  ($([[ $UMLS_ALL == 1 ]] && echo 'all semantic types' || echo "TUIs: ${UMLS_TUIS[*]}"))"
rm -rf umls-chain
UMLS_TUI_ARGS=()
[[ "$UMLS_ALL" == "1" ]] || for t in "${UMLS_TUIS[@]}"; do UMLS_TUI_ARGS+=(--semantic-type "$t"); done
"$ROOT/target/release/umls-import" --meta-dir "$UMLS_META" --version "$UMLS_RELEASE" \
  --out-dir umls-chain "${UMLS_TUI_ARGS[@]}"

# Guard: the base layer must declare EVERY semantic type the concept chunks reference, else the
# kernel rejects the chunks (UnresolvedClassReference, fail-closed). This catches the dangling-STY
# regression directly, before a long load.
base_sty=$(grep -c '^class umlssty:' umls-chain/umls-000-base.esl)
ref_sty=$(grep -ohE 'umlssty:T[0-9]+' umls-chain/umls-[0-9][0-9][0-9].esl | grep -v '000-base' | sort -u | wc -l)
echo "UMLS semantic types: base declares $base_sty, concepts reference $ref_sty"
[[ "$base_sty" -ge "$ref_sty" ]] || { echo "error: $((ref_sty - base_sty)) dangling semantic types — base layer incomplete; aborting before load" >&2; exit 1; }

# ── 5. load both chains in order (release CLI; chain = validation context) ──
load_chain() {
  local label="$1"; shift
  for f in "$@"; do
    echo ">> [$label] load $f"
    "$CLI" --endpoint "http://$ENDPOINT" load "$f"
  done
}
say "loading WordNet chain"
load_chain wordnet wordnet-chain/wordnet-*.esl
say "loading UMLS chain"
load_chain umls umls-chain/umls-*.esl

# ── 6. take down + snapshot the volume (read a copy; never the live volume) ──
say "taking the stack down"
docker compose down

say "snapshotting the volume → $SNAPSHOT_DIR"
# Replace, don't merge: a stale snapshot in the same dir would leave orphan SST files
# (RocksDB ignores them via CURRENT/MANIFEST, but they bloat the copy and confuse).
rm -rf "$SNAPSHOT_DIR"
mkdir -p "$SNAPSHOT_DIR"
docker run --rm -v "$VOLUME":/src:ro -v "$SNAPSHOT_DIR":/dst alpine \
  sh -c "cp -a /src/. /dst/ && chown -R $(id -u):$(id -g) /dst"

echo
echo "================================================================"
echo "reseed complete. snapshot: $SNAPSHOT_DIR"
echo "size: $(du -sh "$SNAPSHOT_DIR" | cut -f1)"
echo "run the (d) measurement against it with:"
echo "  EIGENIUS_DB_SNAPSHOT=$SNAPSHOT_DIR \\"
echo "    cargo test -p eigenius-wordnet --test db_backed_encoding \\"
echo "    wrn_first_page_over_full_lexicon -- --ignored --nocapture"
echo "================================================================"
