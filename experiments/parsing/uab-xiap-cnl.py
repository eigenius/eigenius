#!/usr/bin/env python3
"""Step 4 increment 1 — the CLAUSE-SPLICE split, applied to the whole page.

    python3 experiments/parsing/uab-xiap-cnl.py <page.txt> <out.txt>

The principle a CNL rewrite follows here: **remove exactly what the grammar cannot represent, and
preserve what it can.** That is why this does NOT split at `because` / `although` / `while` /
`though` / `whilst` — D62 2d Fork A made those representable, with their presuppositions exported
as proof obligations, so splitting them away would discard the relation the grammar now carries.

It splits at a CLAUSE-POSITION `:` or `;` only, because the lexer DELETES both (it emits `,` as a
non-word token but drops these), handing the grammar two independent clauses spliced with no
connective. Measured on the 68 gap units that carry one: splitting gave 145 pieces of which 11
parsed, where none of the 68 had.

A clause-position mark is one with a word character before and whitespace after, which excludes
`c.1141C>T`, `2025;16`, `chrX:123` and `https://`.
"""
import re
import sys

BOUND = re.compile(r"(?<=\w)[:;](?=\s)")


def split_unit(line):
    parts = [p.strip() for p in BOUND.split(line) if p.strip()]
    # A trailing fragment with no letter is not a clause — keep it attached rather than emit a
    # unit the parser can only refuse.
    out = []
    for p in parts:
        if out and not re.search(r"[A-Za-z]", p):
            out[-1] = out[-1] + " " + p
        else:
            out.append(p)
    return out or [line]


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src = [l.rstrip("\n") for l in open(sys.argv[1], encoding="utf-8") if l.strip()]
    out = []
    split_units = 0
    for line in src:
        pieces = split_unit(line)
        if len(pieces) > 1:
            split_units += 1
        out.extend(pieces)
    open(sys.argv[2], "w", encoding="utf-8").write("\n".join(out) + "\n")
    print(f"{len(src)} units -> {len(out)} units ({split_units} split)")


if __name__ == "__main__":
    main()
