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

# The FULL four-step store rebuild, which is what a bootstrap-ontology edit costs.
#
#   1  reseed-lexicon-db.sh --umls-all        WordNet + UMLS from source   → wordnet-umls-$D
#   2  build-alignment-snapshot.sh            WordNet↔UMLS merges          → wordnet-umls-aligned-$D
#   3  obograph-import + add-layer            HPO                          → wordnet-umls-hpo-$D
#   4  build-hpo-alignment-snapshot.sh        HPO↔UMLS                     → wordnet-umls-hpo-aligned-$D
#
# WHY THIS FILE EXISTS. The recipe lived only in `docs/notes/work-stack.md` prose and in
# `reseed-logs/chain.sh`, which is OUTSIDE the repo and therefore absent on a fresh checkout — the
# handoff note points at it and it is not there. It was reconstructed by hand twice in one week, and
# a four-step chain reconstructed from prose is exactly the "stale intermediate" failure
# `build-alignment-snapshot.sh` was written to remove. Tracked here so it travels with the branch.
#
# THE THREE PROVENANCE COUNTS are checked, not merely printed: 38,391 merge rows, 20,533 HPO
# resources, 864 equivalences. They are the signal that the rebuilt store is the one the recorded
# numbers were taken on; if any differs, an input changed and measurements are not comparable.
# Override with EXPECT_MERGES / EXPECT_HPO / EXPECT_EQUIV when an input is DELIBERATELY changed.
#
# Usage:
#   scripts/reseed-chain.sh <date-tag>              # e.g. 2026-10-28-advfold
#   SNAPSHOT_ROOT=/path scripts/reseed-chain.sh <date-tag>
#
# Runs in its own compose project so other checkouts' volumes are untouched.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

D="${1:-}"
[[ -n "$D" ]] || { echo "usage: $0 <date-tag>   (e.g. $(date +%Y-%m-%d)-mychange)" >&2; exit 2; }

export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-eigenius-chain}"
export EIGENIUS_KERNEL_TAG="${EIGENIUS_KERNEL_TAG:-chain}"
export EIGENIUS_KERNEL_PORT="${EIGENIUS_KERNEL_PORT:-18061}"
export ENDPOINT="${ENDPOINT:-127.0.0.1:$EIGENIUS_KERNEL_PORT}"

S="${SNAPSHOT_ROOT:-$ROOT/../db-snapshot}"
mkdir -p "$S"
S="$(cd "$S" && pwd)"
HPO_JSON="${HPO_JSON:-references/hpo/hp.json}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

EXPECT_MERGES="${EXPECT_MERGES:-38391}"
EXPECT_HPO="${EXPECT_HPO:-20533}"
EXPECT_EQUIV="${EXPECT_EQUIV:-864}"

step() { echo; echo "##################### $* ($(date +%H:%M:%S))"; }
LOG="$WORK/chain.log"

# `tee` so the counts can be grepped afterwards; PIPESTATUS so a failure in the SCRIPT is not
# masked by tee's success.
run() { "$@" 2>&1 | tee -a "$LOG"; return "${PIPESTATUS[0]}"; }

[[ -f "$HPO_JSON" ]] || { echo "error: no HPO at $HPO_JSON (see docs/notes/work-stack.md)" >&2; exit 1; }

step "1/4 reseed (WordNet --all + UMLS --umls-all)"
run scripts/reseed-lexicon-db.sh --umls-all --snapshot-dir "wordnet-umls-$D"

step "2/4 WordNet<->UMLS alignment"
run scripts/build-alignment-snapshot.sh --base "$S/wordnet-umls-$D" \
  --out "$S/wordnet-umls-aligned-$D" --merges experiments/lexicon-align/merges.json

step "3/4 HPO import + layer"
run cargo run --release -q -p eigenius-obograph --bin obograph-import -- \
  --input "$HPO_JSON" --output "$WORK/hpo.eigon.json" --root-anchor urn:eigenius:lexicon:Entity
run scripts/add-layer-to-snapshot.sh --base "$S/wordnet-umls-aligned-$D" \
  --out "$S/wordnet-umls-hpo-$D" "$WORK/hpo.eigon.json"

step "4/4 HPO<->UMLS alignment"
run scripts/build-hpo-alignment-snapshot.sh --base "$S/wordnet-umls-hpo-$D" \
  --out "$S/wordnet-umls-hpo-aligned-$D"

docker compose down >/dev/null 2>&1 || true

# ── the provenance gate ───────────────────────────────────────────────
step "provenance"
got_merges=$(grep -oE "merge rows *: *[0-9]+" "$LOG" | tail -1 | grep -oE "[0-9]+" || echo 0)
# The HPO load is the `Loaded` line that FOLLOWS its own layer line — not the largest in the log.
# Taking the maximum instead found a 361,101-resource UMLS chunk and reported it as the HPO count,
# which made this check unable to fail. Caught on the 2026-10-28 run.
got_hpo=$(grep -A 2 "hpo.eigon.json" "$LOG" | grep -oE "Loaded [0-9]+ resource" | grep -oE "[0-9]+" | head -1 || echo 0)
got_equiv=$(grep -oE "declared equivalent \(§11\) *: *[0-9]+" "$LOG" | grep -oE "[0-9]+$" || echo 0)
fail=0
chk() { # name got want
  if [[ "$2" == "$3" ]]; then printf "  %-22s %-8s OK\n" "$1" "$2"
  else printf "  %-22s %-8s MISMATCH (expected %s)\n" "$1" "$2" "$3"; fail=1; fi
}
chk "merge rows"      "$got_merges" "$EXPECT_MERGES"
chk "HPO resources"    "$got_hpo"    "$EXPECT_HPO"
chk "HPO equivalences" "$got_equiv" "$EXPECT_EQUIV"

OUT="$S/wordnet-umls-hpo-aligned-$D"
echo
echo "================================================================"
if [[ "$fail" == "1" ]]; then
  echo "PROVENANCE MISMATCH — an input changed. The store is built, but measurements"
  echo "against it are NOT comparable to earlier ones until the difference is explained."
else
  echo "chain complete, provenance reproduced."
fi
echo "store: $OUT  ($(du -sh "$OUT" 2>/dev/null | cut -f1))"
cat "$OUT/PROVENANCE" 2>/dev/null | tail -6
echo "================================================================"
exit "$fail"
