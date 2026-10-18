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

# Provision the SPECIALIST Lexicon (NLM Lexical Systems Group) for D97 — the lexicon's
# syntactic authority: verb complement frames, governed prepositions, countability,
# nominalizations, acronym and abbreviation expansions.
#
# ONE FILE, NO LICENCE GATE. The SPECIALIST NLP Tools are "available to all requesters at
# no charge" — no UTS account, unlike UMLS. The file is gitignored (under /references):
# this repo provisions third-party corpora, it does not vendor them.
#
# THE URL IS NOT DERIVABLE FROM THE PROJECT PAGES, which is why it is pinned here. The LSG
# pages link to `lhncbc.nlm.nih.gov/LSG/...` and `lsg3.nlm.nih.gov/LexSysGroup/...`; both
# redirect to directory listings that answer 403. The distribution is served from a
# different host entirely, `data.lhncbc.nlm.nih.gov/public/`.
#
# SPECIALIST ALSO SHIPS INSIDE THE UMLS FULL RELEASE as an other-knowledge-source
# (`umls-<release>-full.zip` → `<release>aa-otherks.nlm` → `LEX/`), next to the Semantic
# Network that provision-umls.sh extracts from the same nested zip. That route needs a UTS
# licence and a ~30 GB download, so this script takes the direct one. Note a
# `-metathesaurus-*.zip` is a DIFFERENT download containing `<RELEASE>/META/` and nothing
# else — it has no LEX, which is the trap this comment exists to disarm.
#
# VERIFICATION IS BY CONTENT, NOT BY NAME. A release name says little; these counts say
# which file you have. The expected values are D97's own measurements, and a mismatch is
# reported rather than ignored — a silently different lexicon would move every D97 number
# downstream without anything failing.
#
# Terms (`lhncbc.nlm.nih.gov/LSG/Projects/lexicon/current/web/termsAndConditions.html`):
# redistribution is allowed with the terms included, attributing "the SPECIALIST NLP Tools
# with the release number and date" and stating any modifications. Nothing here
# redistributes the file; the importer records the release.
#
# Usage:
#   scripts/provision-specialist.sh                  # fetch if absent, then verify
#   scripts/provision-specialist.sh --check          # verify what is on disk; fetch nothing
#   scripts/provision-specialist.sh --force          # re-fetch even if present
#
# Env overrides:
#   RELEASE   lexicon release year (default: 2026)
#   OUT       output path (default: references/specialist/LEXICON, gitignored)
#   URL       full download URL (default: derived from RELEASE)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RELEASE="${RELEASE:-2026}"
OUT="${OUT:-references/specialist/LEXICON}"
URL="${URL:-https://data.lhncbc.nlm.nih.gov/public/lsg/lexicon/${RELEASE}/release/LEX_DOC/LEXICON}"

# D97's measurements of the 2026 release. `records` and `bytes` are the coarse check;
# the field counts are the ones that distinguish two releases of a similar size.
EXPECT_BYTES=56012657
EXPECT_SHA256=259d0283ebe7b027be730538d2c77c10f13f09bb824106306fbf5838d0a629f5
EXPECT_RECORDS=534345
EXPECT_NOMINALIZATION=16534
EXPECT_ACRONYM=67675
EXPECT_ABBREVIATION=23989

CHECK_ONLY=0
FORCE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK_ONLY=1; shift ;;
    --force) FORCE=1; shift ;;
    -h|--help) sed -n '17,56p' "$0"; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ "$CHECK_ONLY" -eq 1 && ! -f "$OUT" ]]; then
  echo "error: --check but no lexicon at $OUT" >&2
  echo "  run scripts/provision-specialist.sh (no flags) to fetch it." >&2
  exit 2
fi

if [[ -f "$OUT" && "$FORCE" -eq 0 && "$CHECK_ONLY" -eq 0 ]]; then
  echo "provision-specialist: $OUT already present ($(wc -c <"$OUT") bytes); verifying (--force to re-fetch)" >&2
elif [[ "$CHECK_ONLY" -eq 0 ]]; then
  mkdir -p "$(dirname "$OUT")"
  echo "provision-specialist: fetching SPECIALIST ${RELEASE} → $OUT" >&2
  echo "  $URL" >&2
  # To a temporary file, then move: a truncated or 404 body must never land at $OUT looking
  # like a lexicon. -f makes an HTTP error a non-zero exit instead of a saved error page.
  tmp="$(mktemp "${OUT}.partial.XXXXXX")"
  trap 'rm -f "$tmp"' EXIT
  if ! curl -fSL --retry 3 --retry-delay 2 -o "$tmp" "$URL"; then
    echo "error: download failed from $URL" >&2
    echo "  the LSG project pages do NOT serve this file; check the host is data.lhncbc.nlm.nih.gov" >&2
    echo "  (see docs/design/d97-specialist-lexicon.md for the alternative UMLS Full Release route)" >&2
    exit 1
  fi
  mv "$tmp" "$OUT"
  trap - EXIT
fi

# --- verify by content -------------------------------------------------------------

bytes=$(wc -c <"$OUT")
sha=$(sha256sum "$OUT" | cut -d' ' -f1)
records=$(grep -c '^{base=' "$OUT" || true)
nominalization=$(grep -c $'^\tnominalization=' "$OUT" || true)
acronym=$(grep -c $'^\tacronym_of=' "$OUT" || true)
abbreviation=$(grep -c $'^\tabbreviation_of=' "$OUT" || true)

status=0
report() { # name actual expected
  if [[ "$2" == "$3" ]]; then
    printf '  %-16s %-12s ok\n' "$1" "$2"
  else
    printf '  %-16s %-12s MISMATCH (expected %s)\n' "$1" "$2" "$3"
    status=1
  fi
}

echo "provision-specialist: verifying $OUT against D97's measurements" >&2
report bytes           "$bytes"           "$EXPECT_BYTES"
report sha256          "$sha"             "$EXPECT_SHA256"
report records         "$records"         "$EXPECT_RECORDS"
report nominalization  "$nominalization"  "$EXPECT_NOMINALIZATION"
report acronym_of      "$acronym"         "$EXPECT_ACRONYM"
report abbreviation_of "$abbreviation"    "$EXPECT_ABBREVIATION"

if [[ "$status" -ne 0 ]]; then
  cat >&2 <<'MSG'

This is NOT the release D97 measured. Every D97 number — the 399 verbs that gain an object,
the 4 390 verbs WordNet lacks, the 744 named prepositions — was counted against the file
above. Measuring against a different one and comparing to those figures reports a change
that is really a change of input.

Either provision the release D97 measured, or re-measure and update BOTH the expected values
here and D97's tables, in one commit, saying which release.
MSG
  exit 1
fi

echo "provision-specialist: ok — SPECIALIST ${RELEASE}, ${records} records at $OUT" >&2
