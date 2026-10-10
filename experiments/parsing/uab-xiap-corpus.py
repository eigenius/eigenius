#!/usr/bin/env python3
"""Build a parse-coverage corpus from a certification log's live claim spans.

Survey §4 step 4: "Run the `experiments/parsing` harness on the XIAP markdown's claim spans as a
corpus. The register gives 427 gold spans with kinds, so the measurement is per-kind."

The log is gitignored material, so this script takes its path and writes the page OUTSIDE the
tracked tree. Only the script and the aggregate numbers are committed.

A claim is LIVE unless one of its entries is `## E<n> · retired`. Its `text:` is a verbatim span of
the source document, so it is copied unaltered but for one thing: 147 of the 427 spans are headings
or table cells with no sentence-final punctuation, and `kernel/src/dcg/segment.rs` breaks on `.!?`
and nothing else — without a terminator each would run into the next claim. A `.` is appended to
those, and the sidecar records which, so the scorer never credits a fragment as prose.

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
        one_line = " ".join(text.split())
        terminated = bool(re.search(r"[.!?]['\")\]]?$", one_line))
        # A span's own sentence breaks become extra units; the scorer needs the count to hold a
        # claim covered only when ALL of its units are.
        breaks = len(re.findall(r"[.!?]['\")\]]?\s+[A-Z(\[]", one_line))
        page.append(one_line if terminated else one_line + ".")
        rows.append((len(page), cid, kind, loc, len(one_line.split()),
                     "prose" if terminated else "fragment", breaks + 1))

    with open(os.path.join(out_dir, "claims-page.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(page) + "\n")
    with open(os.path.join(out_dir, "claims-map.tsv"), "w", encoding="utf-8") as f:
        f.write("line\tclaim\tkind\tlocation\twords\tform\texpected_units\n")
        for r in rows:
            f.write("\t".join(str(x) for x in r) + "\n")

    frag = sum(1 for r in rows if r[5] == "fragment")
    print(f"{len(rows)} live claims → {len(rows)} lines, "
          f"{sum(r[6] for r in rows)} expected units "
          f"({frag} fragments terminated, {sum(r[6] for r in rows) - len(rows)} extra from "
          f"span-internal sentence breaks)")


if __name__ == "__main__":
    main()
