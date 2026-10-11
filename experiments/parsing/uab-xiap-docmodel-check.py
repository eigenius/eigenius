#!/usr/bin/env python3
"""Join the certification register to the document model, on LOCATION.

    python3 experiments/parsing/uab-xiap-docmodel-check.py <document-model.json> <register-dir>

A register claim's `text:` is NOT a span of the document — measured: 120 of 427 appear verbatim in
any document version, and the split is by extraction pass (pass 3 is 28/28, passes 1-2 are ~23%),
because pass 3 added rule R12 and made the verbatim check mechanical. The earlier claims are the
certifier's canonical RESTATEMENTS: «would be installed by the adenine base editor» for the
document's «that your adenine base editor would install». So the join is on `location:`, never on
text — a claim and a unit are different artifacts about the same place.

The register's locator grammar, read off 475 uses:

    §<section> [, <sub-element>] [¶<ordinal> | ¶ <name>]

with three rules this resolves, each a property of the grammar rather than of the document:
a trailing sub-element is appended WITHOUT a comma (`§Finding 4 table`); sections are abbreviated
(`BLUF`, `setting`); and `Finding N` must match by number because the titles continue past it.

The model is a TREE (D101), so a region's units are not a field on the unit — they are what the
containment walk from that region reaches. §References is back matter, not a section, and the h1
region is the body's top-level paragraphs, which belong to no section at all.
"""

import collections
import json
import re
import sys
import glob
import os

SUB = r"(?:\s+(?:table|heading|row|caption|list))+$"
ALIAS = {"bluf": "bottom_line_up_front", "setting": "the_variant_and_its_setting",
         # The register's "preamble" and "title" both denote the h1 region: the title line itself,
         # and the prose between it and the first `##`. Those paragraphs sit directly in enc:body,
         # under no section, so the region is named rather than resolved to a section id.
         "title": "__h1__", "preamble": "__h1__"}


def slug(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")[:48]


def index(model):
    """id → node, over every block kind the tree contains."""
    ix = {}
    for key in ("sections", "paragraphs", "tables", "reflists", "refs"):
        for n in model.get(key, []):
            ix[n["id"]] = n
    return ix


def units_under(node_id, ix, seen=None):
    """Every DiscourseUnit the containment walk from `node_id` reaches."""
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
    """region key → unit ids. A key is a section slug, `__h1__`, or a back-matter list's slug."""
    ix = index(model)
    sect = {s["id"]: s for s in model["sections"]}
    out = collections.OrderedDict()
    # The h1 region: body entries that are not sections.
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


def resolve(loc, keys):
    head = re.split(r",| ¶", loc)[0].strip().lstrip("§").strip()
    head = re.sub(SUB, "", head, flags=re.I).strip()
    k = slug(head)
    if k in ALIAS:
        return ALIAS[k]
    for key in keys:
        if k and (k == key or key.startswith(k) or k.startswith(key[:18])):
            return key
    m = re.match(r"^finding_(\d+)", k)
    if m:
        for key in keys:
            if key.startswith("finding_%s_" % m.group(1)):
                return key
    return None


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    model = json.load(open(sys.argv[1], encoding="utf-8"))
    reg = regions(model)
    keys = [k for k in reg if k != "__h1__"]

    rows, skipped = [], []
    for f in sorted(glob.glob(os.path.join(sys.argv[2], "*.md"))):
        s = open(f, encoding="utf-8").read()
        if not s.startswith("---"):
            # Not a claim record (the directory's READMEs). Counted and named, not dropped
            # silently — a silent skip is how a missing claim would look identical to no claim.
            skipped.append(os.path.basename(f))
            continue
        if re.search(r"^## E\d+ · retired", s, re.M):
            continue
        fm = s.split("---", 2)[1]
        rows.append((re.search(r"^id: (\S+)", fm, re.M).group(1),
                     re.search(r"^location: (.*)$", fm, re.M).group(1).strip(),
                     re.search(r"^kind: (\S+)", fm, re.M).group(1)))

    per, bad = collections.Counter(), collections.Counter()
    for _, loc, _ in rows:
        key = resolve(loc, keys)
        if key:
            per[key] += 1
        else:
            bad[loc] += 1

    if skipped:
        print(f"skipped {len(skipped)} non-claim file(s): " + ", ".join(skipped))
    total_units = sum(len(v) for v in reg.values())
    print(f"live claims {len(rows)}   regions {len(reg)}   "
          f"units {len(model['units'])} ({total_units} reachable by containment)")
    print(f"locators resolving to a region: {sum(per.values())} of {len(rows)} "
          f"({sum(per.values()) / len(rows):.0%})")
    for loc, n in bad.most_common(8):
        print(f"   UNRESOLVED x{n}  «{loc}»")
    print("\nclaims per region:")
    for key, units in reg.items():
        c = per.get(key, 0)
        if c or units:
            print(f"  {c:>4} claims  {len(units):>4} units   {key}")


if __name__ == "__main__":
    main()
