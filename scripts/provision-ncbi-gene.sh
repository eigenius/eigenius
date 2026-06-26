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

# Provision the NCBI Gene domain lexicon (D65 §5): download → convert → load.
#
#   download  fetch NCBI Gene's gene_info for one organism into references/ncbi/
#             (gitignored — NCBI data is third-party, provisioned on demand; it is
#             a U.S. Government public-domain work).
#   convert   run the DETERMINISTIC importer (no LLM) → an Eigon-ESL document: a
#             typed mirror (ncbi:Gene witnesses) + a derived lexicon (lexicon:ncbi_gene).
#   load      `--validate` (compile + felicity-gate = an in-memory load proof) by default;
#             with --endpoint, ALSO persist the layer into a running `eigenius serve`.
#
# Usage:
#   scripts/provision-ncbi-gene.sh                          # human: download + convert + validate
#   scripts/provision-ncbi-gene.sh --wordnet-anchor         # also emit ncbi:Gene ⊑ wn:gene.n.01
#                                                           #   (only valid on a chain with WordNet)
#   scripts/provision-ncbi-gene.sh --endpoint 127.0.0.1:50051   # ... + load into a service
#
# Env overrides:
#   GENE_INFO_URL  download URL (default: canonical NCBI FTP, Homo sapiens)
#   TAX_ID         NCBI Taxonomy id to keep (default: 9606 = human)
#   REFDIR         where to place the dump (default: references/ncbi)
#   OUT            ESL output path (default: ncbi-gene.esl, gitignored)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TAX_ID="${TAX_ID:-9606}"
REFDIR="${REFDIR:-references/ncbi}"
OUT="${OUT:-ncbi-gene.esl}"
GENE_INFO_URL="${GENE_INFO_URL:-https://ftp.ncbi.nih.gov/gene/DATA/GENE_INFO/Mammalia/Homo_sapiens.gene_info.gz}"

ENDPOINT=""
WORDNET_ANCHOR=""
LIMIT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint) ENDPOINT="$2"; shift 2 ;;
    --wordnet-anchor) WORDNET_ANCHOR="--wordnet-anchor"; shift ;;
    --limit) LIMIT="--limit $2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

GENE_INFO="$REFDIR/Homo_sapiens.gene_info"

# ── download ──────────────────────────────────────────────────────────
if [[ ! -f "$GENE_INFO" ]]; then
  mkdir -p "$REFDIR"
  echo ">> downloading gene_info: $GENE_INFO_URL"
  curl -fSL "$GENE_INFO_URL" --output "$GENE_INFO.gz"
  echo ">> decompressing → $GENE_INFO"
  gunzip -f "$GENE_INFO.gz"
fi
echo ">> gene_info: $GENE_INFO ($(wc -l < "$GENE_INFO") lines)"

# ── convert + validate ────────────────────────────────────────────────
echo ">> converting (tax_id=$TAX_ID) → $OUT"
# shellcheck disable=SC2086
cargo run -q -p eigenius-ncbi-gene --bin ncbi-gene-import -- \
  --gene-info "$GENE_INFO" --tax-id "$TAX_ID" --out "$OUT" --validate $WORDNET_ANCHOR $LIMIT

# ── load into a running service (optional) ─────────────────────────────
if [[ -n "$ENDPOINT" ]]; then
  echo ">> loading into eigenius serve @ $ENDPOINT"
  cargo run -q -p eigenius-cli -- --endpoint "http://$ENDPOINT" load "$OUT"
fi

echo ">> done."
