#!/usr/bin/env python3
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

"""How many shape rules does the expected-reading corpus need, and at what abstraction?

`crates/eigenius-reasoning/src/grade.rs::build_shape_rule` keys a Declared rule on the parsed
proposition with the argument classes abstracted out. Two sentences share a rule only if those
propositions coincide. This script measures how many distinct classes the corpus falls into as
structure below the matrix frame is progressively discarded — the amortisation curve for
issue #111.

The skeletons in `expected-readings.tsv` are SENSE-ERASED (`kernel/src/dcg/skeleton.rs`): every
open-class lexical constant is already `§`. The counts here are therefore what a PERFECT lexical
abstraction would achieve, which is why they bound what VerbNet / FrameNet / PropBank can do.

Fail closed: every skeleton is re-printed and compared against its input before any count is
taken. A parse that does not round-trip aborts the run — a partial corpus is not a measurement.

    python3 experiments/parsing/skeleton-abstraction.py
"""

import collections
import os
import re
import sys

CORPUS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "expected-readings.tsv")

# `->` before `.1`/`.2` before identifiers, so the longest match wins. `$anaphor$0` and `G#0`
# are the parser's own binder spellings; `§` is the erased sense.
TOK = re.compile(r"\s*(->|→|\.1|\.2|\$[A-Za-z0-9_$]*|[A-Za-z_][A-Za-z0-9_#]*|§|Σ|Π|λ|[(),.:])")


class Lexer:
    def __init__(self, s):
        self.toks, i = [], 0
        while i < len(s):
            m = TOK.match(s, i)
            if not m:
                if s[i].isspace():
                    i += 1
                    continue
                raise ValueError(f"lex failure at offset {i}: {s[i:i + 30]!r}")
            self.toks.append(m.group(1))
            i = m.end()
        self.i = 0

    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else None

    def eat(self, expected=None):
        tok = self.toks[self.i]
        if expected and tok != expected:
            raise ValueError(f"expected {expected!r}, got {tok!r} at token {self.i}")
        self.i += 1
        return tok


def parse_expr(p):
    """`->` is right-associative and binds loosest."""
    left = parse_app(p)
    while p.peek() in ("->", "→"):
        p.eat()
        left = ("Arrow", left, parse_app(p))
    return left


def parse_app(p):
    tok = p.peek()
    if tok in ("Σ", "Π"):
        p.eat()
        var = p.eat()
        p.eat(":")
        dom = parse_app(p)
        p.eat(".")
        return ("Sig" if tok == "Σ" else "Pi", var, dom, parse_expr(p))
    if tok == "λ":
        p.eat()
        var = p.eat()
        p.eat(".")
        return ("Lam", var, parse_expr(p))
    node = parse_atom(p)
    while True:
        tok = p.peek()
        if tok == "(":
            p.eat("(")
            args = []
            if p.peek() != ")":
                args.append(parse_expr(p))
                while p.peek() == ",":
                    p.eat(",")
                    args.append(parse_expr(p))
            p.eat(")")
            node = ("App", node, tuple(args))
        elif tok in (".1", ".2"):
            node = ("Proj", p.eat(), node)
        else:
            return node


def parse_atom(p):
    tok = p.eat()
    if tok == "(":
        e = parse_expr(p)
        p.eat(")")
        return e
    return ("Hole",) if tok == "§" else ("Const", tok)


def parse(s):
    p = Lexer(s)
    e = parse_expr(p)
    if p.i != len(p.toks):
        raise ValueError(f"trailing tokens: {p.toks[p.i:]}")
    return e


def show(e):
    k = e[0]
    if k == "Hole":
        return "§"
    if k == "Const":
        return e[1]
    if k == "App":
        return f"{show(e[1])}({', '.join(show(a) for a in e[2])})"
    if k == "Proj":
        return f"{show(e[2])}{e[1]}"
    if k == "Sig":
        return f"Σ{e[1]}:{show(e[2])}. {show(e[3])}"
    if k == "Pi":
        return f"Π{e[1]}:{show(e[2])}. {show(e[3])}"
    if k == "Lam":
        return f"λ{e[1]}. {show(e[2])}"
    if k == "Arrow":
        return f"{show(e[1])} -> {show(e[2])}"
    raise ValueError(k)


def truncate(e, d):
    """Structure to depth `d`; every subtree below it becomes `_`.

    Depth 1 keeps the matrix frame and holes out every argument — the "verb sense plus its
    argument structure" key issue #111 proposes.
    """
    if d <= 0:
        return "_"
    k = e[0]
    if k == "Hole":
        return "§"
    if k == "Const":
        return e[1]
    if k == "App":
        return f"{truncate(e[1], d)}({', '.join(truncate(a, d - 1) for a in e[2])})"
    if k == "Proj":
        return f"{truncate(e[2], d)}{e[1]}"
    if k == "Sig":
        return f"Σ:{truncate(e[2], d - 1)}. {truncate(e[3], d - 1)}"
    if k == "Pi":
        return f"Π:{truncate(e[2], d - 1)}. {truncate(e[3], d - 1)}"
    if k == "Lam":
        return f"λ. {truncate(e[2], d - 1)}"
    if k == "Arrow":
        return f"{truncate(e[1], d - 1)} -> {truncate(e[2], d - 1)}"
    raise ValueError(k)


def load():
    rows = []
    with open(CORPUS, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            cols = line.rstrip("\n").split("\t")
            if len(cols) >= 2:
                rows.append((cols[0], cols[1]))
    return rows


def main():
    rows = load()
    trees, failures = [], []
    for sentence, skel in rows:
        try:
            tree = parse(skel)
            printed = show(tree)
            if printed.replace(" ", "") != skel.replace(" ", "").replace("→", "->"):
                failures.append((sentence, skel, printed))
            else:
                trees.append((sentence, tree))
        except ValueError as e:
            failures.append((sentence, skel, str(e)))

    if failures:
        for sentence, skel, detail in failures:
            print(f"ROUND-TRIP FAILURE «{sentence}»\n  in : {skel}\n  out: {detail}", file=sys.stderr)
        sys.exit(f"\n{len(failures)} of {len(rows)} skeletons did not round-trip; refusing to count.")

    n = len(trees)
    print(f"corpus: {CORPUS}")
    print(f"{n} readings, all round-tripped\n")

    full = collections.Counter(show(t) for _, t in trees)
    print(f"distinct sense-erased skeletons: {len(full)}")
    for skel, count in full.most_common():
        if count > 1:
            print(f"  shared by {count}:")
            for sentence, t in trees:
                if show(t) == skel:
                    print(f"    «{sentence}»")

    print("\nrules needed, as argument structure is discarded:\n")
    print("  depth   distinct rules   sentences/rule")
    print("  " + "-" * 42)
    prev = None
    for d in range(1, 13):
        classes = len(collections.Counter(truncate(t, d) for _, t in trees))
        print(f"  {d:>5}   {classes:>14}   {n / classes:>14.2f}")
        if classes == prev:
            break
        prev = classes
    print(f"  {'full':>5}   {len(full):>14}   {n / len(full):>14.2f}")

    print("\n  depth 1 = matrix frame only, every argument holed (issue #111's proposed key).")
    print("  Senses are already erased, so these are the counts a PERFECT lexical")
    print("  abstraction would reach — the bound on VerbNet / FrameNet / PropBank.")


if __name__ == "__main__":
    main()
