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
         # and the prose between it and the first `##`. The converter files both under the h1
         # section, so they resolve there rather than to regions of their own.
         "title": "__h1__", "preamble": "__h1__"}


def slug(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")[:48]


def resolve(loc, section_ids):
    head = re.split(r",| ¶", loc)[0].strip().lstrip("§").strip()
    head = re.sub(SUB, "", head, flags=re.I).strip()
    k = slug(head)
    if k in ALIAS:
        return ALIAS[k]
    for sid in section_ids:
        if k and (k == sid or sid.startswith(k) or k.startswith(sid[:18])):
            return sid
    m = re.match(r"^finding_(\d+)", k)
    if m:
        for sid in section_ids:
            if sid.startswith("finding_%s_" % m.group(1)):
                return sid
    return None


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    model = json.load(open(sys.argv[1], encoding="utf-8"))
    ids = [s["id"] for s in model["sections"]]
    rows = []
    for f in sorted(glob.glob(os.path.join(sys.argv[2], "*.md"))):
        s = open(f, encoding="utf-8").read()
        if re.search(r"^## E\d+ · retired", s, re.M):
            continue
        fm = s.split("---", 2)[1]
        rows.append((re.search(r"^id: (\S+)", fm, re.M).group(1),
                     re.search(r"^location: (.*)$", fm, re.M).group(1).strip(),
                     re.search(r"^kind: (\S+)", fm, re.M).group(1)))
    h1 = ids[0] if ids else None
    per = collections.Counter()
    bad = collections.Counter()
    for cid, loc, kind in rows:
        sid = resolve(loc, ids)
        if sid == "__h1__" and h1:
            per[h1] += 1
        elif sid:
            per[sid] += 1
        else:
            bad[loc] += 1
    print(f"live claims {len(rows)}   sections {len(ids)}   units {len(model['units'])}")
    print(f"locators resolving to a section: {sum(per.values())} of {len(rows)} "
          f"({sum(per.values()) / len(rows):.0%})")
    for loc, n in bad.most_common(8):
        print(f"   UNRESOLVED x{n}  «{loc}»")
    print("\nclaims per section:")
    units = collections.Counter(u["section"] for u in model["units"])
    for sid in ["__h1__"] + ids:
        c, u = per.get(sid, 0), units.get(sid, 0)
        if c or u:
            print(f"  {c:>4} claims  {u:>4} units   {sid}")


if __name__ == "__main__":
    main()
