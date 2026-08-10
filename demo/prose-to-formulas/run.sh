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
# A string-typed predicate would force something to ASSERT that a proposition containing the class
# `umlscui:C0920283` implies `RequiresActivity("WRN", "helicase")` — string literals nothing relates
# to any class. Typed over `Set` and DEFINED over the parser's own lexicon (D66), the domain
# predicates are not separate from the parse at all: `HasActivity(m, g, a)` unfolds to the parsed
# sentence, so the lift is definitional equality and no bridge exists to get wrong.
#
# Two branches off one base, differing only in the prose:
#
#   formulas-intact   paragraph.txt          → argument commits, ValidateJustification Holds
#   formulas-edited   paragraph-edited.txt   → argument REJECTED
#
# The edit inserts one negation ("had" → "did not have") into the first sentence —
# the measurement. It still parses; the proposition gains a trailing '→ False', so
# it is a different term, and the recorded argument's certificate cites the old one.
#
# Prerequisites:
#   docker compose build kernel                      ← REBUILD AFTER ANY KERNEL CHANGE.
#   EIGENIUS_MOCK_LLM=true docker compose up -d      (kernel on :50051)
#
# `docker-compose.yml` pins `image: eigenius-kernel:local` alongside its `build:` stanza, so
# `docker compose up` REUSES the existing image and never rebuilds. A demo run therefore exercises
# whatever kernel was last built, not the working tree — a change to the lexer, the validator or the
# codec will simply not be there, and the failure looks like a bug in the ESL rather than a stale
# image. `cargo test` passing says nothing about this.
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

# `eig load` that names its artifact first. The narration refers to layers by ROLE ("the claims",
# "the inference"); printing the path ties each role to a file a reviewer can open.
eig_load() {
    local branch=()
    if [[ "$1" == "--branch" ]]; then branch=(--branch "$2"); shift 2; fi
    local shown="${1#"$REPO"/}"
    printf '\033[2m   loading %s\033[0m\n' "$shown"
    eig load "${branch[@]}" "$1"
}

hr() { printf '\n\033[1m%s\033[0m\n' "── $* ─────────────────────────────────────────"; }

hr "0. Kernel on the LEXICON snapshot"
# The encoded claims' propositions are built from lexicon axioms (`wn:v02627934_t` = the verb sense
# of `require`, and so on), so the chain the claims commit to must be the one that DEFINES those
# axioms. Committing a parsed proposition onto a bare core+domain chain fails at the D47 decode with
# `ConstRef references unresolved IRI` — the parser and the chain have to share a lexicon.
SNAPSHOT="${EIGENIUS_DB_SNAPSHOT:-$REPO/../db-snapshot/wordnet-umls-aligned-d66}"
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
    cargo build -q --release -p eigenius-encoding
    # Each variant replays its OWN recording: the reranker's key includes the sentence and its
    # candidate senses, so the edited paragraph is a different question and a shared ranks file
    # would MISS on it and silently fall back to cap-only.
    # `prose-to-esl`, not `prose-to-eigon`: same pipeline, ESL instead of Eigon-JSON. The chain
    # artifacts are committed in the language they were authored in, so a reviewer reads the
    # generated formulas rather than a D47 encoding of them.
    # Only the CLAIMS layer is derived from the prose. The lift from a parsed sentence to domain
    # vocabulary is definitional equality (`onco-typed.esl`), so the claims are the only
    # generated artifact.
    "$REPO/target/release/prose-to-esl" --snapshot "$SNAPSHOT" \
        --source "$HERE/paragraph.txt"        --pins "$HERE/pins.tsv" \
        --ranks  "$HERE/ranks.json"           --ns   "urn:eigenius:demo:formulas" \
        --out    "$HERE/claims-intact.esl"
    "$REPO/target/release/prose-to-esl" --snapshot "$SNAPSHOT" \
        --source "$HERE/paragraph-edited.txt" --pins "$HERE/pins.tsv" \
        --ranks  "$HERE/ranks-edited.json"    --ns   "urn:eigenius:demo:formulas" \
        --out    "$HERE/claims-edited.esl"
    echo "NOTE: inference.esl is NOT regenerated — it is the RECORDED derivation."
fi


# `narrate.py` renders a D47 term with concept names substituted, so it needs the ENCODING, not the
# source. Compile each file once into the scratch dir — the ESL is the committed artifact, the JSON
# is derived on demand, and there is only ever one copy of the content.
SCRATCH="$(mktemp -d)"; trap 'rm -rf "$SCRATCH"' EXIT
narrate() {
    local esl="$1" suffix="$2"
    local json="$SCRATCH/$(basename "${esl%.esl}").json"
    # The bare binary, not `eig`: `compile` is local-only and refuses when `--endpoint` is set.
    [[ -f "$json" ]] || "$REPO/target/debug/eigenius" compile "$esl" > "$json"
    python3 "$HERE/narrate.py" "$json" "$suffix"
}

hr "1. Vocabulary + the pinned literature rule"
eig_load "$REPO/ontologies/encoding/encoding.esl"
eig_load "$HERE/onco-typed.esl"
# A rule from the literature — NOT from this document. Its antecedent IS a parse, reached through
# the definitions above; `onco2:` names make it readable. The ONLY DeclarationTrace on the branch.
eig_load "$HERE/literature-rules.esl"
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
eig_load --branch formulas-intact "$HERE/claims-intact.esl"
echo
echo "   Each sentence is now a FORMULA over classes the chain already held:"
echo
echo "   «MSI cancer models had the exonuclease activity of WRN.»"
narrate "$HERE/claims-intact.esl" claim_1
echo
echo "   «MSI cancer models required the helicase activity of WRN.»"
narrate "$HERE/claims-intact.esl" claim_2
echo
echo "   Note the arguments: UMLS concepts and WordNet synsets the graph already"
echo "   contained. Not strings about them — the classes themselves."
echo
echo "   There is NO lift step. \`onco-typed.esl\` DEFINES HasActivity and RequiresActivity"
echo "   over the parser's own lexicon, so HasActivity(MSI, WRN, exonuclease) and the"
echo "   formula above are the same term — the kernel computes that. Nothing declares it,"
echo "   so there is nothing here for anyone to have got wrong."
echo
echo "-- THE INFERENCE: apply the pinned literature rule to the MEASUREMENT claim"
echo "   rule (pinned, cited):  ∀m. HasActivity(m, WRN, exonuclease) ⟹ RequiresActivity(m, WRN, helicase)"
echo "   specialized at m := «MSI cancer models», then applied to sentence 1's own parse"
if eig_load --branch formulas-intact "$HERE/inference.esl"; then
    echo
    echo "   concluded proposition:"
    narrate "$HERE/inference.esl" sentence
    echo
    echo "✓ COMMITTED."
    echo "  RequiresActivity(MSI, WRN, helicase) is now justified TWICE on this branch:"
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
eig_load --branch formulas-edited "$HERE/claims-edited.esl"
echo
echo "   the measurement's formula, before and after — the edit is VISIBLE in the term:"
echo "   before:"; narrate "$HERE/claims-intact.esl" claim_1
echo "   after :"; narrate "$HERE/claims-edited.esl" claim_1

echo
echo "-- sentence 2 is untouched by the edit (the ASSERTED route):"
echo "   claims-edited.esl committed claim_2 carrying the same formula as before, and that"
echo "   formula IS RequiresActivity(MSI, WRN, helicase) — by definition, not by assertion."
echo "   The document still says the conclusion."
echo

# The derivation itself, attempted on the edited branch. Step 2 committed it; here its antecedent
# never came into existence, so the same file is refused. Loading it is what makes the dependency
# VISIBLE rather than asserted in narration — without this the demo only shows the lift failing and
# leaves the audience to take the consequence on trust.
echo
echo "-- THE INFERENCE that stood on sentence 1 — the same file step 2 committed:"
if eig_load --branch formulas-edited "$HERE/inference.esl"; then
    echo "   ✗ UNEXPECTED: the inference must not commit on the edited measurement." >&2; exit 1
else
    echo
    echo "   ✓ REJECTED — the derivation is gone with the measurement it stood on."
    echo
    echo "     inference.esl cites claim_1 DIRECTLY — the parser's own IsDerivedAs witness —"
    echo "     for its antecedent. The witness key hashes the PROPOSITION, and the edited"
    echo "     sentence parses to that term with a trailing '→ False'. Different proposition,"
    echo "     different key, no witness. One line of formula is enough to miss. (The kernel"
    echo "     reports the gate verdict, not the missing witness; the ValidateJustification"
    echo "     diagnostic is not surfaced through Load today.)"
    echo
    echo "     The ASSERTED route survived; the DERIVED one did not. That asymmetry is"
    echo "     the point — a conclusion the graph produced carries a live dependency on"
    echo "     what it was produced from, and the document merely repeating it is not"
    echo "     the same fact."
    echo
    echo "     Nothing compared the two texts."
fi
