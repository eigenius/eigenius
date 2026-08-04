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
# prose-to-formulas — a paragraph of the WRN paper, parsed, committed, and then edited until the
# kernel refuses it. The domain predicates take FORMULAS, NOT STRINGS.
#
# A string-typed predicate would force the bridge to assert that a proposition containing the class
# `umlscui:C0920283` implies `RequiresActivity("WRN", "helicase")` — string literals nothing relates
# to any class. Typed `Set -> Set -> Prop`, the consequent instead names the SAME
# classes the antecedent contains, and the emitter refuses any argument class the sentence does not
# actually mention.
#
# Two branches off one base, differing only in the prose:
#
#   formulas-intact   paragraph.txt          → argument commits, ValidateJustification Holds
#   formulas-edited   paragraph-edited.txt   → argument REJECTED
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
#   ./demo/prose-to-formulas/run.sh
#   ./demo/prose-to-formulas/run.sh --reparse           # re-derive the claims layers from the
#                                                    # lexicon snapshot instead of using the
#                                                    # committed fixtures (needs the snapshot)

set -euo pipefail

ENDPOINT="${EIGENIUS_ENDPOINT:-http://localhost:50051}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
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
SNAPSHOT="${EIGENIUS_DB_SNAPSHOT:-$REPO/../db-snapshot/wordnet-umls-aligned-2026-08-03-specpoly}"
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
        --ranks  "$HERE/ranks.json"           --ns   "urn:eigenius:demo:formulas" \
        --out    "$HERE/claims-intact.json"   --claims "$HERE/claims.tsv" \
        --rules-out "$HERE/rules.json"        --citations-out "$HERE/argument.json"
    "$REPO/target/release/prose-to-eigon" --snapshot "$SNAPSHOT" \
        --source "$HERE/paragraph-edited.txt" --pins "$HERE/pins.tsv" \
        --ranks  "$HERE/ranks-edited.json"    --ns   "urn:eigenius:demo:formulas" \
        --out    "$HERE/claims-edited.json"
    echo "NOTE: argument.json is deliberately NOT regenerated — it is the RECORDED argument."
fi

hr "1. Vocabulary + the pinned literature rule"
eig load "$REPO/ontologies/encoding/encoding.esl"
eig load "$HERE/onco-typed.esl"
# A rule from the literature — NOT from this document. Hand-authorable because it is in domain
# vocabulary; a rule whose antecedent had to be a PARSE would be inexpressible in ESL.
eig load "$HERE/literature-rules.esl"
BASE="$(eig branch show main --json | grep -o '"head_layer": *"[^"]*"' | cut -d'"' -f4)"
[[ -n "$BASE" ]] || { echo "ERROR: could not read main's head layer" >&2; exit 1; }
echo "base head: $BASE"

for br in formulas-intact formulas-edited; do
    eig branch delete "$br" >/dev/null 2>&1 || true
    eig branch create "$br" --from "$BASE"
done

hr "2. INTACT — the document as written"
cat "$HERE/paragraph.txt"
echo
echo "-- the parsed claims (one enc:EncodedClaim + ProgramTrace per sentence)"
eig load --branch formulas-intact "$HERE/claims-intact.json"
echo
echo "   Each sentence is now a FORMULA over classes the chain already held:"
echo
echo "   «MSI cancer models had the exonuclease activity of WRN.»"
python3 "$HERE/narrate.py" "$HERE/claims-intact.json" claim_1
echo
echo "   «MSI cancer models required the helicase activity of WRN.»"
python3 "$HERE/narrate.py" "$HERE/claims-intact.json" claim_2
echo
echo "   Note the arguments: UMLS concepts and WordNet synsets the graph already"
echo "   contained. Not strings about them — the classes themselves."
echo "-- the vocabulary lift: shape rules, then one citation per sentence"
eig load --branch formulas-intact "$HERE/rules.json"
eig load --branch formulas-intact "$HERE/bridges.json"
echo
echo "-- THE INFERENCE: apply the pinned literature rule to the MEASUREMENT claim"
echo "   rule (pinned, cited):  HasActivity(WRN, exonuclease) ⟹ RequiresActivity(WRN, helicase)"
if eig load --branch formulas-intact "$HERE/inference.json"; then
    echo
    echo "   concluded proposition:"
    python3 "$HERE/narrate.py" "$HERE/inference.json" sentence
    echo
    echo "✓ COMMITTED."
    echo "  RequiresActivity(WRN, helicase) is now justified TWICE on this branch:"
    echo "    · because sentence 2 asserts it            (the document says so)"
    echo "    · because it FOLLOWS from sentence 1       (measurement + published rule)"
    echo "  The second justification does not depend on the document stating the conclusion."
else
    echo "✗ UNEXPECTED: the intact inference should commit." >&2
    exit 1
fi

hr "3. EDITED — the measurement is negated"
diff <(tr ' ' '\n' < "$HERE/paragraph.txt") \
     <(tr ' ' '\n' < "$HERE/paragraph-edited.txt") || true
eig load --branch formulas-edited "$HERE/claims-edited.json"
eig load --branch formulas-edited "$HERE/rules.json"
echo
echo "   the measurement's formula, before and after — the edit is VISIBLE in the term:"
echo "   before:"; python3 "$HERE/narrate.py" "$HERE/claims-intact.json" claim_1
echo "   after :"; python3 "$HERE/narrate.py" "$HERE/claims-edited.json" claim_1
echo
echo "The same recorded lift, against the edited measurement…"
if eig load --branch formulas-edited "$HERE/bridges.json"; then
    echo "✗ UNEXPECTED: the lift should not survive an edited measurement." >&2
    exit 1
else
    echo
    echo "✓ REJECTED — as intended."
    echo
    echo "  Sentence 1 now parses to a different proposition, so no IsDerivedAs witness matches"
    echo "  the one its citation names. The lift fails, and with it everything downstream:"
    echo "  the inferred RequiresActivity claim has no antecedent left to stand on."
    echo
    echo "  Nothing compared the two texts. A measurement changed, and every conclusion that"
    echo "  rested on it stopped committing."
fi
