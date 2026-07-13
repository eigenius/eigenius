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

# Build an ALIGNED snapshot from a base snapshot and a merge set, in ONE step:
#
#   merges.json  ──emit(reads the base chain)──▶  alignment.esl  ──load──▶  aligned snapshot
#
# The ESL is a BUILD ARTEFACT of this run, written to a scratch dir and regenerated every time. It
# is never a hand-carried input.
#
# Why: emit and load used to be two commands in the README with the .esl passed between them by
# hand. On 2026-07-12 merges.json was rebuilt (26 690 → 38 397 merges, the plural surfaces) and the
# load was run WITHOUT re-running the emit. The stale .esl loaded cleanly, the snapshot was named
# `-v3`, and the measurement reported a v2 result under a v3 name. Nothing failed; the wrong thing
# succeeded. Fusing the two steps removes the intermediate a human can forget to refresh.
#
# Usage:
#   scripts/build-alignment-snapshot.sh --base <snapshot-dir> --out <new-snapshot-dir> \
#                                       [--merges experiments/lexicon-align/merges.json]

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BASE=""
OUT=""
MERGES="experiments/lexicon-align/merges.json"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)   BASE="$2";   shift 2 ;;
    --out)    OUT="$2";    shift 2 ;;
    --merges) MERGES="$2"; shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ -n "$BASE" && -d "$BASE" && -f "$BASE/CURRENT" ]] || {
  echo "error: --base must be a RocksDB snapshot dir (got: '$BASE')" >&2; exit 2; }
[[ -n "$OUT" ]] || { echo "error: --out is required" >&2; exit 2; }
[[ -f "$MERGES" ]] || { echo "error: no such merge set: $MERGES" >&2; exit 2; }

say() { echo; echo "=== $* ==="; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ESL="$WORK/alignment.esl"

# The emitter opens RocksDB READ-WRITE — it rewrites CURRENT/MANIFEST/WAL on open. Reading the base
# directly would mutate the very snapshot the aligned one is supposed to be measured against, so it
# reads a throwaway copy.
say "copying the base chain for the emitter to read ($(du -sh "$BASE" | cut -f1))"
cp -a "$BASE" "$WORK/chain"

say "emitting the alignment layer from $MERGES"
cargo run --release --features chain --bin lexicon-align-emit -- \
  --snapshot "$WORK/chain" --merges "$MERGES" --out "$ESL"

echo
echo "  merge rows      : $(grep -c '"cui"' "$MERGES" || true)"
echo "  entries rewritten: $(grep -c '^resource ' "$ESL" || true)"

# The load goes through the kernel: Rule 22, type-checking, the commit gate. A layer that would
# corrupt the chain is rejected here rather than discovered at parse time.
say "loading it onto a fresh copy of the base → $OUT"
scripts/add-layer-to-snapshot.sh --base "$BASE" --out "$OUT" "$ESL"

# Keep the artefact next to the merge set for inspection. It is gitignored and regenerated on every
# run; nothing downstream reads it.
cp "$ESL" experiments/lexicon-align/alignment.esl
echo "(a copy of the emitted layer is at experiments/lexicon-align/alignment.esl — inspection only)"
