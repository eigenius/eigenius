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

# SKELETON ADJUDICATION LEDGER — the VALIDITY gate.
#
# `expected-readings.tsv` gates FAITHFULNESS: is the correct reading still PRESENT? It says nothing
# about the OTHER readings, so a unit can pass it while carrying invalid ones. This script gates the
# complementary question: is EVERY reading the parser produces one we have adjudicated?
#
# Two distinct questions, and conflating them is how an informal review drifts:
#   faithfulness  the correct reading is present   -> expected-readings.tsv  (a pin per unit)
#   validity      every present reading is judged  -> adjudications.tsv      (a verdict per skeleton)
#
# `correct` IS NOT RECORDED HERE. `expected-readings.tsv` already asserts, per unit, which skeleton is
# the intended reading — that is exactly a `correct` verdict, adjudicated with a note. Copying those
# into this ledger would give the same fact two sources of truth that can drift, so the script READS
# the pin file and treats each pin as the unit's `correct`. This ledger holds the OTHER readings.
#
# THE VERDICTS. Exactly one of:
#   correct    the intended reading — taken from `expected-readings.tsv`, not written here.
#   available  structurally available and NOT something the grammar should refuse — semantically
#              false, or true but dispreferred. Ranking's problem, not the grammar's. A PP that could
#              attach high or low gives one `correct` and one `available`, not an `invalid`.
#   invalid    the grammar should NOT produce it. The evidence field MUST say why it is inadmissible
#              under EVERY sense assignment, and name the rule or mechanism that should block it.
#              This is the defect backlog; each row is a rule change waiting to be written.
#
# The `invalid`/`available` line is the one that decides whether a finding is a rule change or a
# ranking change, so it is the field to argue over. "Looks wrong" is not a verdict — a reading that
# is merely false is `available` unless it is false under every sense assignment.
#
# FAIL-CLOSED. An UNADJUDICATED skeleton is a finding, not a pass: the run reports non-zero and
# prints a worksheet of the missing rows, ready to fill in. So any grammar change that introduces a
# new reading surfaces here instead of being absorbed into a count. STALE rows (a verdict whose
# skeleton the parser no longer produces) are reported too — usually the trace of a fixed defect,
# and worth deleting deliberately rather than leaving to rot.
#
# Usage:
#   scripts/audit-skeletons.sh                      # report coverage
#   scripts/audit-skeletons.sh --worksheet out.tsv  # also write the unadjudicated rows to fill in
#   scripts/audit-skeletons.sh --unit 'Some cancers do not'   # restrict to matching units
#
# Env: EIGENIUS_DB_SNAPSHOT and EIGENIUS_SENSE_RANKS are BOTH REQUIRED and must match the baseline's
# snapshot and recording — the ledger is keyed on skeleton strings, and a different sense
# configuration yields a different skeleton set (225 under the tracked replay, 531 cap-only), which
# would report the entire ledger as stale. EIGENIUS_WRN_PAGE defaults to the reference page.
# `--release` is load-bearing (see scripts/measure-parse-rate.sh).
#
# Exit: 0 fully adjudicated, 1 unadjudicated rows remain, 2 malformed ledger / duplicate `correct`.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LEDGER="${LEDGER:-experiments/parsing/adjudications.tsv}"
WORKSHEET=""
UNIT_FILTER=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --worksheet) WORKSHEET="$2"; shift 2 ;;
    --unit)      UNIT_FILTER="$2"; shift 2 ;;
    --ledger)    LEDGER="$2"; shift 2 ;;
    *) echo "error: unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ -n "${EIGENIUS_DB_SNAPSHOT:-}" ]] || { echo "error: EIGENIUS_DB_SNAPSHOT must be set (absolute)" >&2; exit 2; }
# The ledger is keyed on SKELETON STRINGS, and the skeleton SET depends on the sense configuration:
# the same page yields 225 skeletons under the tracked replay and 531 cap-only (measured 2026-07-28,
# and it silently invalidated every row on the first run of this script). So the recording is
# REQUIRED, not optional — auditing under a different configuration than the baseline compares
# against a different corpus and reports the whole ledger as stale.
[[ -n "${EIGENIUS_SENSE_RANKS:-}" ]] || {
  echo "error: EIGENIUS_SENSE_RANKS must be set to the SAME recording the baseline uses" >&2
  echo "       (experiments/parsing/ranks/<recording>.json, absolute path)." >&2
  echo "       Without it the harness runs cap-only and produces a DIFFERENT skeleton set —" >&2
  echo "       225 vs 531 on the reference page — so every ledger row would read as stale." >&2
  exit 2
}
: "${EIGENIUS_WRN_PAGE:=$ROOT/references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v3.txt}"
export EIGENIUS_WRN_PAGE

DUMP="$(mktemp)"; trap 'rm -f "$DUMP"' EXIT
echo ">> enumerating skeletons (release; this parses the whole page)" >&2
EIGENIUS_DUMP_SKELETONS=1 cargo test --release -p eigenius-wordnet --test db_backed_encoding \
  wrn_first_page_over_full_lexicon -- --ignored --nocapture 2>&1 \
  | sed -n '/PER-UNIT SKELETONS/,/END SKELETONS/p' > "$DUMP"

LEDGER="$LEDGER" WORKSHEET="$WORKSHEET" UNIT_FILTER="$UNIT_FILTER" DUMP="$DUMP" python3 - <<'PY'
import io, os, re, sys, collections

dump, ledger = os.environ["DUMP"], os.environ["LEDGER"]
worksheet, unit_filter = os.environ["WORKSHEET"], os.environ["UNIT_FILTER"]

# (unit, skeleton) pairs the parser actually produces
produced, cur = collections.OrderedDict(), None
for ln in io.open(dump, encoding="utf-8"):
    m = re.match(r'^«(.*)»\s+\[(\d+) skeleton', ln)
    if m:
        cur = m.group(1); produced.setdefault(cur, [])
    elif cur is not None and ln.startswith("    "):
        produced[cur].append(ln.strip())
if unit_filter:
    produced = collections.OrderedDict((u, s) for u, s in produced.items() if unit_filter in u)

# `correct` verdicts come from the PIN FILE, which already adjudicates one skeleton per unit.
pins = "experiments/parsing/expected-readings.tsv"
verdicts, malformed = {}, []
if os.path.exists(pins):
    for ln in io.open(pins, encoding="utf-8"):
        if ln.startswith("#") or not ln.strip():
            continue
        f = ln.rstrip("\n").split("\t")
        if len(f) >= 2:
            verdicts[(f[0].strip(), f[1].strip())] = ("correct", "pinned in expected-readings.tsv")
n_pins = len(verdicts)
VALID = {"available", "invalid"}
if os.path.exists(ledger):
    for i, ln in enumerate(io.open(ledger, encoding="utf-8"), 1):
        if ln.startswith("#") or not ln.strip():
            continue
        f = ln.rstrip("\n").split("\t")
        if len(f) < 4 or f[2].strip() not in VALID:
            malformed.append((i, ln.strip()[:90])); continue
        verdicts[(f[0].strip(), f[1].strip())] = (f[2].strip(), f[3].strip())

n_units = len(produced)
n_sk = sum(len(v) for v in produced.values())
adj = [(u, s) for u, ss in produced.items() for s in ss if (u, s) in verdicts]
una = [(u, s) for u, ss in produced.items() for s in ss if (u, s) not in verdicts]
stale = [k for k in verdicts if k not in {(u, s) for u, ss in produced.items() for s in ss}]

by = collections.Counter(verdicts[k][0] for k in adj)
print("\n=== skeleton adjudication ledger: %s ===" % ledger)
print("  units %d   skeletons %d" % (n_units, n_sk))
print("  pins read from expected-readings.tsv: %d" % n_pins)
print("  ADJUDICATED   %4d  (correct %d, available %d, invalid %d)"
      % (len(adj), by["correct"], by["available"], by["invalid"]))
print("  UNADJUDICATED %4d" % len(una))
print("  stale rows    %4d" % len(stale))
if malformed:
    print("  MALFORMED     %4d  (need 4 tab fields; verdict in %s)" % (len(malformed), sorted(VALID)))
    for i, t in malformed[:5]:
        print("      line %d: %s" % (i, t))

# a unit with more than one `correct` is an authoring error — the pin must be unique
multi = [u for u, ss in produced.items()
         if sum(1 for s in ss if verdicts.get((u, s), ("", ""))[0] == "correct") > 1]
for u in multi:
    print("  ERROR: more than one `correct` for «%s»" % u[:70])

print("\n  by unit (unadjudicated first):")
rows = sorted(produced.items(), key=lambda kv: (-sum(1 for s in kv[1] if (kv[0], s) not in verdicts), -len(kv[1])))
for u, ss in rows:
    miss = sum(1 for s in ss if (u, s) not in verdicts)
    mark = "  " if miss == 0 else "<<"
    print("   %s %2d/%2d adjudicated  %s" % (mark, len(ss) - miss, len(ss), u[:64]))

if stale:
    print("\n  STALE (verdict recorded, skeleton no longer produced) — delete deliberately:")
    for u, s in stale[:10]:
        print("    «%s»  %s" % (u[:52], s[:60]))

if worksheet and una:
    with io.open(worksheet, "w", encoding="utf-8") as fh:
        fh.write("# UNADJUDICATED skeletons — fill in column 3 (correct|available|invalid) and column 4\n")
        fh.write("# (the evidence). An `invalid` verdict MUST say why the reading is inadmissible under\n")
        fh.write("# EVERY sense assignment, and name the rule that should block it. Append to %s.\n" % ledger)
        for u, s in una:
            fh.write("%s\t%s\t\t\n" % (u, s))
    print("\n  worksheet written: %s (%d rows)" % (worksheet, len(una)))

if malformed or multi:
    sys.exit(2)
# FAIL CLOSED: an unadjudicated skeleton is a finding, not a pass.
sys.exit(1 if una else 0)
PY
