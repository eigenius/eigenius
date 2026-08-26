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
# prose-to-formulas v2 — the same argument as v1, produced by the COMPOSED PIPELINE.
#
# What differs from v1 (demo/prose-to-formulas), which this replaces:
#
#   · NO PINS. v1 selected each sentence's reading by a human-verified skeleton pin. Here the
#     reading ranker chooses, in document context, and the choice is recorded as an
#     `enc:DecisionPoint` with the ranker's own rationale. It lands the SAME term v1's pin did —
#     `claim_1`'s proposition is byte-identical across the two demos — so the pin is no longer
#     load-bearing, it is a check that was passed.
#   · ANAPHORA. A third sentence refers back: «These findings show that WRN is a vulnerability of
#     MSI cancer models.» «These findings» is a restrictor-typed hole (D64) that resolves against
#     claims LANDED EARLIER IN THE SAME RUN (D68) — the binding is committed as an
#     `enc:AnaphorBinding` naming the antecedent claim by IRI, with the proposer's confidence and
#     reasoning.
#   · DISCOURSE KINDS. Each landed claim carries its kind as a second `is_a` class (enc:Finding /
#     enc:Observation / …). That axis is what the anaphor's restrictor is checked against: a
#     claim the classifier judged an OBSERVATION is not eligible for «these findings», and the
#     recorded binding shows the veto doing exactly that.
#
# Two branches off one base, differing only in the prose:
#
#   v2-intact   paragraph.txt          → argument commits, ValidateJustification Holds
#   v2-edited   paragraph-edited.txt   → argument REJECTED
#
# The edit negates the first sentence — the measurement. It still parses; the proposition is a
# different term, so the certificate that cites the original has no witness.
#
# The two variants are parsed and selected INDEPENDENTLY — no shared draw, no shared pin — and
# they nevertheless land the SAME term apart from the negation: `claim_1`'s proposition here is
# `<intact term> -> logic:False`, byte-identical otherwise. That did not hold before D69: the
# negated sentence's 120-reading pool rendered to 4 distinct strings, so the ranker could not see
# the concept-vs-compound distinction and picked a compound reading. With the expanded register
# it sees it and picks the same reading a human pinned.
#
# Prerequisites:
#   docker compose build kernel                      ← REBUILD AFTER ANY KERNEL CHANGE.
#   EIGENIUS_MOCK_LLM=true docker compose up -d      (kernel on :50051)
#
# Usage:
#   ./demo/prose-to-formulas-v2/run.sh
#   ./demo/prose-to-formulas-v2/run.sh --reparse     # re-derive the claims through the composed
#                                                    # pipeline (needs the snapshot; replays the
#                                                    # four committed draws, so no LLM and no key)

set -euo pipefail

ENDPOINT="${EIGENIUS_ENDPOINT:-http://localhost:50051}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
REPARSE=0
[[ "${1:-}" == "--reparse" ]] && REPARSE=1

cd "$REPO"
cargo build -q -p eigenius-cli
eig() { "$REPO/target/debug/eigenius" --endpoint "$ENDPOINT" "$@"; }

eig_load() {
    local branch=()
    if [[ "$1" == "--branch" ]]; then branch=(--branch "$2"); shift 2; fi
    local shown="${1#"$REPO"/}"
    printf '\033[2m   loading %s\033[0m\n' "$shown"
    eig load "${branch[@]}" "$1"
}

hr() { printf '\n\033[1m%s\033[0m\n' "── $* ─────────────────────────────────────────"; }

hr "0. Kernel on the LEXICON snapshot"
# Newest ALIGNED snapshot by mtime, the same rule `just stage-snapshot` and
# `scripts/measure-parse-rate.sh` use. This was a hardcoded name until `2026-08-21`, which
# rots on every reseed: a bootstrap edit rehashes the chain, the pinned snapshot drifts, and
# the run dies in `prose-to-esl` with ManifestDrift after having already staged the stale
# store into the volume. `scripts/reseed-lexicon-db.sh` even ends by telling you to re-point
# this line by hand — a step that only works if someone reads the message.
#
# ALIGNED, not raw: the anaphor's restrictor is checked against a claim's KIND class, and
# `claim-kind-alignment.esl` arrives only in the aligned layer. A raw snapshot fails closed.
SNAPSHOT_ROOT="${SNAPSHOT_ROOT:-$REPO/../db-snapshot}"
SNAPSHOT="${EIGENIUS_DB_SNAPSHOT:-$(ls -1dt "$SNAPSHOT_ROOT"/wordnet-umls-aligned-* 2>/dev/null | head -1 || true)}"
[[ -n "$SNAPSHOT" ]] || {
    echo "ERROR: no aligned snapshot under $SNAPSHOT_ROOT" >&2
    echo "  run scripts/reseed-lexicon-db.sh --umls-all, then scripts/build-alignment-snapshot.sh" >&2
    exit 1
}
[[ -d "$SNAPSHOT" && -f "$SNAPSHOT/CURRENT" ]] || {
    echo "ERROR: not a RocksDB store: $SNAPSHOT" >&2; exit 1
}
VOLUME="${VOLUME:-eigenius_eigenius_db}"
# **Take the stack down before staging.** The staging step below is
# `rm -rf /dst/*` on the volume, and `docker compose up -d` is a no-op against an
# already-running container — so re-running this script without this line deletes
# the store OUT FROM UNDER A LIVE RocksDB and then lets the same kernel keep
# serving it. The failures that produces are arbitrary and blame the wrong thing:
# on 2026-08-26 it surfaced as `CoreNamespaceViolation { core:Asserts }` from
# `merge_independent_heads`, which reads as a merge defect and is not one. A run
# that aborts part-way (the common case while iterating) leaves the container up,
# so this is the normal path, not an edge case.
docker compose down >/dev/null 2>&1 || true
echo "staging $(basename "$SNAPSHOT") into volume $VOLUME (the snapshot itself is read-only)"
docker run --rm -v "$SNAPSHOT":/src:ro -v "$VOLUME":/dst alpine \
    sh -c 'rm -rf /dst/* && cp -a /src/. /dst/' >/dev/null
EIGENIUS_MOCK_LLM=true docker compose up -d --no-deps kernel >/dev/null
for _ in $(seq 1 60); do
    if eig branch list >/dev/null 2>&1; then break; fi
    sleep 1
done
eig branch list >/dev/null || { echo "ERROR: kernel did not come up at $ENDPOINT" >&2; exit 1; }
echo "kernel healthy at $ENDPOINT, serving the lexicon chain"

if [[ $REPARSE == 1 ]]; then
    hr "0b. Re-deriving the claims through the COMPOSED PIPELINE"
    cargo build -q --release -p eigenius-encoding
    # All four LLM stages replay from committed draws: sense ranks, reading selections, anaphora
    # proposals, discourse kinds. No LLM, no network, no key — and a MISS in any of them fails
    # closed rather than silently falling back.
    #
    # NOTHING TO CHAIN-LOAD (since `2026-08-20`). The anaphor's restrictor is still checked
    # against a claim's KIND class — that has not changed — but both halves now arrive on their
    # own: `encoding.esl` is in the kernel's bootstrap chain, and `claim-kind-alignment.esl` is
    # layered into the aligned snapshot by scripts/build-alignment-snapshot.sh.
    for variant in "paragraph:claims-intact:" "paragraph-edited:claims-edited:-edited"; do
        IFS=: read -r src out suffix <<<"$variant"
        # The source path is passed REPO-RELATIVE and the command runs from $REPO: it lands in the
        # artifact verbatim as `enc:source_path`, so an absolute path would commit this machine's
        # home directory and the artifact would stop regenerating identically anywhere else.
        # (`enc:source_sha256` pins the bytes; the path is caller-supplied text.)
        ( cd "$REPO" && "$REPO/target/release/prose-to-esl" --snapshot "$SNAPSHOT" \
            --source "demo/prose-to-formulas-v2/$src.txt" \
            --ranks      "$HERE/ranks$suffix.json" \
            --selections "$HERE/selections$suffix.json" \
            --proposals  "$HERE/proposals$suffix.json" \
            --kinds      "$HERE/kinds$suffix.json" \
            --ns   "urn:eigenius:demo:v2" \
            --out  "$HERE/$out.esl" )
    done
    echo "NOTE: inference.esl is NOT regenerated — it is the RECORDED derivation."
fi

SCRATCH="$(mktemp -d)"; trap 'rm -rf "$SCRATCH"' EXIT
narrate() {
    local esl="$1" suffix="$2"
    local json="$SCRATCH/$(basename "${esl%.esl}").json"
    [[ -f "$json" ]] || "$REPO/target/debug/eigenius" compile "$esl" > "$json"
    python3 "$HERE/narrate.py" "$json" "$suffix"
}

hr "1. Vocabulary, the claim-kind axis, and the pinned literature rule"
# NOT loaded here any more (`2026-08-20`): `encoding.esl` is in the kernel's bootstrap chain and
# `claim-kind-alignment.esl` is layered into the aligned snapshot. They arrive on their own.
#
# Re-loading them was not merely redundant, it was FATAL. Every resource in a bootstrapped
# ontology re-loaded is a REDEFINITION, and a redefinition triggers Rule 22's retroactive
# validation across the chain — which on the 7.6M-resource lexicon reached 27 GB and was killed by
# the host OOM killer. Witnessed twice on `2026-08-20`; `commit.retroactive.start` is the last line
# in the kernel log both times. See docs/notes/kernel-oom-notebook-session.md.
eig_load "$HERE/onco-typed.esl"
eig_load "$HERE/literature-rules.esl"
BASE="$(eig branch show main --json | grep -o '"head_layer": *"[^"]*"' | cut -d'"' -f4)"
[[ -n "$BASE" ]] || { echo "ERROR: could not read main's head layer" >&2; exit 1; }
echo "base head: $BASE"

for br in v2-intact v2-edited; do
    eig branch delete "$br" >/dev/null 2>&1 || true
    eig branch create "$br" --from "$BASE"
done

hr "2. INTACT — the document as written"
cat "$HERE/paragraph.txt"
echo
echo "-- the parsed claims: one enc:EncodedClaim + DeclarationTrace per sentence, each carrying its"
echo "   DISCOURSE KIND as a second is_a class, plus the DecisionPoint recording who chose the"
echo "   reading and why, plus the AnaphorBinding for sentence 3."
eig_load --branch v2-intact "$HERE/claims-intact.esl"
echo
echo "   «MSI cancer models had the exonuclease activity of WRN.»"
narrate "$HERE/claims-intact.esl" claim_1
echo
echo "   «MSI cancer models required the helicase activity of WRN.»"
narrate "$HERE/claims-intact.esl" claim_2
echo
echo "   NO PIN CHOSE THESE. The reading ranker did, in document context — and it landed the"
echo "   same term v1's human-verified pin did (claim_1's proposition is byte-identical across"
echo "   the two demos). The choice is on the chain as an enc:DecisionPoint with its rationale:"
python3 - "$HERE/claims-intact.esl" <<'PY'
import re, sys, textwrap
s = open(sys.argv[1]).read()
m = re.search(r'resource \w+:decision_3 .*?reflection:rationale = "(.*?)";', s, re.S)
if m:
    # ESL string escapes only: \" and \n. (`unicode_escape` would mangle UTF-8 — it decodes
    # byte-wise, so an em dash comes back as mojibake.)
    txt = m.group(1).replace('\\"', '"').replace('\\n', ' ')
    txt = txt.split('Ranker rationale:')[-1].strip()
    print(textwrap.fill(txt, 92, initial_indent="     ", subsequent_indent="     "))
PY
echo
echo "-- «These findings show that WRN is a vulnerability of MSI cancer models.»"
echo "   The demonstrative is a HOLE typed by its restrictor; it resolved against claims landed"
echo "   earlier IN THIS RUN. The binding is on the chain, naming the antecedent by IRI:"
python3 - "$HERE/claims-intact.esl" <<'PY'
import re, sys, textwrap
s = open(sys.argv[1]).read()
m = re.search(r'resource \w+:binding_3_0[^{]*\{(.*?)\n\}', s, re.S)
if not m:
    raise SystemExit("     (no AnaphorBinding in the artifact — the anaphor did not resolve)")
for line in m.group(1).strip().splitlines():
    line = line.strip().replace('\\"', '"')
    if line.startswith(("encoding:antecedent", "encoding:bound_by", "encoding:confidence",
                        "reflection:rationale")):
        print(textwrap.fill(line, 92, initial_indent="     ", subsequent_indent="       "))
PY
echo
echo "   and the claim it produced — note the antecedent claim INSIDE the formula:"
narrate "$HERE/claims-intact.esl" claim_3
echo
echo "   That is the kind axis doing work: a claim the classifier judged an OBSERVATION rather"
echo "   than a FINDING is not eligible for «these findings», and the kernel's restrictor veto —"
echo "   not the model — is what enforces it. The model only proposes among what survives."
echo
echo "-- THE INFERENCE: apply the pinned literature rule to the MEASUREMENT claim"
echo "   rule (pinned, cited):  ∀m. HasActivity(m, WRN) ⟹ RequiresActivity(m, WRN)"
if eig_load --branch v2-intact "$HERE/inference.esl"; then
    echo
    echo "   concluded proposition:"
    narrate "$HERE/inference.esl" sentence
    echo
    echo "✓ COMMITTED."
    echo "  RequiresActivity(MSI, WRN) is justified TWICE on this branch:"
    echo "    · because sentence 2 asserts it            (the document says so)"
    echo "    · because it FOLLOWS from sentence 1       (measurement + published rule)"
else
    echo "✗ UNEXPECTED: the intact inference should commit." >&2
    exit 1
fi

hr "3. EDITED — the measurement is negated"
diff <(tr ' ' '\n' < "$HERE/paragraph.txt") \
     <(tr ' ' '\n' < "$HERE/paragraph-edited.txt") || true
eig_load --branch v2-edited "$HERE/claims-edited.esl"
echo
echo "   the measurement's formula, before and after:"
echo "   before:"; narrate "$HERE/claims-intact.esl" claim_1
echo "   after :"; narrate "$HERE/claims-edited.esl" claim_1
echo
echo "   Parsed and selected INDEPENDENTLY — no shared draw, no pin — and identical apart from"
echo "   the trailing -> False. One token of prose, one line of formula."
echo
echo "-- THE INFERENCE that stood on sentence 1 — the same file step 2 committed:"
if eig_load --branch v2-edited "$HERE/inference.esl"; then
    echo "   ✗ UNEXPECTED: the inference must not commit on the edited measurement." >&2; exit 1
else
    echo
    echo "   ✓ REJECTED — the derivation is gone with the measurement it stood on."
    echo
    echo "     inference.esl cites claim_1 DIRECTLY — the parser's own IsDeclaredAs witness — for"
    echo "     its antecedent. The witness key hashes the PROPOSITION; the edited sentence parses"
    echo "     to a different term, so there is no witness under that key. (The kernel reports the"
    echo "     gate verdict, not the missing witness; the ValidateJustification diagnostic is not"
    echo "     surfaced through Load today — the in-process acceptance test shows it.)"
    echo
    echo "     The ASSERTED route survived; the DERIVED one did not. Nothing compared the texts."
fi
