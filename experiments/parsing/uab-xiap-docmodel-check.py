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
region is the body's top-level paragraphs, which belong to no section at all. That walk lives in
`uab_xiap_docmodel.py`, shared with the corpus builder: two copies would be two chances to
disagree about which units a region holds.
"""

import collections
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import uab_xiap_docmodel as dm  # noqa: E402

SUB = r"(?:\s+(?:table|heading|row|caption|list))+$"
ALIAS = {"bluf": "bottom_line_up_front", "setting": "the_variant_and_its_setting",
         # The register's "preamble" and "title" both denote the h1 region: the title line itself,
         # and the prose between it and the first `##`. Those paragraphs sit directly in enc:body,
         # under no section, so the region is named rather than resolved to a section id.
         "title": "__h1__", "preamble": "__h1__"}


def resolve(loc, keys):
    head = re.split(r",| ¶", loc)[0].strip().lstrip("§").strip()
    head = re.sub(SUB, "", head, flags=re.I).strip()
    k = dm.slug(head)
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
    model = dm.load(sys.argv[1])
    reg = dm.regions(model)
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
