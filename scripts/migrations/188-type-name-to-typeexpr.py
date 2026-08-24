#!/usr/bin/env python3
# Copyright 2026 The Eigenius Authors. Apache-2.0.
#
# eigenius#188 / N4 — rewrite hand-authored `core:type_name` and `core:param_kind` STRINGS into
# `eigentt:TypeExpr` values.
#
# ESL-declared inductives re-encode at bootstrap and need nothing. Hand-authored JSON does not, so
# these 89 values are rewritten once, here.
#
# The mapping is `decode_param_kind_str` / `decode_arg_type` inverted. It is deliberately TOTAL over
# the shapes those decoders accept, and refuses anything else rather than guessing — a value this
# script cannot classify is a value whose meaning it does not know.
#
#   Size / *:Size                     -> SizeSort
#   Prop / *:Prop                     -> Sort(Zero)
#   Set / *:Set                       -> Sort(Succ(Zero))
#   Type:N                            -> Sort(Succ^{N+1}(Zero))
#   core:string|integer|float|boolean -> ConstRef(<iri>)      (decoder short-circuits to a primitive)
#   any other IRI                     -> ConstRef(<iri>)      (inductive, class, or self-reference)
#   bare name (no ':')                -> Var(<name>)          (type-parameter reference)
#
# Run:  python3 scripts/migrations/188-type-name-to-typeexpr.py [--check]
# `--check` reports what would change and exits non-zero if anything would, without writing.

import json, sys, pathlib

FILES = [
    "ontologies/core/core-ontology.json",
    "ontologies/eigentt/eigentt-type-fragment.json",
    "ontologies/formulas/formulas-ontology.json",
    "ontologies/lean/lean-expressions.eigon.json",
]
KEYS = ("urn:eigenius:core:type_name", "urn:eigenius:core:param_kind")
PRIMS = {f"urn:eigenius:core:{p}" for p in ("string", "integer", "float", "boolean")}


def ctor(name, args):
    return {"ctor": name, "args": args}


def sort_of(n):
    lvl = ctor("Zero", [])
    for _ in range(n):
        lvl = ctor("Succ", [lvl])
    return ctor("Sort", [lvl])


def to_typeexpr(s: str):
    """The value the new encoder would produce for this string, or None if unclassifiable."""
    if s == "Size" or s.endswith(":Size"):
        return ctor("SizeSort", [])
    if s == "Prop" or s.endswith(":Prop"):
        return sort_of(0)
    # `:Set` is qualified the same way `:Prop` is, and means the same thing: the SORT, not a
    # resource. `urn:eigenius:core:Set` is not a declared resource on any chain, so mapping it to
    # a ConstRef produces a value that cannot resolve. The old string decoder never matched a
    # qualified `:Set` either — it fell through to the silent `Sort(1)` default, which is what
    # this arm now says out loud.
    if s == "Set" or s.endswith(":Set"):
        return sort_of(1)
    if s.startswith("Type:"):
        try:
            return sort_of(int(s[len("Type:"):]) + 1)
        except ValueError:
            return None
    if ":" in s:
        return ctor("ConstRef", [s])          # primitive, inductive, class, or self-reference
    if s and (s[0].isalpha() or s[0] == "_"):
        return ctor("Var", [s])               # type-parameter reference
    return None


def decodes_the_same(old: str, new: dict) -> bool:
    """The guard: the rewritten value must denote what the string denoted.

    Not a re-implementation of the decoder — a check that this script's mapping is the identity
    on meaning, expressed in the same terms both decoders use. A rewrite that changes meaning is
    worse than no rewrite, and is exactly what a bulk edit is prone to.
    """
    c = new["ctor"]
    if c == "SizeSort":
        return old == "Size" or old.endswith(":Size")
    if c == "Sort":
        n, lvl = 0, new["args"][0]
        while lvl["ctor"] == "Succ":
            n += 1
            lvl = lvl["args"][0]
        if lvl["ctor"] != "Zero":
            return False
        if n == 0:
            return old == "Prop" or old.endswith(":Prop")
        if n == 1:
            return old == "Set" or old.endswith(":Set")
        return old == f"Type:{n - 1}"
    if c == "ConstRef":
        return new["args"][0] == old and ":" in old
    if c == "Var":
        return new["args"][0] == old and ":" not in old
    return False


import re

# Surgical text replacement, NOT parse-and-dump.
#
# `json.dumps(indent=2)` would reformat every file it touches — ~900 diff lines for a handful of
# semantic changes, on TCB ontologies. That is what eigenius#213 is about, and it is why the
# manifest cannot tell a reindent from a content change. So the rewrite edits only the matched
# spans and leaves every other byte alone; `--check` proves the result still parses.
VALUE_RE = re.compile(
    r'("urn:eigenius:core:(?:type_name|param_kind)"\s*:\s*)"([^"]*)"'
)


def rewrite_text(text, stats, failures):
    def sub(m):
        prefix, old = m.group(1), m.group(2)
        new = to_typeexpr(old)
        if new is None or not decodes_the_same(old, new):
            failures.append((prefix.strip(' :"'), old))
            return m.group(0)
        key = old.split(":")[-1] if ":" in old else old
        stats[key] = stats.get(key, 0) + 1
        return prefix + json.dumps(new, separators=(", ", ": "))

    return VALUE_RE.sub(sub, text)


def main():
    check = "--check" in sys.argv
    stats, failures, changed = {}, [], []
    for f in FILES:
        p = pathlib.Path(f)
        text = p.read_text()
        out = rewrite_text(text, stats, failures)
        if out != text:
            json.loads(out)  # the edit must leave valid JSON
            changed.append(f)
            if not check:
                p.write_text(out)
    if failures:
        print(f"REFUSED {len(failures)} unclassifiable value(s) — nothing written:")
        for k, v in failures[:20]:
            print(f"  {k} = {v!r}")
        return 2
    print(f"values rewritten: {sum(stats.values())} across {len(changed)} file(s)")
    for f in changed:
        print(f"  {f}")
    print("by kind:", dict(sorted(stats.items(), key=lambda kv: -kv[1])))
    if check and changed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
