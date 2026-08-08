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
#   scripts/reseed-lexicon-db.sh --snapshot-dir /path/to/dir   # a path
#   scripts/reseed-lexicon-db.sh --snapshot-dir my-snap-name   # a bare NAME → $SNAPSHOT_ROOT/my-snap-name
#
# Either form is resolved to an ABSOLUTE path before it reaches `docker -v`. That matters: docker
# treats a non-absolute `-v` source as a NAMED VOLUME, not a bind mount, so the store would land in
# a docker volume while the local directory stayed empty. The snapshot copy is verified afterwards
# (CURRENT present, size sane) and the script FAILS rather than reporting success on an empty dir.
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
# Bytes of ESL per concept chunk = bytes per LAYER COMMIT. Empty ⇒ leave the importer's own default
# (100 MiB, sized against the kernel's 128 MiB gRPC Load limit).
#
# **Do not reach for this to control memory.** It was added on 2026-08-03 on the theory that peak RSS
# was a per-commit transient, so smaller chunks would cap it. Measured (WordNet chain only, 1 s
# sampling, 15 s idle after every commit — `docs/notes/2026-08-03-reseed-memory-profile.md`):
#
#   chunk 001 (100 MiB) → 6.7 GB ; idle 15 s → NO release
#   chunk 002 (101 MiB) → 15.4 GB ; idle 15 s → NO release
#   all loads finished, container idle 1 min later → still 15.39 GB
#
# Memory is RETAINED per layer, not transient, so total resident tracks total data loaded rather than
# chunk size. Worse, chunk 002 cost more than 001 on both axes (+8.7 GB vs +6.7 GB; 111 s vs 55 s)
# for the same bytes, so there is a chain-DEPTH term too — and smaller chunks mean MORE layers. This
# knob most likely makes the problem worse. It stays only because chunk size is a legitimate thing to
# vary when the gRPC limit or a partial-load retry demands it.
SPLIT_BYTES="${SPLIT_BYTES:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --umls-all)     UMLS_ALL=1; shift ;;
    --no-build)     BUILD_IMAGE=0; shift ;;
    --snapshot-dir) SNAPSHOT_DIR="$2"; shift 2 ;;
    --split-bytes)  SPLIT_BYTES="$2"; shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$SNAPSHOT_DIR" ]] || SNAPSHOT_DIR="$SNAPSHOT_ROOT/wordnet-umls-$(date +%Y-%m-%d)"
# A bare NAME (no `/`) means "under SNAPSHOT_ROOT", matching the default's shape. Anything with a
# `/` is a path. Either way SNAPSHOT_DIR ends up ABSOLUTE before it reaches `docker -v`, and that is
# load-bearing: `docker run -v <relative-or-bare>:/dst` does NOT bind-mount, it creates a docker
# NAMED VOLUME. The 3 GB store then lands in that volume, the local directory stays empty, and the
# run still looks like it worked. Verified failure mode, 2026-07-27.
[[ "$SNAPSHOT_DIR" == */* ]] || SNAPSHOT_DIR="$SNAPSHOT_ROOT/$SNAPSHOT_DIR"
mkdir -p "$(dirname "$SNAPSHOT_DIR")"
SNAPSHOT_DIR="$(cd "$(dirname "$SNAPSHOT_DIR")" && pwd)/$(basename "$SNAPSHOT_DIR")"

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
# Junk-atom drop set (D63 alignment): committed by `lexicon-align drops`; absent ⇒ no drops (non-fatal).
DROPS="${DROPS:-experiments/lexicon-align/drops.json}"
[[ -f "$DROPS" ]] || say "note: $DROPS absent — no junk-atom drops (run: lexicon-align drops)"
say "converting WordNet (--all) → wordnet-chain/"
rm -rf wordnet-chain
cargo run --release -q -p eigenius-wordnet --bin wordnet-import -- --all --dict "$DICT" --countability "$COUNTABILITY" --out-dir wordnet-chain

say "converting UMLS → umls-chain/  ($([[ $UMLS_ALL == 1 ]] && echo 'all semantic types' || echo "TUIs: ${UMLS_TUIS[*]}"))"
rm -rf umls-chain
UMLS_TUI_ARGS=()
[[ "$UMLS_ALL" == "1" ]] || for t in "${UMLS_TUIS[@]}"; do UMLS_TUI_ARGS+=(--semantic-type "$t"); done
# `--countability`: mass-shim a concept whose preferred-name head is uncountable (RC-1 head-inheritance,
# d63-parse-gap-closure §4 Step 4) so a bare abbreviation of a mass phenomenon (`MSI`) parses as a subject.
UMLS_COUNTABILITY_ARGS=()
[[ -f "$COUNTABILITY" ]] && UMLS_COUNTABILITY_ARGS+=(--countability "$COUNTABILITY")
# `--drop-atoms`: skip junk atoms whose only contribution is a case-mangled collision with a common
# word (`gENE`→`gene`), judged a different concept by the D63 adjudicator (`lexicon-align drops`). The
# common word stays covered by WordNet (every dropped surface is a WordNet lemma); this only removes
# the spurious biomedical reading of it.
UMLS_DROP_ARGS=()
[[ -f "$DROPS" ]] && UMLS_DROP_ARGS+=(--drop-atoms "$DROPS")
# `--drop-chv-redundant` (A2, D63): drop each concept's redundant multiword CHV-only alias (a compound
# surface only CHV gives it, already covered by an authoritative source) — removes a spurious second
# concept-reading of a compound; coverage-safe. Opt-out with DROP_CHV_REDUNDANT=0.
UMLS_CHV_ARGS=()
[[ "${DROP_CHV_REDUNDANT:-1}" == "1" ]] && UMLS_CHV_ARGS+=(--drop-chv-redundant)
UMLS_SPLIT_ARGS=()
[[ -n "$SPLIT_BYTES" ]] && UMLS_SPLIT_ARGS=(--split-bytes "$SPLIT_BYTES")
"$ROOT/target/release/umls-import" --meta-dir "$UMLS_META" --version "$UMLS_RELEASE" \
  --out-dir umls-chain "${UMLS_SPLIT_ARGS[@]}" \
  "${UMLS_TUI_ARGS[@]}" "${UMLS_COUNTABILITY_ARGS[@]}" "${UMLS_DROP_ARGS[@]}" "${UMLS_CHV_ARGS[@]}"

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

# ── VERIFY the copy. A reseed that reports success on an empty directory is worse than one that
# fails: the emptiness is discovered later, by a measurement that silently used the wrong store.
# `du` showing kilobytes after copying gigabytes is a hard error, not a line of output.
SNAP_BYTES="$(du -sb "$SNAPSHOT_DIR" | cut -f1)"
MIN_BYTES=$((512 * 1024 * 1024))
if [[ ! -f "$SNAPSHOT_DIR/CURRENT" ]] || (( SNAP_BYTES < MIN_BYTES )); then
  echo >&2
  echo "error: the snapshot copy produced nothing usable." >&2
  echo "  dir    : $SNAPSHOT_DIR" >&2
  echo "  bytes  : $SNAP_BYTES (expected >= $MIN_BYTES)" >&2
  echo "  CURRENT: $([[ -f "$SNAPSHOT_DIR/CURRENT" ]] && echo present || echo MISSING)" >&2
  echo >&2
  echo "  The seeded store is still in docker volume '$VOLUME' — it is NOT lost. Recover with:" >&2
  echo "    docker run --rm -v $VOLUME:/src:ro -v $SNAPSHOT_DIR:/dst alpine \\" >&2
  echo "      sh -c 'cp -a /src/. /dst/ && chown -R $(id -u):$(id -g) /dst'" >&2
  echo >&2
  echo "  Check also for a stray docker NAMED VOLUME with the snapshot's name — that is what" >&2
  echo "  'docker -v' creates when handed a non-absolute path: docker volume ls" >&2
  exit 1
fi

echo
echo "================================================================"
echo "reseed complete. snapshot: $SNAPSHOT_DIR"
printf "size: %s (%.2f GiB / %.2f GB, %s files)\n" \
  "$SNAP_BYTES" \
  "$(echo "scale=4; $SNAP_BYTES/1073741824" | bc)" \
  "$(echo "scale=4; $SNAP_BYTES/1000000000" | bc)" \
  "$(ls -1 "$SNAPSHOT_DIR" | wc -l)"
echo "run the (d) measurement against it with:"
echo "  EIGENIUS_DB_SNAPSHOT=$SNAPSHOT_DIR scripts/measure-parse-rate.sh"
echo
echo "  (measure-parse-rate.sh builds RELEASE and enables the reranker. Do NOT hand-roll a"
echo "   'cargo test' invocation: without --release, NbE readback overflows the stack and the"
echo "   harness reports phantom GRAMMAR-GAPs; without --features use-llm it runs cap-only,"
echo "   which inflates gaps by construction.)"
echo "================================================================"
