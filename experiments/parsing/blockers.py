#!/usr/bin/env python3
"""Emit the ONE-AWAY blocker sentences: the single failing sentence of each source unit that is
one sentence short of full coverage.

    python3 experiments/parsing/blockers.py <run.log> <cnl-map.tsv> <page.txt> [--all]

Only a gap that is the LAST failing sentence of its source unit converts a unit when fixed. Every
other gap is worth zero on its own, so a work order built on the raw gap count is priced wrong: the
CNL page has 499 grammar-gaps but only ~94 of them are worth a unit. This prints those, one per
line, ready for `EIGENIUS_PROBE_FILE`.

The run must be `--units-per-line`, so run-log unit N (0-based) is page line N+1 and cnl-map's
`out_first`/`out_last` (1-based, contiguous) address the same lines. Only `disposition == R` rows
are claim units; `PROV` rows are provenance records and are reported separately, never mixed into
the rate.

Output is UAB corpus text — write it outside the repo (see the `no UAB data into git` rule).
"""

import collections
import re
import sys

COVERED = {"ENCODED", "AMBIG", "OPEN"}
TAG = re.compile(r"^\[unit +(\d+),[^]]*\] ([A-Z-]+)")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    show_all = "--all" in sys.argv
    if len(args) != 3:
        sys.exit(__doc__)
    run_log, map_tsv, page_txt = args

    tags = {}
    with open(run_log, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            if m := TAG.match(line):
                tags[int(m.group(1))] = m.group(2)
    lines = open(page_txt, encoding="utf-8").read().splitlines()

    units, hist = [], collections.Counter()
    with open(map_tsv, encoding="utf-8") as fh:
        header = next(fh).rstrip("\n").split("\t")
        col = {n: i for i, n in enumerate(header)}
        for row in fh:
            f = row.rstrip("\n").split("\t")
            if f[col["disposition"]] != "R":
                continue
            lo, hi = int(f[col["out_first"]]), int(f[col["out_last"]])
            # cnl-map is 1-based over page lines; the run log is 0-based over the same lines.
            failing = [i for i in range(lo - 1, hi) if tags.get(i, "GRAMMAR-GAP") not in COVERED]
            hist[min(len(failing), 4)] += 1
            units.append((f[col["src_unit"]], failing))

    print(f"# claim units: {len(units)}", file=sys.stderr)
    for k in sorted(hist):
        label = f"{k}" if k < 4 else "4+"
        print(f"#   {label:>2} failing: {hist[k]}", file=sys.stderr)

    want = [u for u in units if len(u[1]) == 1] if not show_all else units
    print(f"# emitting {len(want)} blocker sentence(s)", file=sys.stderr)
    for _src, failing in want:
        for i in failing:
            if 0 <= i < len(lines):
                print(lines[i])


if __name__ == "__main__":
    main()
