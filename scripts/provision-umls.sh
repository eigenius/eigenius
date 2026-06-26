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

# Provision the UMLS domain lexicon (D65 §5): extract → convert → load.
#
# UMLS is LICENSED, not public-domain. You must hold your own UMLS Metathesaurus
# License and download the release yourself; this script does NOT fetch it. Place
# the Level-0 Metathesaurus zip at references/umls-<release>-metathesaurus-level0.zip
# (the default below), then run this script.
#
#   extract   unzip only the RRF files the importer needs (MRCONSO/MRSTY/MRSAB/
#             MRRANK/MRDEF) into references/umls/<release>/META/ (gitignored — UMLS
#             data is licensed and is NEVER committed).
#   convert   run the DETERMINISTIC importer (no LLM) → an Eigon-ESL document: a typed
#             mirror (umls:Concept classes under umls:SemanticType classes) + a derived
#             lexicon (lexicon:umls). Only SRL-0 (Level 0) sources are emitted; the
#             output carries the UMLS license notice + the redistribution constraint.
#   load      `--validate` (compile + felicity-gate) by default; with --endpoint, ALSO
#             persist the layer into a running `eigenius serve`.
#
# Usage:
#   scripts/provision-umls.sh                          # WRN-relevant subset (default TUIs)
#   scripts/provision-umls.sh --all                    # all semantic types (large!)
#   scripts/provision-umls.sh --tui T047 --tui T028    # custom semantic-type allowlist
#   scripts/provision-umls.sh --endpoint 127.0.0.1:50051
#
# Env overrides:
#   UMLS_ZIP    the Level-0 Metathesaurus zip (default: references/umls-2026AA-metathesaurus-level0.zip)
#   RELEASE     the release label (default: 2026AA)
#   META        extracted META dir (default: references/umls/<release>/META)
#   OUT         ESL output path (default: umls.esl, gitignored)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RELEASE="${RELEASE:-2026AA}"
UMLS_ZIP="${UMLS_ZIP:-references/umls-${RELEASE}-metathesaurus-level0.zip}"
META="${META:-references/umls/${RELEASE}/META}"
OUT="${OUT:-umls.esl}"

# Default semantic-type allowlist — the WRN-paper-relevant types (Disease or Syndrome,
# Cell or Molecular Dysfunction, Neoplastic Process, Gene or Genome, Diagnostic
# Procedure, Pharmacologic Substance, Enzyme, Amino Acid/Peptide/Protein).
DEFAULT_TUIS=(T047 T049 T191 T028 T060 T121 T126 T116)

ENDPOINT=""
LIMIT=""
ALL=""
TUIS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint) ENDPOINT="$2"; shift 2 ;;
    --all) ALL="1"; shift ;;
    --tui) TUIS+=("$2"); shift 2 ;;
    --limit) LIMIT="--limit $2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

# ── extract ───────────────────────────────────────────────────────────
NEEDED=(MRCONSO.RRF MRSTY.RRF MRSAB.RRF MRRANK.RRF MRDEF.RRF)
missing=0
for f in "${NEEDED[@]}"; do [[ -f "$META/$f" ]] || missing=1; done
if [[ "$missing" == "1" ]]; then
  [[ -f "$UMLS_ZIP" ]] || { echo "error: UMLS zip not found at $UMLS_ZIP (obtain it with your UMLS license)" >&2; exit 2; }
  mkdir -p "$META"
  echo ">> extracting RRF files from $UMLS_ZIP → $META"
  for f in "${NEEDED[@]}"; do
    unzip -o -j "$UMLS_ZIP" "${RELEASE}/META/$f" -d "$META" >/dev/null
  done
fi
echo ">> META: $META"

# ── convert + validate ────────────────────────────────────────────────
TUI_ARGS=()
if [[ -z "$ALL" ]]; then
  [[ ${#TUIS[@]} -gt 0 ]] || TUIS=("${DEFAULT_TUIS[@]}")
  for t in "${TUIS[@]}"; do TUI_ARGS+=(--semantic-type "$t"); done
  echo ">> semantic-type allowlist: ${TUIS[*]}"
else
  echo ">> importing ALL semantic types (this is large)"
fi

echo ">> converting (release=$RELEASE) → $OUT"
# shellcheck disable=SC2086
cargo run -q -p eigenius-umls --bin umls-import -- \
  --meta-dir "$META" --version "$RELEASE" --out "$OUT" --validate "${TUI_ARGS[@]}" $LIMIT

# ── load into a running service (optional) ─────────────────────────────
if [[ -n "$ENDPOINT" ]]; then
  echo ">> loading into eigenius serve @ $ENDPOINT"
  cargo run -q -p eigenius-cli -- --endpoint "http://$ENDPOINT" load "$OUT"
fi

echo ">> done."
