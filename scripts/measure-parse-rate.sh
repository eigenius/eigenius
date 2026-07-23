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
#   3. **--release is LOAD-BEARING.** A debug build does not merely run slower — it CHANGES THE
#      RESULT: debug stack frames are larger, NbE readback recursion overflows the stack, the parse
#      dies, and the harness reports it as a GRAMMAR-GAP indistinguishable from a real one. This
#      script always builds release and aborts if it sees a stack overflow.
#
# Usage:
#   scripts/measure-parse-rate.sh                    # CNL-v3 page, live LLM reranker, newest snapshot
#                                                    # → release build, log + provenance to experiments/parsing/,
#                                                    #   then evaluated by scripts/eval-parse-rate.sh
#   scripts/measure-parse-rate.sh --page original    # the raw OCR-cleaned first page
#   scripts/measure-parse-rate.sh --page cnl-v2      # the CNL v2 rewrite
#   scripts/measure-parse-rate.sh --page cnl         # the CNL v1 rewrite
#   scripts/measure-parse-rate.sh --page /abs/or/rel/path.txt
#   scripts/measure-parse-rate.sh --no-llm           # cap-only (no reranker) for an A/B
#   scripts/measure-parse-rate.sh --replay <ranks.json>  # replay a recorded run: NO LLM, deterministic
#   scripts/measure-parse-rate.sh --pos-prune        # ARM: cross-POS prune (GH#97) — CHANGES the result
#   scripts/measure-parse-rate.sh --combinatory-core # ARM: extra CCG combinators — CHANGES the grammar
#   scripts/measure-parse-rate.sh --attribution    # + page ambiguity roll-up (read-only; see README §7)
#   scripts/measure-parse-rate.sh --context-window # reranker sees +/-2 sentences — CHANGES the result (unproven)
#   scripts/measure-parse-rate.sh --snapshot /path/to/store
#
# Env overrides:
#   EIGENIUS_DB_SNAPSHOT  snapshot store dir (takes precedence over --snapshot / autodetect)
#   ANTHROPIC_API_KEY     required for the live reranker (unless --no-llm)
#   SNAPSHOT_ROOT         where to autodetect the newest snapshot (default: ../db-snapshot)
#   OUT_DIR               where run directories are written (default: experiments/parsing/results)
#
# Each run gets its OWN DIRECTORY under OUT_DIR, named for its identity
# (<stamp>-<commit>[-dirty]-<page>-<kind>[-arms]/), holding:
#   run.log     the harness output, led by a provenance header
#   ranks.json  every ranking the LLM reranker produced (replay it with --replay)
# `experiments/*/results/` is gitignored — the committed artifact is experiments/parsing/baseline.json.
#
# LOCKED DOWN — the script strips these from the environment and declares them itself, so an ambient
# value cannot silently change the grammar: EIGENIUS_POS_PRUNE, EIGENIUS_COMBINATORY_CORE,
# EIGENIUS_PARSE_DEBUG, EIGENIUS_DUMP_CELL. Select the arms with the flags above; the chosen config
# is written into the log header and into its filename.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ORIG_PWD="$PWD"
PAGES_DIR="$ROOT/references/publications/WRN-Helicase-Nature-OCR"
SNAPSHOT_ROOT="${SNAPSHOT_ROOT:-$ROOT/../db-snapshot}"
OUT_DIR="${OUT_DIR:-$ROOT/experiments/parsing/results}"

PAGE_ARG="cnl-v3"
SNAP="${EIGENIUS_DB_SNAPSHOT:-}"
USE_LLM=1
POS_PRUNE=0
COMB_CORE=0
ATTRIBUTION=0
CONTEXT_WINDOW=0
REPLAY=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --page)             PAGE_ARG="$2"; shift 2 ;;
    --snapshot)         SNAP="$2"; shift 2 ;;
    --no-llm)           USE_LLM=0; shift ;;
    --replay)           REPLAY="$2"; shift 2 ;;
    --pos-prune)        POS_PRUNE=1; shift ;;
    --combinatory-core) COMB_CORE=1; shift ;;
    --attribution)      ATTRIBUTION=1; shift ;;
    --context-window)   CONTEXT_WINDOW=2; shift ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

# ── LOCK DOWN THE ENVIRONMENT ────────────────────────────────────────────────
# Every knob below silently changes the RESULT. An ambient value inherited from the caller's shell
# would change the grammar with nothing in the log to say so — so the run declares each one
# explicitly and `env -u` strips anything not declared. A config is chosen HERE or not at all.
#
# `EIGENIUS_POS_PRUNE` is read with `.is_ok()`: ANY value, including the empty string, enables it.
# Setting it to "0" would turn it ON. It must be UNSET to be off — hence `env -u`, not `VAR=0`.
for v in EIGENIUS_POS_PRUNE EIGENIUS_COMBINATORY_CORE EIGENIUS_PARSE_DEBUG EIGENIUS_DUMP_CELL EIGENIUS_DUMP_RANK_PROMPT EIGENIUS_ATTRIBUTION_ROLLUP EIGENIUS_TRACE_ATTRIBUTION EIGENIUS_CONTEXT_SENTENCES; do
  if [[ -n "${!v:-}" ]]; then
    echo "note: ignoring ambient $v=${!v} — the run declares its own config (use the flags)" >&2
  fi
done
ENV_STRIP=(env -u EIGENIUS_POS_PRUNE -u EIGENIUS_COMBINATORY_CORE -u EIGENIUS_PARSE_DEBUG -u EIGENIUS_DUMP_CELL -u EIGENIUS_DUMP_RANK_PROMPT -u EIGENIUS_ATTRIBUTION_ROLLUP -u EIGENIUS_TRACE_ATTRIBUTION -u EIGENIUS_CONTEXT_SENTENCES)
[[ "$POS_PRUNE" == "1" ]] && ENV_STRIP+=(EIGENIUS_POS_PRUNE=1)
[[ "$COMB_CORE" == "1" ]] && ENV_STRIP+=(EIGENIUS_COMBINATORY_CORE=1)
# Read-only instrument: it observes the forest and does NOT change the parse (the four metrics are
# byte-identical with and without it) — but it is declared here like every other knob so an ambient
# value can never enter a run silently.
[[ "$ATTRIBUTION" == "1" ]] && ENV_STRIP+=(EIGENIUS_ATTRIBUTION_ROLLUP=1)
# Context window CHANGES the reranker's answer (and its ranks.json key) — declared, off unless armed.
[[ "$CONTEXT_WINDOW" != "0" ]] && ENV_STRIP+=(EIGENIUS_CONTEXT_SENTENCES="$CONTEXT_WINDOW")

# ── resolve the page (named shortcut → absolute; else realpath from the invocation dir) ──
case "$PAGE_ARG" in
  cnl-v3)   PAGE="$PAGES_DIR/first-page-cnl-v3.txt" ;;
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
# ABSOLUTIZE (gotcha #1, for the snapshot too): the test binary's CWD is the crate dir, not here, so
# a RELATIVE snapshot path silently "not found" from there → a 0.00s SKIP that reads as a pass. Done
# while CWD is still the invocation dir (the existence check above resolved SNAP against it).
SNAP="$(cd "$SNAP" && pwd)"

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

# ── Run identity ─────────────────────────────────────────────────────────────
# One DIRECTORY per run, named for its identity; both artifacts live inside it. `results/` is
# gitignored (large + regenerable); the committed artifact is experiments/parsing/baseline.json.
COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
DIRTY="$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
STAMP="$(date +%Y-%m-%d-%H%M)"
RUN_ID="${STAMP}-${COMMIT}"
[[ "$DIRTY" != "0" ]] && RUN_ID+="-dirty"
RUN_ID+="-$(basename "$PAGE" .txt)"
RUN_ID+="-$([[ "$USE_LLM" == "1" ]] && echo reranked || echo caponly)"
[[ -n "$REPLAY" ]]      && RUN_ID+="-replay"
[[ "$POS_PRUNE" == "1" ]] && RUN_ID+="-posprune"
[[ "$COMB_CORE" == "1" ]] && RUN_ID+="-combcore"
RUN_DIR="$OUT_DIR/$RUN_ID"
mkdir -p "$RUN_DIR"
LOG="$RUN_DIR/run.log"

# ── The reranker's decisions: RECORD, or REPLAY ──────────────────────────────
# The reranker is an LLM — the only component that can answer differently for the same code against
# the same store. Every run RECORDS its rankings; `--replay <file>` re-runs them with no LLM at all.
# That is what makes a measurement reproducible, and what lets a parser change be A/B'd against
# FIXED rankings (isolating the code from the model).
if [[ -n "$REPLAY" ]]; then
  [[ -f "$REPLAY" ]] || { echo "error: --replay file not found: $REPLAY" >&2; exit 1; }
  RANKS="$(cd "$(dirname "$REPLAY")" && pwd)/$(basename "$REPLAY")"
  RANKS_MODE="REPLAY $RANKS (deterministic, no LLM)"
elif [[ "$USE_LLM" == "1" ]]; then
  RANKS="$RUN_DIR/ranks.json"
  RANKS_MODE="RECORD → $RANKS"
else
  # --no-llm without --replay is CAP-ONLY: there is no live ranker to record, so pointing
  # EIGENIUS_SENSE_RANKS at a file that will never be written is meaningless — and the harness now
  # (correctly) treats "set but missing, no live ranker" as FATAL, because that is exactly how a run
  # silently degrades to cap-only with sense ELIMINATION off. Leave it UNSET for the honest cap-only arm.
  RANKS=""
  RANKS_MODE="none (cap-only: no reranker, no ranks file)"
fi
[[ -n "$RANKS" ]] && ENV_STRIP+=(EIGENIUS_SENSE_RANKS="$RANKS")

# ── Provenance header — a log without it cannot be reproduced or trusted ─────
KNOBS="$(grep -hoE 'const (SENSE_CAP|CELL_BEAM): usize = [0-9]+' \
  "$ROOT/crates/eigenius-wordnet/tests/db_backed_encoding.rs" \
  | sed -E 's/const ([A-Z_]+): usize = ([0-9]+)/\1=\2/' | tr '\n' ' ')"
{
  echo "# eigenius parse-rate experiment"
  echo "# run:       $RUN_ID"
  echo "# commit:    $COMMIT$([[ "$DIRTY" != "0" ]] && echo "  (WORKING TREE DIRTY — $DIRTY files; NOT reproducible)")"
  echo "# page:      $PAGE"
  echo "# snapshot:  $SNAP"
  echo "# reranker:  $RERANKER"
  echo "# profile:   release"
  echo "# config:    pos_prune=$POS_PRUNE combinatory_core=$COMB_CORE attribution=$ATTRIBUTION context_window=$CONTEXT_WINDOW $KNOBS"
  echo "# rust_min_stack: ${RUST_MIN_STACK:-default}"
  echo "# ranks:     $RANKS_MODE"
  echo "# started:   $(date -Iseconds)"
  echo "# command:   EIGENIUS_SENSE_RANKS=$RANKS EIGENIUS_DB_SNAPSHOT=$SNAP EIGENIUS_WRN_PAGE=$PAGE cargo test --release -p eigenius-wordnet ${FEATURES[*]} --test db_backed_encoding wrn_first_page_over_full_lexicon -- --ignored --nocapture"
  echo
} > "$LOG"

echo "run dir:  $RUN_DIR"
echo "ranks:    $RANKS_MODE"
echo "log:      $LOG"
echo

# --release is LOAD-BEARING, not an optimization. A debug build has larger stack frames, so NbE
# readback recursion overflows the stack, the parse dies, and the harness reports it as a
# GRAMMAR-GAP — indistinguishable from a real one. (2026-07-11: a debug run reported 12 phantom
# gaps + a stack overflow against a snapshot that measures grammar-gap 0 in release.)
set +e
"${ENV_STRIP[@]}" \
EIGENIUS_DB_SNAPSHOT="$SNAP" \
EIGENIUS_WRN_PAGE="$PAGE" \
  cargo test --release -p eigenius-wordnet "${FEATURES[@]}" --test db_backed_encoding \
    wrn_first_page_over_full_lexicon -- --ignored --nocapture 2>&1 | tee -a "$LOG"
STATUS=${PIPESTATUS[0]}
set -e

echo
if grep -q 'stack overflow' "$LOG"; then
  echo "FAIL: the run hit a STACK OVERFLOW — its gap count is not meaningful." >&2
  exit 1
fi
echo "=== evaluation ==="
"$ROOT/scripts/eval-parse-rate.sh" "$LOG" --baseline
exit "$STATUS"
