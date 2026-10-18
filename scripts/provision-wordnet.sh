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

# Provision WordNet 3.0 for the DCG / lexicon engine (D63): download → convert → load.
#
#   download  fetch WordNet 3.0 into references/WordNet-3.0/ (gitignored — WordNet is a
#             third-party corpus, not vendored in this repo; provisioned on demand).
#   convert   run the DETERMINISTIC importer (no LLM) → an Eigon-ESL lexicon document.
#             The output embeds WordNet content (glosses/lemmas) and so carries the
#             WordNet 3.0 license notice at its head (see crates/eigenius-wordnet).
#   load      `--validate` (compile + felicity-gate = an in-memory load proof) by default;
#             with --endpoint, ALSO persist the layer into a running `eigenius serve`.
#
# Usage:
#   scripts/provision-wordnet.sh                      # download + convert + validate
#   scripts/provision-wordnet.sh --seed gene --seed depend   # a small SEEDED slice (fast)
#   scripts/provision-wordnet.sh --endpoint 127.0.0.1:50051  # ... + load into a service
#
# Env overrides:
#   WORDNET_URL     download URL (default: canonical Princeton; override for a mirror)
#   WORDNET_SHA256  if set, verify the tarball's SHA-256 before extracting
#   REFDIR          where to extract WordNet (default: references)
#   OUT             ESL output path (default: wordnet-full.esl, gitignored)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

WORDNET_URL="${WORDNET_URL:-https://wordnetcode.princeton.edu/3.0/WordNet-3.0.tar.gz}"
REFDIR="${REFDIR:-references}"
DICT="$REFDIR/WordNet-3.0/dict"
OUT="${OUT:-wordnet-full.esl}"

# Bound: --all by default; --seed/--limit pass through to the importer for a small slice.
IMPORT_ARGS=(--all)
ENDPOINT=""
explicit_bound=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint) ENDPOINT="$2"; shift 2 ;;
    --seed)     [[ $explicit_bound == 0 ]] && IMPORT_ARGS=(); explicit_bound=1; IMPORT_ARGS+=(--seed "$2"); shift 2 ;;
    --limit)    [[ $explicit_bound == 0 ]] && IMPORT_ARGS=(); explicit_bound=1; IMPORT_ARGS+=(--limit "$2"); shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

# 1. DOWNLOAD (idempotent — skip if the dict is already present).
if [[ -d "$DICT" ]]; then
  echo "wordnet: dict already present at $DICT — skipping download"
else
  echo "wordnet: downloading $WORDNET_URL"
  mkdir -p "$REFDIR"
  tmp="$(mktemp)"
  trap 'rm -f "$tmp"' EXIT
  curl -fSL "$WORDNET_URL" -o "$tmp"
  if [[ -n "${WORDNET_SHA256:-}" ]]; then
    echo "wordnet: verifying SHA-256"
    echo "${WORDNET_SHA256}  ${tmp}" | sha256sum -c -
  fi
  tar -xzf "$tmp" -C "$REFDIR"
  if [[ ! -d "$DICT" ]]; then
    echo "error: $DICT not found after extracting the tarball — check WORDNET_URL/layout" >&2
    exit 1
  fi
  echo "wordnet: extracted → $REFDIR/WordNet-3.0/"
fi

# 2. CONVERT + LOAD.
#    - With --endpoint: emit a PARTITIONED chain (--out-dir) and load each file in
#      filename order. The full lexicon (~165 MB) exceeds the kernel's 128 MiB gRPC
#      Load limit as a single document, so it MUST be chained; the kernel validates
#      each layer at load time (the chain is the validation context).
#    - Without --endpoint: emit a SINGLE document and self-validate in memory (a
#      compile + felicity-gate load proof). The full import is ~325k entries.
if [[ -n "$ENDPOINT" ]]; then
  OUTDIR="${OUTDIR:-wordnet-chain}"
  echo "wordnet: converting → $OUTDIR/  (partitioned; importer args: ${IMPORT_ARGS[*]})"
  cargo run --release -p eigenius-wordnet --bin wordnet-import -- \
    "${IMPORT_ARGS[@]}" --dict "$DICT" --out-dir "$OUTDIR"
  echo "wordnet: loading chain from $OUTDIR/ into eigenius service at $ENDPOINT"
  for f in "$OUTDIR"/wordnet-*.esl; do
    echo ">> loading $f"
    cargo run --release -p eigenius-cli --bin eigenius -- --endpoint "http://$ENDPOINT" load "$f"
  done
else
  echo "wordnet: converting → $OUT  (single document; importer args: ${IMPORT_ARGS[*]} --validate)"
  cargo run --release -p eigenius-wordnet --bin wordnet-import -- \
    "${IMPORT_ARGS[@]}" --dict "$DICT" --out "$OUT" --validate
fi

echo "wordnet: done → $OUT"
