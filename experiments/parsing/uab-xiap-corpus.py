#!/usr/bin/env python3
"""Build a parse-coverage corpus from a certification log's live claim spans.

Survey §4 step 4: "Run the `experiments/parsing` harness on the XIAP markdown's claim spans as a
corpus. The register gives 427 gold spans with kinds, so the measurement is per-kind."

The log is gitignored material, so this script takes its path and writes the page OUTSIDE the
tracked tree. Only the script and the aggregate numbers are committed.

A claim is LIVE unless one of its entries is `## E<n> · retired`. Its `text:` is a verbatim span of
the source document and is copied unaltered: the page is measured with `--units-per-line`, so one
line is one unit and nothing is inferred from punctuation. Inferring it does not work here —
`segment_sentences` turns these 427 spans into 653 units, splitting 90 and merging 5 pairs, after
which no outcome can be attributed to the claim that produced it.

146 spans are headings or table cells with no sentence-final punctuation («Methods», «Helix α1:
Met375–Arg381»). They stay as they are and the sidecar marks them `fragment`, so the scorer reports
them apart from prose rather than crediting or damning the grammar for the document's typography.

    python3 experiments/parsing/uab-xiap-corpus.py <claims-dir> <out-dir>

writes `<out-dir>/claims-page.txt` and `<out-dir>/claims-map.tsv`.
"""

import json
import os
import re
import sys


def live_claims(claims_dir):
    """Every non-retired claim, as (id, kind, location, text)."""
    for name in sorted(os.listdir(claims_dir)):
        if not name.endswith(".md"):
            continue
        src = open(os.path.join(claims_dir, name), encoding="utf-8").read()
        if re.search(r"^## E\d+ · retired", src, re.M):
            continue
        front = src.split("---", 2)[1]

        def field(key):
            m = re.search(rf"^{key}: (.*)$", front, re.M)
            return m.group(1).strip() if m else ""

        text = field("text")
        if text.startswith('"') and text.endswith('"'):
            try:
                text = json.loads(text)
            except ValueError:
                text = text[1:-1]
        yield field("id"), field("kind"), field("location"), text


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    claims_dir, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)

    page, rows = [], []
    for cid, kind, loc, text in live_claims(claims_dir):
        # Newlines inside a span would split one unit into two, so they fold to spaces; nothing
        # else is touched.
        one_line = " ".join(text.split())
        terminated = bool(re.search(r"[.!?]['\")\]]?$", one_line))
        page.append(one_line)
        rows.append((len(page), cid, kind, loc, len(one_line.split()),
                     "prose" if terminated else "fragment"))

    with open(os.path.join(out_dir, "claims-page.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(page) + "\n")
    with open(os.path.join(out_dir, "claims-map.tsv"), "w", encoding="utf-8") as f:
        f.write("line\tclaim\tkind\tlocation\twords\tform\n")
        for r in rows:
            f.write("\t".join(str(x) for x in r) + "\n")

    frag = sum(1 for r in rows if r[5] == "fragment")
    print(f"{len(rows)} live claims → {len(rows)} lines = {len(rows)} units under "
          f"--units-per-line ({frag} fragments, {len(rows) - frag} prose)")


if __name__ == "__main__":
    main()
