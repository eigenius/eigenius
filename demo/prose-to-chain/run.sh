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
#
# prose-to-chain — a paragraph of the WRN paper, parsed, committed, and then
# edited until the kernel refuses it.
#
# Two branches off one base, differing only in the prose:
#
#   prose-intact   paragraph.txt          → argument commits, ValidateJustification Holds
#   prose-edited   paragraph-edited.txt   → argument REJECTED
#
# The edit deletes one negation ("did not require" → "required") from the second
# sentence. It still parses, and to exactly the same STRUCTURE — the skeleton is
# the helicase sentence's — so nothing syntactic notices. What changes is the
# proposition, and the recorded argument's certificate cites the old one.
#
# Prerequisites:
#   EIGENIUS_MOCK_LLM=true docker compose up -d      (kernel on :50051)
#
# Usage:
#   ./demo/prose-to-chain/run.sh
#   ./demo/prose-to-chain/run.sh --reparse           # re-derive the claims layers from the
#                                                    # lexicon snapshot instead of using the
#                                                    # committed fixtures (needs the snapshot)

set -euo pipefail

ENDPOINT="${EIGENIUS_ENDPOINT:-http://localhost:50051}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WRN="$REPO/experiments/publications/wrn-helicase"
REPARSE=0
[[ "${1:-}" == "--reparse" ]] && REPARSE=1

cd "$REPO"
cargo build -q -p eigenius-cli
eig() { "$REPO/target/debug/eigenius" --endpoint "$ENDPOINT" "$@"; }

hr() { printf '\n\033[1m%s\033[0m\n' "── $* ─────────────────────────────────────────"; }

hr "0. Kernel on the LEXICON snapshot"
# The encoded claims' propositions are built from lexicon axioms (`wn:v02627934_t` = the verb sense
# of `require`, and so on), so the chain the claims commit to must be the one that DEFINES those
# axioms. Committing a parsed proposition onto a bare core+domain chain fails at the D47 decode with
# `ConstRef references unresolved IRI` — the parser and the chain have to share a lexicon.
SNAPSHOT="${EIGENIUS_DB_SNAPSHOT:-$REPO/../db-snapshot/wordnet-umls-aligned-2026-08-03-sigmaproj}"
[[ -f "$SNAPSHOT/CURRENT" ]] || {
    echo "ERROR: no lexicon snapshot at $SNAPSHOT" >&2
    echo "  build one: scripts/reseed-lexicon-db.sh && scripts/build-alignment-snapshot.sh …" >&2
    exit 1
}
VOLUME="${VOLUME:-eigenius_eigenius_db}"
echo "staging $(basename "$SNAPSHOT") into volume $VOLUME (the snapshot itself is read-only)"
docker compose down >/dev/null 2>&1 || true
docker volume rm "$VOLUME" >/dev/null 2>&1 || true
docker volume create "$VOLUME" >/dev/null
docker run --rm -v "$(readlink -f "$SNAPSHOT")":/src:ro -v "$VOLUME":/dst alpine \
    sh -c 'cp -a /src/. /dst/' >/dev/null
EIGENIUS_MOCK_LLM=true docker compose up -d --no-deps kernel >/dev/null
until [[ "$(docker inspect -f '{{.State.Health.Status}}' eigenius-kernel-1 2>/dev/null)" == "healthy" ]]; do
    [[ "$(docker inspect -f '{{.State.Status}}' eigenius-kernel-1 2>/dev/null)" == "exited" ]] && {
        echo "ERROR: kernel exited before becoming healthy" >&2; docker logs --tail 20 eigenius-kernel-1; exit 1; }
    sleep 3
done
echo "kernel healthy at $ENDPOINT, serving the lexicon chain"

if [[ $REPARSE == 1 ]]; then
    hr "0b. Re-deriving the claims layers from the lexicon snapshot"
    SNAPSHOT="${EIGENIUS_DB_SNAPSHOT:?set EIGENIUS_DB_SNAPSHOT to an aligned WordNet+UMLS snapshot}"
    cargo build -q --release -p eigenius-encoding
    # Each variant replays its OWN recording: the reranker's key includes the sentence and its
    # candidate senses, so the edited paragraph is a different question and a shared ranks file
    # would MISS on it and silently fall back to cap-only.
    "$REPO/target/release/prose-to-eigon" --snapshot "$SNAPSHOT" \
        --source "$HERE/paragraph.txt"        --pins "$HERE/pins.tsv" \
        --ranks  "$HERE/ranks.json"           --ns   "urn:eigenius:demo:prose" \
        --out    "$HERE/claims-intact.json"
    "$REPO/target/release/prose-to-eigon" --snapshot "$SNAPSHOT" \
        --source "$HERE/paragraph-edited.txt" --pins "$HERE/pins.tsv" \
        --ranks  "$HERE/ranks-edited.json"    --ns   "urn:eigenius:demo:prose" \
        --out    "$HERE/claims-edited.json"
    echo "NOTE: argument.json is deliberately NOT regenerated — it is the RECORDED argument."
fi

hr "1. Vocabulary"
# D62 pipeline classes (DiscourseUnit / ScopedUnit / EncodedClaim / DecisionPoint) — the emitted
# claims layer is written in this vocabulary, and it is NOT part of the bootstrap.
eig load "$REPO/ontologies/encoding/encoding.esl"
# Domain vocabulary, reused from the WRN case study unchanged.
eig load "$REPO/experiments/benchmark/base-ontologies/bench-core.esl"
eig load "$WRN/chain/01-onco.esl"
BASE="$(eig branch show main --json | grep -o '"head_layer": *"[^"]*"' | cut -d'"' -f4)"
[[ -n "$BASE" ]] || { echo "ERROR: could not read main's head layer" >&2; exit 1; }
echo "base head: $BASE"

for br in prose-intact prose-edited; do
    eig branch delete "$br" >/dev/null 2>&1 || true
    eig branch create "$br" --from "$BASE"
done

hr "2. INTACT — the paragraph as written"
cat "$HERE/paragraph.txt"
eig load --branch prose-intact "$HERE/claims-intact.json"
echo
echo "Loading the recorded argument (2 Declared bridges + 2 ReasoningSentences)…"
if eig load --branch prose-intact "$HERE/argument.json"; then
    echo
    echo "✓ COMMITTED. Every certificate type-checked against its cited witnesses."
else
    echo "✗ UNEXPECTED: the intact argument should commit." >&2
    exit 1
fi

hr "3. EDITED — one negation deleted from the second sentence"
diff <(tr ' ' '\n' < "$HERE/paragraph.txt") \
     <(tr ' ' '\n' < "$HERE/paragraph-edited.txt") || true
eig load --branch prose-edited "$HERE/claims-edited.json"
echo
echo "The edited prose parses, and to the SAME structural skeleton. Now the SAME"
echo "recorded argument, unchanged, against the edited claims…"
if eig load --branch prose-edited "$HERE/argument.json"; then
    echo
    echo "✗ UNEXPECTED: the argument committed against edited prose. The demo's claim is FALSE." >&2
    exit 1
else
    echo
    echo "✓ REJECTED — as intended."
    echo
    echo "  The certificate for sentence 2 cites  derived(claim_2, P)  where P is the"
    echo "  proposition the parser derived from the ORIGINAL sentence. The edited prose"
    echo "  derives a different proposition, so no IsDerivedAs witness matches, the"
    echo "  JustifiedBy certificate fails to type-check, and ValidateJustification"
    echo "  returns Fails — which rejects the commit."
    echo
    echo "  Nothing compared the two texts. The kernel rejected an argument that no"
    echo "  longer follows from what the document says."
fi
