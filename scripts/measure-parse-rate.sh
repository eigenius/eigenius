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

# Measure the parse / encode success rate of the DCG parser over a page of prose, over the FULL
# WordNet+UMLS lexicon, optionally with the live Anthropic sense reranker (the D62 (d) measurement).
#
# Drives the `wrn_first_page_over_full_lexicon` harness
# (crates/eigenius-wordnet/tests/db_backed_encoding.rs): it segments the page into units and
# classifies each — ENCODED / AMBIG / OPEN / MISSING-LEXEME / GRAMMAR-GAP / SCALE-BOUND — then prints
# a summary line and the distinct-OOV list. The reranker line ("contextual reranker: …") reports
# whether the live LLM engaged.
#
# Requires a reseeded snapshot (scripts/reseed-lexicon-db.sh): the persisted chain is rooted at the
# bootstrap it was seeded with (content hashes), so after any bootstrap-ontology edit the harness
# fail-closed SKIPs on ManifestDrift until you reseed. This script autodetects the newest snapshot.
#
# Two gotchas this script handles so you don't rediscover them:
#   1. EIGENIUS_WRN_PAGE must be ABSOLUTE — the test binary's CWD is the crate dir, not the repo root,
#      so a relative page path silently "not found" → a 0.00s SKIP that looks like a pass.
#   2. The live reranker needs BOTH `--features use-llm` AND ANTHROPIC_API_KEY; without the key the
#      harness runs cap-only and silently reports "reranker: none".
#
# Usage:
#   scripts/measure-parse-rate.sh                    # CNL-v2 page, live LLM reranker, newest snapshot
#   scripts/measure-parse-rate.sh --page original    # the raw OCR-cleaned first page
#   scripts/measure-parse-rate.sh --page cnl         # the CNL v1 rewrite
#   scripts/measure-parse-rate.sh --page /abs/or/rel/path.txt
#   scripts/measure-parse-rate.sh --no-llm           # cap-only (no reranker) for an A/B
#   scripts/measure-parse-rate.sh --snapshot /path/to/store
#
# Env overrides:
#   EIGENIUS_DB_SNAPSHOT  snapshot store dir (takes precedence over --snapshot / autodetect)
#   ANTHROPIC_API_KEY     required for the live reranker (unless --no-llm)
#   SNAPSHOT_ROOT         where to autodetect the newest snapshot (default: ../db-snapshot)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORIG_PWD="$PWD"
PAGES_DIR="$ROOT/references/publications/WRN-Helicase-Nature-OCR"
SNAPSHOT_ROOT="${SNAPSHOT_ROOT:-$ROOT/../db-snapshot}"

PAGE_ARG="cnl-v2"
SNAP="${EIGENIUS_DB_SNAPSHOT:-}"
USE_LLM=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --page)     PAGE_ARG="$2"; shift 2 ;;
    --snapshot) SNAP="$2"; shift 2 ;;
    --no-llm)   USE_LLM=0; shift ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

# ── resolve the page (named shortcut → absolute; else realpath from the invocation dir) ──
case "$PAGE_ARG" in
  cnl-v2)   PAGE="$PAGES_DIR/first-page-cnl-v2.txt" ;;
  cnl)      PAGE="$PAGES_DIR/first-page-cnl.txt" ;;
  original) PAGE="$PAGES_DIR/first-page-cleaned.txt" ;;
  /*)       PAGE="$PAGE_ARG" ;;
  *)        PAGE="$(cd "$ORIG_PWD" && realpath "$PAGE_ARG" 2>/dev/null || echo "$PAGE_ARG")" ;;
esac
[[ -f "$PAGE" ]] || { echo "error: page not found: $PAGE" >&2; exit 1; }

# ── resolve the snapshot (env/arg → newest under SNAPSHOT_ROOT) ──
if [[ -z "$SNAP" ]]; then
  SNAP="$(ls -1dt "$SNAPSHOT_ROOT"/wordnet-umls-* 2>/dev/null | head -1 || true)"
fi
[[ -n "$SNAP" && -f "$SNAP/CURRENT" ]] || {
  echo "error: no RocksDB snapshot found (looked under $SNAPSHOT_ROOT for wordnet-umls-*; run scripts/reseed-lexicon-db.sh)" >&2
  exit 1
}

# ── reranker wiring ──
FEATURES=()
if [[ "$USE_LLM" == "1" ]]; then
  [[ -n "${ANTHROPIC_API_KEY:-}" ]] || {
    echo "error: live reranker requested but ANTHROPIC_API_KEY is unset (pass --no-llm for cap-only)" >&2
    exit 1
  }
  FEATURES=(--features use-llm)
  RERANKER="live Anthropic reranker (--features use-llm)"
else
  RERANKER="cap-only (no reranker)"
fi

cd "$ROOT"
echo "=== parse-rate measurement ==="
echo "page:     $PAGE"
echo "snapshot: $SNAP"
echo "reranker: $RERANKER"
echo

EIGENIUS_DB_SNAPSHOT="$SNAP" \
EIGENIUS_WRN_PAGE="$PAGE" \
  cargo test -p eigenius-wordnet "${FEATURES[@]}" --test db_backed_encoding \
    wrn_first_page_over_full_lexicon -- --ignored --nocapture
