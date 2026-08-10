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

# Add one or more ESL layers ON TOP of an existing snapshot, producing a NEW snapshot.
#
# The base snapshot is treated as IMMUTABLE: it is restored into a scratch docker volume, the layers
# are loaded there through the kernel (so they go through the validator — Rule 22, type-checking, the
# commit gate), and the result is snapshotted to a new directory. The base is never written to.
#
# Why this exists: a reseed (scripts/reseed-lexicon-db.sh) rebuilds the whole store from the source
# corpora (~20 min). Adding a derived layer — e.g. the WordNet↔UMLS alignment (D63,
# docs/notes/d63-wordnet-umls-concept-unification.md) — needs only the base plus the new ESL, and
# must leave the base intact so the two can be measured against each other.
#
# Usage:
#   scripts/add-layer-to-snapshot.sh --base <snapshot-dir> --out <new-snapshot-dir> <layer.esl>...
#
# Env:
#   ENDPOINT   kernel gRPC endpoint (default: 127.0.0.1:50051)
#   VOLUME     docker volume to stage in (default: eigenius_eigenius_db)

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENDPOINT="${ENDPOINT:-127.0.0.1:50051}"
VOLUME="${VOLUME:-eigenius_eigenius_db}"
BASE=""
OUT=""
LAYERS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) BASE="$2"; shift 2 ;;
    --out)  OUT="$2";  shift 2 ;;
    -*) echo "error: unknown flag: $1" >&2; exit 2 ;;
    *)  LAYERS+=("$1"); shift ;;
  esac
done

[[ -n "$BASE" && -d "$BASE" && -f "$BASE/CURRENT" ]] || {
  echo "error: --base must be a RocksDB snapshot dir (got: '$BASE')" >&2; exit 2; }
[[ -n "$OUT" ]] || { echo "error: --out is required" >&2; exit 2; }
[[ ${#LAYERS[@]} -gt 0 ]] || { echo "error: give at least one .esl layer" >&2; exit 2; }
for f in "${LAYERS[@]}"; do
  [[ -f "$f" ]] || { echo "error: no such layer: $f" >&2; exit 2; }
done
[[ "$(readlink -f "$BASE")" != "$(readlink -f "$OUT")" ]] || {
  echo "error: --out must differ from --base; the base snapshot is immutable" >&2; exit 2; }

say() { echo; echo "=== $* ==="; }

CLI="$ROOT/target/release/eigenius"
say "building the release CLI"
cargo build --release -p eigenius-cli

# The KERNEL image too — not just the CLI.
#
# `docker-compose.yml` pins `image: eigenius-kernel:local` beside its `build:` stanza, so
# `docker compose up` reuses whatever was last built and never rebuilds. That silently undermines
# this script's whole premise: the layer below is loaded THROUGH THE KERNEL precisely so it passes
# the current validator, and a stale image validates it against old rules. The result is a
# persistent snapshot containing something today's kernel would reject — discovered later, far from
# here. Building is cheap next to a snapshot copy; not building is a correctness hole.
say "building the kernel image (compose pins a tag, so `up` alone would reuse a stale one)"
docker compose build kernel

say "staging the base snapshot into a clean volume ($VOLUME)"
# The base is copied IN; it is never opened in place. RocksDB takes a store read-write on open and
# rewrites CURRENT/MANIFEST/WAL — a run against the base directly would mutate it (the bug fixed in
# the parse harness on 2026-07-11).
docker compose down 2>/dev/null || true
docker volume rm "$VOLUME" 2>/dev/null || true
docker volume create "$VOLUME" >/dev/null
docker run --rm -v "$(readlink -f "$BASE")":/src:ro -v "$VOLUME":/dst alpine \
  sh -c "cp -a /src/. /dst/"

say "bringing the kernel up on the staged volume"
docker compose up -d --no-deps kernel
until [[ "$(docker inspect -f '{{.State.Health.Status}}' eigenius-kernel-1 2>/dev/null)" == "healthy" ]]; do
  [[ "$(docker inspect -f '{{.State.Status}}' eigenius-kernel-1 2>/dev/null)" == "exited" ]] && {
    echo "error: kernel exited before becoming healthy" >&2
    docker logs --tail 40 eigenius-kernel-1
    exit 1
  }
  sleep 2
done

# Loading through the kernel means the layer goes through the VALIDATOR — Rule 22 (references resolve
# same-or-lower), type-checking, the commit gate. A layer that would corrupt the chain is rejected
# here rather than discovered at parse time.
for f in "${LAYERS[@]}"; do
  say "loading layer: $f  ($(grep -c '^resource ' "$f" || echo '?') resources)"
  "$CLI" --endpoint "http://$ENDPOINT" load "$f"
done

say "taking the stack down"
docker compose down

say "snapshotting → $OUT"
rm -rf "$OUT"
mkdir -p "$OUT"
docker run --rm -v "$VOLUME":/src:ro -v "$(readlink -f "$OUT")":/dst alpine \
  sh -c "cp -a /src/. /dst/ && chown -R $(id -u):$(id -g) /dst"

echo
echo "================================================================"
echo "done. base (untouched): $BASE"
echo "      new snapshot    : $OUT  ($(du -sh "$OUT" | cut -f1))"
echo
echo "measure it against the base with:"
echo "  scripts/measure-parse-rate.sh --snapshot $(readlink -f "$OUT")"
echo "================================================================"
