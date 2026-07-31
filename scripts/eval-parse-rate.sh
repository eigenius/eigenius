#!/usr/bin/env bash
#
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

# Evaluate a parse-rate run log (from scripts/measure-parse-rate.sh) — the SCORING half of the
# experiment. Extracts the outcome metrics, validates the run was trustworthy, and compares against
# the reference baseline.
#
# Why this exists as a separate, scripted step: reading these numbers by eye is how they get read
# WRONG. Three specific traps, each of which has silently produced a false result:
#
#   1. `grammar-gap` MUST come from the `=== WRN first page over FULL lexicon: … ===` summary line.
#      The per-unit listing enumerates only AMBIG units and OMITS grammar gaps — counting from it
#      reports 0 gaps on a run that had many.
#   2. A run that never printed a summary line DID NOT COMPLETE (stack overflow / abort / SKIP).
#      Its partial counts are not a result. This script refuses to score it.
#   3. A cap-only run (no `--features use-llm`) inflates gaps by construction and is NOT comparable
#      to a reranked one. This script reports which it was and refuses to compare across kinds.
#
# Usage:
#   scripts/eval-parse-rate.sh <run.log>              # score one run
#   scripts/eval-parse-rate.sh <run.log> <base.log>   # score, and diff against a baseline run
#   scripts/eval-parse-rate.sh --baseline <run.log>   # score against the committed reference
#
# Exit: 0 = the run is valid AND meets the baseline; 1 = invalid run; 2 = regression vs baseline.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The committed reference. Run LOGS are gitignored (experiments/*/results/), so the baseline is a
# small distilled JSON that travels with the repo — the gate must survive a clean checkout.
BASELINE_JSON="$ROOT/experiments/parsing/baseline.json"

# Args in any order. An unrecognized argument is an ERROR, never silently ignored — a flag that is
# quietly dropped is how a comparison silently does not happen.
BASE=""
USE_JSON=0
LOG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --baseline) USE_JSON=1; shift ;;
    -*)         echo "error: unknown flag: $1" >&2; exit 2 ;;
    *)
      if [[ -z "$LOG" ]]; then LOG="$1"; else BASE="$1"; fi
      shift ;;
  esac
done
[[ -n "$LOG" ]] || { echo "usage: $0 <run.log> [--baseline | <other-run.log>]" >&2; exit 2; }
[[ -f "$LOG" ]] || { echo "error: no such log: $LOG" >&2; exit 2; }

# ── Trap 2: did the run COMPLETE? ────────────────────────────────────────────
summary_of() { grep -m1 -E '^=== WRN first page over FULL lexicon' "$1" || true; }

SUMMARY="$(summary_of "$LOG")"
if [[ -z "$SUMMARY" ]]; then
  echo "INVALID: no summary line — the run did not complete."
  if grep -q 'stack overflow' "$LOG"; then
    echo "  cause: STACK OVERFLOW. Almost always a DEBUG build — rebuild with --release."
  elif grep -qi 'SKIP' "$LOG"; then
    echo "  cause: $(grep -m1 -i 'SKIP' "$LOG")"
  fi
  echo "  Partial per-unit counts are NOT a result. Do not report them."
  exit 1
fi

# ── Metrics come from the SUMMARY LINE ONLY (trap 1) ─────────────────────────
field() { sed -E "s/.*$1 ([0-9]+).*/\1/" <<<"$2"; }
units()  { sed -E 's/.*: ([0-9]+) units.*/\1/' <<<"$1"; }

U=$(units "$SUMMARY")
ENC=$(field 'encoded'        "$SUMMARY")
AMB=$(field 'ambiguous'      "$SUMMARY")
OPN=$(field 'open'           "$SUMMARY")
MIS=$(field 'missing-lexeme' "$SUMMARY")
GAP=$(field 'grammar-gap'    "$SUMMARY")
# total-readings (the multiplicity signal). Absent on logs from before it was wired into the harness
# (2026-07-17); a non-numeric extraction means "not present", handled downstream.
TR=$(field 'total-readings'  "$SUMMARY"); [[ "$TR" =~ ^[0-9]+$ ]] || TR=""
# total-skeletons (the DRIFT-FREE structural multiplicity signal — senses erased, so the reranker's
# sense choices cannot mask a structural change; the tracked lever, less cap/sense-entangled than
# total-readings). Absent on logs from before it was wired (2026-07-19).
TS=$(field 'total-skeletons' "$SUMMARY"); [[ "$TS" =~ ^[0-9]+$ ]] || TS=""
# expected-hits / expected-curated (the FAITHFULNESS signal — how many curated units still contain
# their verified-correct reading). Replaces encoded-count, which a correctness fix can lower (a unit
# encoded on a WRONG reading becomes ambiguous once the right one is restored). Absent on old logs.
EH=$(field 'expected-hits'    "$SUMMARY"); [[ "$EH" =~ ^[0-9]+$ ]] || EH=""
EC=$(field 'expected-curated' "$SUMMARY"); [[ "$EC" =~ ^[0-9]+$ ]] || EC=""

# ── Trap 3: which KIND of run was this? ──────────────────────────────────────
RERANK="$(grep -m1 -E '^contextual reranker:' "$LOG" | sed 's/^contextual reranker: //' || echo 'unknown')"
KIND=caponly; grep -qE '^contextual reranker: AnthropicSenseRanker \(live\)' "$LOG" && KIND=reranked
PROFILE=unknown
grep -qE 'Finished .release. profile' "$LOG" && PROFILE=release
grep -qE 'Finished .(dev|test). profile' "$LOG" && PROFILE=debug
SECS="$(grep -m1 -oE 'finished in [0-9.]+s' "$LOG" | grep -oE '[0-9.]+' || echo '?')"
COMMIT="$(grep -m1 -E '^# commit:' "$LOG" | sed 's/^# commit: *//' || echo 'not recorded')"
CONFIG="$(grep -m1 -E '^# config:' "$LOG" | sed 's/^# config: *//' || echo 'not recorded')"

printf '  %-16s %s\n' "commit"   "$COMMIT"
printf '  %-16s %s\n' "profile"  "$PROFILE"
printf '  %-16s %s\n' "reranker" "$RERANK"
printf '  %-16s %s\n' "config"   "$CONFIG"
printf '  %-16s %ss\n' "runtime" "$SECS"
echo
printf '  %-16s %s\n' "units"          "$U"
printf '  %-16s %s\n' "grammar-gap"    "$GAP"
printf '  %-16s %s\n' "missing-lexeme" "$MIS"
printf '  %-16s %s\n' "ambiguous"      "$AMB"
printf '  %-16s %s\n' "open"           "$OPN"
printf '  %-16s %s\n' "encoded"        "$ENC"
[[ -n "$TR" ]] && printf '  %-16s %s\n' "total-readings"  "$TR"
[[ -n "$TS" ]] && printf '  %-16s %s\n' "total-skeletons" "$TS"
echo

# Reading-count histogram — emitted by the harness with PINNED buckets (READING_BUCKETS in
# db_backed_encoding.rs). Surfaced verbatim so the buckets never drift between the run and the score.
if grep -qE '^  histogram:' "$LOG"; then
  echo "  reading-count histogram (pinned buckets):"
  grep -E '^  histogram:' "$LOG" | sed -E 's/^  histogram: */    /'
  echo
fi

RC=0
if [[ "$PROFILE" == "debug" ]]; then
  echo "  UNTRUSTWORTHY: DEBUG build. Debug stack frames overflow in NbE readback, killing parses"
  echo "                 that are then reported as GRAMMAR-GAPs. Re-run with --release."
  exit 1
fi

# ── Coverage gate: every sentence must parse ─────────────────────────────────
if [[ "$GAP" -eq 0 && "$MIS" -eq 0 ]]; then
  echo "  COVERAGE: PASS — every unit parses (grammar-gap 0, missing-lexeme 0)."
else
  echo "  COVERAGE: FAIL — grammar-gap $GAP, missing-lexeme $MIS."
  RC=2
fi

# ── Faithfulness: the open goal — how many resolve to ONE reading ────────────
echo "  RESOLUTION: $ENC/$U encoded (single reading); $AMB ambiguous, $OPN open."

# ── Baseline comparison against the COMMITTED reference ──────────────────────
if [[ "$USE_JSON" == "1" ]]; then
  [[ -f "$BASELINE_JSON" ]] || { echo "  error: no $BASELINE_JSON" >&2; exit 1; }
  BGAP=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected']['grammar_gap'])")
  BENC=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected']['encoded'])")
  BUNI=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected']['units'])")
  echo
  echo "  vs committed baseline (experiments/parsing/baseline.json):"
  [[ "$U" -ne "$BUNI" ]] && echo "    NOTE: unit count differs ($BUNI → $U) — a different page or segmentation."
  printf '    %-16s %s → %s %s\n' "grammar-gap" "$BGAP" "$GAP" \
    "$([[ "$GAP" -gt "$BGAP" ]] && echo '  REGRESSION' || { [[ "$GAP" -lt "$BGAP" ]] && echo '  improved' || echo '  (unchanged)'; })"
  # encoded is INFORMATIONAL, not gated: a correctness fix can lower it (a unit encoded on the WRONG
  # reading becomes ambiguous when the right reading is restored — the 2026-07-20 gloss bug). The
  # faithfulness gate is expected-hits below.
  printf '    %-16s %s → %s %s\n' "encoded (info)" "$BENC" "$ENC" \
    "$([[ "$ENC" -lt "$BENC" ]] && echo '  lower (not gated — see expected-hits)' || { [[ "$ENC" -gt "$BENC" ]] && echo '  higher' || echo '  (unchanged)'; })"
  [[ "$GAP" -gt "$BGAP" ]] && RC=2

  # ── Faithfulness gate: no curated unit may LOSE its expected (verified-correct) reading ──────────
  BEH=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected'].get('expected_reading_hits',0))")
  BEC=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected'].get('expected_reading_curated',0))")
  if [[ -n "$EH" ]]; then
    if [[ "$EH" -lt "$BEH" ]]; then
      printf '    %-16s %s → %s   REGRESSION (a curated unit lost its expected reading)\n' \
        "expected-hits" "$BEH" "$EH"; RC=2
    elif [[ -n "$EC" && "$EC" -lt "$BEC" ]]; then
      printf '    %-16s %s → %s   REGRESSION (curated set shrank %s → %s)\n' \
        "expected-hits" "$BEH" "$EH" "$BEC" "$EC"; RC=2
    else
      printf '    %-16s %s/%s → %s/%s%s\n' "expected-hits" "$BEH" "$BEC" "$EH" "$EC" \
        "$([[ "$EH" -gt "$BEH" ]] && echo '   more coverage' || echo '   (all hold)')"
    fi
  else
    echo "    expected-hits    (not in this log — predates the faithfulness gate; re-measure)"
  fi

  # ── Multiplicity gate: total_readings must not exceed the ceiling (over-generation must not grow) ─
  BCEIL=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected'].get('total_readings_ceiling',0))")
  BTR=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected'].get('total_readings',0))")
  if [[ -n "$TR" ]]; then
    if [[ "$TR" -gt "$BCEIL" ]]; then
      printf '    %-16s %s → %s   REGRESSION (> ceiling %s)\n' "total-readings" "$BTR" "$TR" "$BCEIL"
      RC=2
    else
      printf '    %-16s %s → %s%s (ceiling %s)\n' "total-readings" "$BTR" "$TR" \
        "$([[ "$TR" -lt "$BTR" ]] && echo '   improved' || { [[ "$TR" -gt "$BTR" ]] && echo '   rose (within ceiling)' || echo '   (unchanged)'; })" "$BCEIL"
    fi
  else
    echo "    total-readings   (not in this log — harness predates the metric; re-measure to gate it)"
  fi

  # ── Structural-multiplicity gate: total-skeletons (drift-free, sense-erased) must not exceed the
  #    ceiling. THE CLEAN LEVER — total-readings sense-multiplies structure and the reranker masks real
  #    wins (M3/RNR both fell in structure while reranked total_readings rose), so a skeleton RISE is the
  #    true over-generation signal. See gates.multiplicity.
  BSCEIL=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected'].get('skeletons_ceiling',0))")
  BTS=$(python3 -c "import json;print(json.load(open('$BASELINE_JSON'))['expected'].get('skeletons',0))")
  if [[ -n "$TS" && "$BSCEIL" -gt 0 ]]; then
    if [[ "$TS" -gt "$BSCEIL" ]]; then
      printf '    %-16s %s → %s   REGRESSION (> ceiling %s)\n' "total-skeletons" "$BTS" "$TS" "$BSCEIL"
      RC=2
    else
      printf '    %-16s %s → %s%s (ceiling %s)\n' "total-skeletons" "$BTS" "$TS" \
        "$([[ "$TS" -lt "$BTS" ]] && echo '   improved' || { [[ "$TS" -gt "$BTS" ]] && echo '   rose (within ceiling)' || echo '   (unchanged)'; })" "$BSCEIL"
    fi
  elif [[ -z "$TS" ]]; then
    echo "    total-skeletons  (not in this log — harness predates the metric; re-measure to gate it)"
  fi
fi

# ── Or against another RUN's log (like-for-like only) ────────────────────────
if [[ -n "$BASE" && -f "$BASE" ]]; then
  BSUM="$(summary_of "$BASE")"
  if [[ -z "$BSUM" ]]; then
    echo "  baseline $BASE did not complete — cannot compare."
    exit 1
  fi
  BKIND=caponly; grep -qE '^contextual reranker: AnthropicSenseRanker \(live\)' "$BASE" && BKIND=reranked
  BCONFIG="$(grep -m1 -E '^# config:' "$BASE" | sed 's/^# config: *//' || echo 'not recorded')"
  echo
  if [[ "$KIND" != "$BKIND" ]]; then
    echo "  NOT COMPARABLE: this run is '$KIND', baseline is '$BKIND'."
    echo "  A cap-only run inflates gaps by construction. Compare like with like."
    exit 1
  fi
  # The knobs (pos_prune / combinatory_core / SENSE_CAP / CELL_BEAM) change the RESULT. Two runs on
  # different configs are two different experiments, and diffing them is meaningless.
  if [[ "$CONFIG" != "$BCONFIG" ]]; then
    echo "  NOT COMPARABLE: config differs."
    echo "    this run : $CONFIG"
    echo "    baseline : $BCONFIG"
    exit 1
  fi
  BGAP=$(field 'grammar-gap' "$BSUM"); BENC=$(field 'encoded' "$BSUM")
  echo "  vs baseline ($(basename "$BASE")):"
  printf '    %-16s %s → %s %s\n' "grammar-gap" "$BGAP" "$GAP" \
    "$([[ "$GAP" -gt "$BGAP" ]] && echo '  REGRESSION' || { [[ "$GAP" -lt "$BGAP" ]] && echo '  improved' || echo '  (unchanged)'; })"
  # encoded informational (see the --baseline path); faithfulness is expected-hits.
  printf '    %-16s %s → %s %s\n' "encoded (info)" "$BENC" "$ENC" \
    "$([[ "$ENC" -lt "$BENC" ]] && echo '  lower (not gated)' || { [[ "$ENC" -gt "$BENC" ]] && echo '  higher' || echo '  (unchanged)'; })"
  BEH2=$(field 'expected-hits' "$BSUM")
  [[ "$EH" =~ ^[0-9]+$ && "$BEH2" =~ ^[0-9]+$ ]] && {
    printf '    %-16s %s → %s %s\n' "expected-hits" "$BEH2" "$EH" \
      "$([[ "$EH" -lt "$BEH2" ]] && echo '  REGRESSION' || echo '  (all hold)')"
    [[ "$EH" -lt "$BEH2" ]] && RC=2
  }
  [[ "$GAP" -gt "$BGAP" ]] && RC=2
fi

exit "$RC"
