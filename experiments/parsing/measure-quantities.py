#!/usr/bin/env python3
"""Count measure phrases in a corpus and classify them by what governs them.

Produces the numbers D95 ("Quantities in the tokenizer and parser pipeline") cites:
the construction table, the total, and the per-preposition counts that size the
subcategorisation cost.

    ./measure-quantities.py <file.txt> [<file.txt> ...]
    ./measure-quantities.py --verbose <file.txt>      # dump every instance per bucket
    ./measure-quantities.py --shapes <file.txt>       # the shapes D95 slices 6-7 build

`--shapes` counts what surrounds a number rather than what governs it: a bound
before it (`less than`, `at least`, `<`, `~`), a range, a PP it modifies
(`9 days after`), an `of` after it, `every` before it, a `per` rate, `N-fold`.
A symbolic bound inside parentheses is counted apart, since those are almost all
P values in a statistics aside.

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


# A bare number, optionally a range, with no leading bound marker (SHAPES adds those).
N = r"\d+(?:[.,]\d+)?(?:\s*[-–]\s*\d+(?:[.,]\d+)?)?"
SHAPES = [
    ("bound in words", re.compile(
        r"\b(?:(?:less|more|fewer|greater|higher|lower)\s+than|at\s+(?:least|most)|up\s+to)\s+"
        r"(?:" + N + r"|one|two|three|half)\b", re.I)),
    ("bound in symbols, running text", re.compile(r"[<>≤≥]\s*[-−]?" + N)),
    ("approximation", re.compile(r"(?:~\s*|\b(?:approximately|around|roughly)\s+)" + N, re.I)),
    ("range with a unit or %", re.compile(
        r"\b\d+(?:\.\d+)?\s*[-–]\s*\d+(?:\.\d+)?\s*(?:%|°C|h\b|min\b|days?\b|weeks?\b|[µμu]?[gLlM]\b)")),
    ("measure phrase before a PP", re.compile(
        r"\b" + N + r"\s*(?:h|hours?|min|minutes?|days?|weeks?)\s+(?:after|before|post)\b", re.I)),
    ("measure phrase + of + noun", re.compile(
        r"\b" + N + r"\s*(?:%|[µμu]?[gLl](?:/m?[lL])?|[µμu]?L|ml|mL)\s+of\s+(?:the\s+)?[A-Za-z]")),
    ("every + number", re.compile(r"\bevery\s+" + N + r"\b", re.I)),
    ("per + noun (rate)", re.compile(
        r"\bper\s+(?:million|well|sample|cell|mouse|mice|condition|replicate|plate)\b", re.I)),
    ("N-fold", re.compile(r"\b\d+(?:\.\d+)?\s*[-–]?\s*fold\b", re.I)),
]


def inside_parentheses(text, pos):
    depth = 0
    for ch in text[:pos]:
        if ch == "(":
            depth += 1
        elif ch == ")" and depth:
            depth -= 1
    return depth > 0


def shapes(paths, verbose):
    text = ""
    for p in paths:
        with open(p, encoding="utf-8", errors="replace") as fh:
            text += fh.read() + "\n"
    text = re.sub(r"\s+", " ", text)
    text, _ = FIGREF.subn(" FIGREF ", text)
    print(f"{'shape':<40}{'n':>5}   first instances")
    for label, pattern in SHAPES:
        found = [(m.start(), m.group(0)) for m in pattern.finditer(text)]
        if label.startswith("bound in symbols"):
            inside = [f for f in found if inside_parentheses(text, f[0])]
            found = [f for f in found if not inside_parentheses(text, f[0])]
            rows = [(label, found), ("bound in symbols, in parentheses", inside)]
        else:
            rows = [(label, found)]
        for name, hits in rows:
            shown = "; ".join(h for _, h in hits[:4])
            print(f"{name:<40}{len(hits):>5}   {shown}")
            if verbose:
                for pos, hit in hits:
                    print(f"      …{text[max(0, pos - 60):pos + len(hit) + 40]}…")
    return 0


if __name__ == "__main__":
    flags = {a for a in sys.argv[1:] if a.startswith("--")}
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        sys.exit(2)
    if "--shapes" in flags:
        sys.exit(shapes(args, "--verbose" in flags))
    sys.exit(main(args, "--verbose" in flags))
