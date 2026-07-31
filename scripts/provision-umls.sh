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
#             data is licensed and is NEVER committed). ALSO extracts the Semantic
#             Network into references/umls/<release>/NET/ when the FULL release zip
#             is present — see the NET block below for why it is not in the others.
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
# The Semantic Network (SRDEF/SRSTR/SRSTRE1/SRSTRE2) — the semantic-TYPE layer, as opposed to META's
# concept layer. It is a SEPARATE UMLS Knowledge Source and is NOT in any `-metathesaurus-*.zip`:
# those contain `<RELEASE>/META/` and nothing else. It ships only in the FULL release, and there it
# is nested one level deeper — `<RELEASE>-full/<release>aa-otherks.nlm` is itself a zip holding
# `<RELEASE>/NET/`.
NET="${NET:-references/umls/${RELEASE}/NET}"
UMLS_FULL_ZIP="${UMLS_FULL_ZIP:-references/umls-${RELEASE}-full.zip}"
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

# ── extract the Semantic Network (optional; only the FULL release carries it) ──
# Small — SRDEF 41 KB, SRSTR 30 KB — and it is the only source of the semantic-TYPE hierarchy
# (`isa` between types, plus SRDEF's tree numbers). MRSTY gives a concept's TYPES but says nothing
# about how those types relate, which is why `common_super` bottoms out at `lexicon:Entity` for
# every UMLS pair without this.
NET_FILES=(SRDEF SRFIL SRFLD SRSTR SRSTRE1 SRSTRE2 SU)
net_missing=0
for f in "${NET_FILES[@]}"; do [[ -f "$NET/$f" ]] || net_missing=1; done
if [[ "$net_missing" == "1" ]]; then
  if [[ -f "$UMLS_FULL_ZIP" ]]; then
    mkdir -p "$NET"
    tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
    echo ">> extracting the Semantic Network from $UMLS_FULL_ZIP → $NET"
    # `2026aa-otherks.nlm` is a zip-in-a-zip; pull it out, then take NET/ from it.
    otherks="$(unzip -Z1 "$UMLS_FULL_ZIP" '*otherks.nlm' 2>/dev/null | head -1)"
    if [[ -n "$otherks" ]]; then
      unzip -o -j "$UMLS_FULL_ZIP" "$otherks" -d "$tmp" >/dev/null
      unzip -o -j "$tmp/$(basename "$otherks")" "${RELEASE}/NET/*" -d "$NET" >/dev/null
      chmod 644 "$NET"/* 2>/dev/null || true
      echo ">> NET: $NET ($(ls -1 "$NET" | wc -l) files)"
    else
      echo ">> WARNING: no *otherks.nlm inside $UMLS_FULL_ZIP — Semantic Network not extracted" >&2
    fi
    rm -rf "$tmp"; trap - EXIT
  else
    echo ">> NET: skipped — no $UMLS_FULL_ZIP. The Semantic Network is NOT in the"
    echo "         metathesaurus-only archives; fetch the Full Release to get it."
  fi
else
  echo ">> NET: $NET"
fi

# ── convert + validate ────────────────────────────────────────────────
TUI_ARGS=()
if [[ -z "$ALL" ]]; then
  [[ ${#TUIS[@]} -gt 0 ]] || TUIS=("${DEFAULT_TUIS[@]}")
  for t in "${TUIS[@]}"; do TUI_ARGS+=(--semantic-type "$t"); done
  echo ">> semantic-type allowlist: ${TUIS[*]}"
else
  echo ">> importing ALL semantic types (this is large)"
fi

# CONVERT + LOAD.
#   - With --endpoint: emit a PARTITIONED chain (--out-dir) and load each file in
#     filename order. Full Level-0 (--all) is millions of concept classes / lexical
#     entries — far over the 128 MiB gRPC Load limit and too large to validate in
#     memory — so it MUST be chained; the kernel validates each layer at load time.
#   - Without --endpoint: emit a SINGLE document and self-validate in memory. Suitable
#     for a bounded subset; for --all prefer --endpoint (the in-memory validate of the
#     whole import will exhaust memory).
if [[ -n "$ENDPOINT" ]]; then
  OUTDIR="${OUTDIR:-umls-chain}"
  echo ">> converting (release=$RELEASE) → $OUTDIR/ (partitioned)"
  # shellcheck disable=SC2086
  cargo run -q -p eigenius-umls --bin umls-import -- \
    --meta-dir "$META" --version "$RELEASE" --out-dir "$OUTDIR" "${TUI_ARGS[@]}" $LIMIT
  echo ">> loading chain from $OUTDIR/ into eigenius serve @ $ENDPOINT"
  for f in "$OUTDIR"/umls-*.esl; do
    echo ">> loading $f"
    cargo run -q -p eigenius-cli -- --endpoint "http://$ENDPOINT" load "$f"
  done
else
  echo ">> converting (release=$RELEASE) → $OUT (single document)"
  # shellcheck disable=SC2086
  cargo run -q -p eigenius-umls --bin umls-import -- \
    --meta-dir "$META" --version "$RELEASE" --out "$OUT" --validate "${TUI_ARGS[@]}" $LIMIT
fi

echo ">> done."
