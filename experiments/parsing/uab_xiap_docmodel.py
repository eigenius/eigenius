"""Shared reader for the XIAP document model (D101) — the containment walk, once.

Importable (underscores, not hyphens) by `uab-xiap-docmodel-check.py` and
`uab-xiap-docmodel-corpus.py`, which both need the same question answered: which units does a
region contain? In a tree that is not a field on the unit, it is what the walk from the region
reaches, so two copies of the walk would be two chances to disagree.

A "region" is what the certification register addresses with `§<name>`: a section, the h1 region
(the body's top-level paragraphs, which belong to no section), or a back-matter list.
"""

import collections
import json
import re

BLOCK_KEYS = ("sections", "paragraphs", "tables", "reflists", "refs")


def load(path):
    return json.load(open(path, encoding="utf-8"))


def slug(s, n=48):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")[:n]


def index(model):
    """id → node, over every block kind the tree contains."""
    ix = {}
    for key in BLOCK_KEYS:
        for n in model.get(key, []):
            ix[n["id"]] = n
    return ix


def units_under(node_id, ix, seen=None):
    """Every DiscourseUnit id the containment walk from `node_id` reaches, in document order."""
    seen = seen if seen is not None else set()
    if node_id in seen:
        return []
    seen.add(node_id)
    n = ix.get(node_id)
    if n is None:
        return []
    out = list(n.get("units", []))
    for child in n.get("children", []) + n.get("refs", []):
        out.extend(units_under(child, ix, seen))
    return out


def regions(model):
    """region key → unit ids. Keys: `__h1__`, a section slug, or a back-matter list's slug."""
    ix = index(model)
    sect = {s["id"] for s in model["sections"]}
    out = collections.OrderedDict()
    h1 = []
    for bid in model["body"]:
        if bid not in sect:
            h1.extend(units_under(bid, ix))
    out["__h1__"] = h1
    for s in model["sections"]:
        out[s["id"].replace("xdoc:sec_", "")] = units_under(s["id"], ix)
    for bid in model["back"]:
        n = ix.get(bid)
        if n:
            out[slug(n.get("title") or "back_matter")] = units_under(bid, ix)
    return out


def region_of(model):
    """unit id → region key. The inverse of `regions`, for per-unit reporting."""
    return {u: key for key, us in regions(model).items() for u in us}


def block_of(model):
    """unit id → the kind of block that contains it: `paragraph`, `table` or `ref`.

    This is the document model's own structural distinction, and it is what makes a per-kind
    coverage table mean something: a table ROW and a reference ANNOTATION fail for different
    reasons than a prose sentence does.
    """
    out = {}
    for key, label in (("paragraphs", "paragraph"), ("tables", "table"), ("refs", "ref")):
        for n in model.get(key, []):
            for u in n.get("units", []):
                out[u] = label
    return out
