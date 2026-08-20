#!/usr/bin/env python3
"""Render a committed claim's proposition as a readable formula.

The demo's point is that a sentence BECOMES a formula over the classes the chain already holds.
Nothing in a stream of `Loaded N resource(s)` shows that, so this reads the D47-encoded
`canonical_proposition` back out and prints it with concept names substituted.

Usage:  narrate.py <eigon-json> <claim-suffix>
"""
import json
import sys


# The demo's vocabulary. A static table rather than live `eigenius inspect` calls: resolving a
# dozen IRIs through a 1 GB lexicon chain takes long enough to stall the narration, and a demo
# wants the same output every time. Anything absent falls back to its IRI local name.
GLOSS = {
    "urn:eigenius:umlscui:C0388246": "WRN",
    "urn:eigenius:umlscui:C0920283": "helicase",
    "urn:eigenius:wn:n14606137": "exonuclease",
    "urn:eigenius:umlscui:C0920269": "MSI",
    "urn:eigenius:wn:n13440063": "activity",
    "urn:eigenius:wn:n05890249": "model",
    "urn:eigenius:wn:n14239918": "cancer",
    "urn:eigenius:wn:v02203362_t": "had",
    "urn:eigenius:wn:v02627934_t": "required",
    "urn:eigenius:wn:v01188725_t": "required",
    "urn:eigenius:demo:onco-typed:HasActivity": "HasActivity",
    "urn:eigenius:demo:onco-typed:RequiresActivity": "RequiresActivity",
}


def gloss(iri: str) -> str:
    return GLOSS.get(iri, iri.rsplit(":", 1)[-1])


# Structural combinators the DCG uses; rendered as notation rather than function calls.
THE = "urn:eigenius:ontology:the"
KIND_OF = "urn:eigenius:ontology:kind_of"
COMPOUND_KIND = "urn:eigenius:ontology:compound_kind"
COMPOUND = "urn:eigenius:ontology:compound"
PREP_OF = "urn:eigenius:ontology:prep_of"
AND = "urn:eigenius:logic:And"
FALSE = "urn:eigenius:logic:False"


def spine(n):
    """Unfold an App spine into (head, [args])."""
    args = []
    while isinstance(n, dict) and n.get("ctor") == "App":
        args.insert(0, n["args"][1])
        n = n["args"][0]
    return n, args


def render(n) -> str:
    if not isinstance(n, dict):
        return str(n)
    c, a = n.get("ctor"), n.get("args", [])
    if c == "ConstRef":
        return gloss(a[0])
    if c == "Var":
        return a[0]
    if c == "LitString":
        return f'"{a[0]}"'
    if c == "Sort":
        return "Set" if a[0] == 1 else f"Sort({a[0]})"
    if c == "Fst":
        return render(a[0])          # the projection is noise for a reader
    if c == "Snd":
        return f"snd({render(a[0])})"
    if c == "Sig":
        # Parenthesised: a Sigma routinely appears as an ARGUMENT (a compound modifier),
        # and without brackets "Sx:cancer. MSI-kind(x)-kind(y)" reads as the wrong tree.
        return f"(Σ{a[0]}:{render(a[1])}. {render(a[2])})"
    if c == "Pi":
        if a[0] == "":
            return f"({render(a[1])} ⟹ {render(a[2])})"
        return f"∀{a[0]}:{render(a[1])}. {render(a[2])}"
    if c == "CtorApp":
        return f"{gloss(a[0])}.{a[1]}"
    if c == "UnitVal":
        return "_"
    if c == "App":
        head, args = spine(n)
        if isinstance(head, dict) and head.get("ctor") == "ConstRef":
            iri = head["args"][0]
            r = [render(x) for x in args]
            if iri == THE and len(r) == 1:
                inner = r[0]
                if inner.startswith("(") and inner.endswith(")"):
                    inner = inner[1:-1]
                return f"the({inner})"
            if iri == KIND_OF and len(r) == 1:
                return r[0]
            if iri == COMPOUND_KIND and len(r) == 2:
                return f"{r[1]}-kind({r[0]})"
            if iri == COMPOUND and len(r) == 2:
                return f"{r[1]}-compound({r[0]})"
            if iri == PREP_OF and len(r) == 2:
                return f"of({r[0]}, {r[1]})"
            if iri == AND and len(r) == 2:
                return f"{r[0]} ∧ {r[1]}"
            return f"{gloss(iri)}({', '.join(r)})"
        return f"{render(head)}({', '.join(render(x) for x in args)})"
    return f"{c}({', '.join(render(x) for x in a)})"


def main():
    path, suffix = sys.argv[1], sys.argv[2]
    doc = json.load(open(path))
    # `serialize_document` emits a bare object for a single resource, an array otherwise.
    for r in doc if isinstance(doc, list) else [doc]:
        if not r["@id"].endswith(suffix):
            continue
        p = r.get("urn:eigenius:reflection:canonical_proposition") or r.get(
            "urn:eigenius:reasoning:proposition"
        )
        if p:
            print("   ", render(p))
            return
    print("    (no proposition found for", suffix, ")")


if __name__ == "__main__":
    main()
