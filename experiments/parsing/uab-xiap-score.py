#!/usr/bin/env python3
"""Score a parse-coverage run over the certification-log corpus, per claim kind.

    python3 experiments/parsing/uab-xiap-score.py <run.log> <claims-map.tsv>

The harness prints one `[unit N, …] TAG` line per unit in `segment_sentences` order, so unit index
maps to claim by the cumulative `expected_units` of `claims-map.tsv`. The mapping is only sound if
both agree on how many units the page has, so the total is asserted against the run's own summary
line before anything is scored — a corpus the harness segmented differently is a different
experiment, not a reproduction.

A claim counts as COVERED when every one of its units is `ENCODED` or `AMBIG`: the parser produced
at least one reading. `AMBIG` is coverage, not failure — choosing among readings is the selection
stage, which this measurement does not run.

Spans marked `fragment` in the sidecar (headings, table cells — no sentence-final punctuation in the
source) are reported separately. They are not prose and a grammar that refuses them is not thereby
wrong, so folding them into one rate would flatter or damn the parser by a ratio of the document's
typography.
"""

import collections
import re
import sys

COVERED = {"ENCODED", "AMBIG"}


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    log, mapping = open(sys.argv[1], encoding="utf-8", errors="replace").read(), sys.argv[2]

    units = [(int(i), int(tok), float(sec), tag) for i, tok, sec, tag in
             re.findall(r"^\[unit\s+(\d+),\s+(\d+) tok,\s+([\d.]+)s\]\s+(\S+)", log, re.M)]
    if not units:
        sys.exit("no `[unit …]` lines in the log — the run did not reach the sweep")

    summary = re.search(r"=== WRN first page over FULL lexicon: (\d+) units", log)
    if summary and int(summary.group(1)) != len(units):
        sys.exit(f"summary says {summary.group(1)} units, log has {len(units)} lines")

    rows = []
    with open(mapping, encoding="utf-8") as f:
        next(f)
        for line in f:
            _, cid, kind, loc, words, form, exp = line.rstrip("\n").split("\t")
            rows.append((cid, kind, loc, int(words), form, int(exp)))

    expected = sum(r[5] for r in rows)
    if expected != len(units):
        sys.exit(f"corpus expects {expected} units, the harness produced {len(units)} — "
                 f"the segmentation disagrees, so unit→claim indices are not aligned")

    # Walk the units in order, assigning each to the claim whose span emitted it.
    per_claim, i = {}, 0
    for cid, kind, loc, words, form, exp in rows:
        per_claim[cid] = (kind, form, words, [u[3] for u in units[i:i + exp]],
                          sum(u[2] for u in units[i:i + exp]))
        i += exp

    def table(title, keep):
        sel = {c: v for c, v in per_claim.items() if keep(v)}
        if not sel:
            return
        print(f"\n{title} — {len(sel)} claims")
        print(f"  {'kind':<11} {'n':>4} {'covered':>8} {'rate':>7}   outcomes")
        agg = collections.Counter()
        for kind in sorted({v[0] for v in sel.values()},
                           key=lambda k: -sum(1 for v in sel.values() if v[0] == k)):
            group = [v for v in sel.values() if v[0] == kind]
            cov = sum(1 for v in group if all(t in COVERED for t in v[3]))
            tags = collections.Counter(t for v in group for t in v[3])
            agg.update(tags)
            detail = "  ".join(f"{t} {n}" for t, n in tags.most_common())
            print(f"  {kind:<11} {len(group):>4} {cov:>8} {cov / len(group):>6.1%}   {detail}")
        cov = sum(1 for v in sel.values() if all(t in COVERED for t in v[3]))
        print(f"  {'ALL':<11} {len(sel):>4} {cov:>8} {cov / len(sel):>6.1%}   "
              + "  ".join(f"{t} {n}" for t, n in agg.most_common()))

    print(f"{len(units)} units over {len(rows)} live claims, "
          f"{sum(u[2] for u in units) / 60:.1f} min of parsing")
    table("PROSE (sentence-final punctuation in the source)", lambda v: v[1] == "prose")
    table("FRAGMENTS (headings, table cells)", lambda v: v[1] == "fragment")

    # The lexical gap the survey expects to find (D97). `augmentation:` reports what the page's own
    # OOV closure grounded; `distinct OOV tokens` is what survived it and is the real gap.
    aug = re.search(r"^augmentation: (\d+) OOV grounded \+ injected, (\d+) named-entity "
                    r"individual\(s\), (\d+) residual OOV", log, re.M)
    if aug:
        print(f"\nOOV closure: {aug.group(1)} grounded + injected, {aug.group(2)} named entities, "
              f"{aug.group(3)} residual")
    oov = re.search(r"^distinct OOV tokens \((\d+)\): \[(.*)\]", log, re.M)
    if oov:
        toks = re.findall(r'"([^"]*)"', oov.group(2))
        print(f"distinct OOV tokens ({oov.group(1)}): {', '.join(toks[:40])}"
              + (" …" if len(toks) > 40 else ""))

    slow = sorted(per_claim.items(), key=lambda kv: -kv[1][4])[:10]
    print("\nslowest claims (id, kind, seconds, outcomes):")
    for cid, (kind, form, words, tags, sec) in slow:
        print(f"  {cid} {kind:<11} {sec:>7.1f}s  {' '.join(tags)}")


if __name__ == "__main__":
    main()
