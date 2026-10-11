#!/usr/bin/env python3
"""Build a parse-coverage corpus from the XIAP document model (D101) — survey §4 step 2.

    python3 experiments/parsing/uab-xiap-docmodel-corpus.py <document-model.json> <out-dir>

writes `<out-dir>/docmodel-page.txt` and `<out-dir>/docmodel-map.tsv`, consumable unchanged by
`experiments/parsing/uab-xiap-score.py`.

WHY NOT THE STEP-4 CORPUS. Step 4 measured the register's 427 claim spans — the certifier's
canonical RESTATEMENTS of what the document says, one per claim. This measures the document's own
450 units. They are different corpora about the same report, so step 4's figures (missing-lexeme
334, 495 distinct OOV types, 131 grounded, 230 residual) do not transfer and are not a baseline
for these.

The document model also supplies a distinction the register could not: `kind` here is the
containing BLOCK — paragraph, table row, or reference annotation — because a table row and a
bibliography annotation fail for different reasons than a prose sentence does, and one coverage
rate over all three says nothing about any of them.

`form` keeps step 4's rule (sentence-final punctuation ⇒ prose, else fragment) so that one axis
stays comparable between the two measurements.

One line is one unit: the harness must run `--units-per-line`, which the scorer asserts.
"""

import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import uab_xiap_docmodel as dm  # noqa: E402


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    model_path, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    model = dm.load(model_path)

    region = dm.region_of(model)
    block = dm.block_of(model)
    reachable = set(region)

    page, rows, orphans = [], [], []
    for u in model["units"]:
        uid = u["id"]
        if uid not in reachable:
            # A unit the containment walk never reaches is a defect in the model, not a datum:
            # it would be measured without belonging anywhere. Reported, never silently dropped.
            orphans.append(uid)
            continue
        one_line = " ".join(u["prose"].split())
        terminated = bool(re.search(r"[.!?]['\")\]]?$", one_line))
        page.append(one_line)
        rows.append((len(page), uid, block.get(uid, "unplaced"), region[uid],
                     len(one_line.split()), "prose" if terminated else "fragment"))

    with open(os.path.join(out_dir, "docmodel-page.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(page) + "\n")
    with open(os.path.join(out_dir, "docmodel-map.tsv"), "w", encoding="utf-8") as f:
        f.write("line\tunit\tkind\tregion\twords\tform\n")
        for r in rows:
            f.write("\t".join(str(x) for x in r) + "\n")

    by_block, by_form = {}, {}
    for r in rows:
        by_block[r[2]] = by_block.get(r[2], 0) + 1
        by_form[r[5]] = by_form.get(r[5], 0) + 1
    print(f"{len(model['units'])} units in the model → {len(rows)} lines "
          f"= {len(rows)} units under --units-per-line")
    print("  by block: " + "  ".join(f"{k} {v}" for k, v in sorted(by_block.items())))
    print("  by form : " + "  ".join(f"{k} {v}" for k, v in sorted(by_form.items())))
    print(f"  regions : {len(dm.regions(model))}")
    if orphans:
        print(f"  ORPHANS (unreachable by containment, EXCLUDED): {len(orphans)} "
              + ", ".join(orphans[:8]))


if __name__ == "__main__":
    main()
