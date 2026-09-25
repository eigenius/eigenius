#!/usr/bin/env python3
"""Count measure phrases in a corpus and classify them by what governs them.

Produces the numbers D95 ("Quantities in the tokenizer and parser pipeline") cites:
the construction table, the total, and the per-preposition counts that size the
subcategorisation cost.

    ./measure-quantities.py <file.txt> [<file.txt> ...]
    ./measure-quantities.py --verbose <file.txt>      # dump every instance per bucket

For a PDF, extract first — `pdftotext -enc UTF-8 paper.pdf paper.txt`.

LIMITATIONS, all of which produced a wrong-but-plausible number at least once:

  * Figure references parse as measure phrases. `Fig. 1d` matches <number><unit>
    with d = day. They are substituted out before matching; eight of them reached
    a published count before that was added.
  * The "appositive" bucket is contaminated by design. The governor is the token
    before the measure phrase, so a comma separating two list items reads as
    apposition: `0.1% Tween20, 0.1% BSA` is two prenominally-modified NPs, not a
    label and a value. Run with --verbose and read the bucket before citing it.
  * `pdftotext` merges columns on a two-column page, leaving fragments such as
    `2 5 A` in "other". The typographically clean extraction is partial instead.
  * The taxonomy is this script's, not a standard one. It classifies by the
    preceding token, which approximates syntax and does not parse anything.

The honest use is comparison and magnitude, not precision: it says PP-governed
measure phrases are a minority and names which prepositions carry them.
"""
import re
import sys
import collections

NUM = r"[<>~±]?-?\d+(?:[.,]\d+)?(?:\s*[-–]\s*\d+(?:[.,]\d+)?)?"
UNIT = (r"(?:°\s?C|%|×\s?g|\d?g\b|[µμumnkMGpf]?"
        r"[gGlLmMsShHdWJVAK](?:/[a-zA-Z]+)?\b|min\b|h\b|days?\b|weeks?\b|bp\b|kb\b|"
        r"nt\b|[nµμm]?M\b|rpm\b|-?fold\b)")

FIGREF = re.compile(r"(?:Extended Data )?(?:Fig|Figs|Table|Supplementary)\.?\s*"
                    r"\d+[a-z]?(?:[-–,]\s*[a-z])?")

PREPS = {"to", "on", "in", "with", "from", "for", "at", "upon", "about", "against",
         "into", "of", "as", "by", "within", "over", "under", "between", "per",
         "after", "before", "during"}
VERBAL = {"containing", "purified", "incubated", "diluted", "supplemented"}


def classify(raw_governor):
    g = raw_governor.lower().strip(",.;:()[]")
    if g in PREPS:
        return "PP-governed", g
    if g == "every":
        return "distributive (every N unit)", None
    if g in {"n", "="}:
        return "sample size (n = N)", None
    if raw_governor.rstrip().endswith((",", ")")):
        return "appositive / list (CONTAMINATED - see docstring)", None
    if g in VERBAL:
        return "verbal argument", None
    return "other", None


def main(paths, verbose):
    text = ""
    for p in paths:
        with open(p, encoding="utf-8", errors="replace") as fh:
            text += fh.read() + "\n"
    text = re.sub(r"\s+", " ", text)
    text, n_figref = FIGREF.subn(" FIGREF ", text)

    pattern = re.compile(r"(\S+)\s+(" + NUM + r")\s*(" + UNIT + r")")
    kinds = collections.Counter()
    preps = collections.Counter()
    instances = collections.defaultdict(list)

    for m in pattern.finditer(text):
        if "FIGREF" in m.group(1):
            continue
        kind, prep = classify(m.group(1))
        kinds[kind] += 1
        if prep:
            preps[prep] += 1
        instances[kind].append(m.group(0).strip())

    total = sum(kinds.values())
    print(f"source words : {len(text.split()):,}   figure refs removed: {n_figref}")
    print(f"{'construction':<48}{'n':>5}{'share':>8}")
    for kind, count in kinds.most_common():
        print(f"{kind:<48}{count:>5}{count / total * 100:>7.0f}%")
    print(f"{'total':<48}{total:>5}")
    print()
    print("prepositions governing a measure phrase "
          f"({len(preps)} of {len(PREPS)} matched):")
    for prep, count in preps.most_common():
        print(f"   {prep:<10}{count}")

    if verbose:
        for kind in kinds:
            print(f"\n--- {kind} ---")
            for inst in instances[kind]:
                print("   ", inst)
    return 0


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--verbose"]
    if not args:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(args, "--verbose" in sys.argv))
