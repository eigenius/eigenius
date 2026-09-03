#!/usr/bin/env python3
"""Fail if a relative markdown link under docs/ points at something that does not exist.

Ran as a one-off sweep on 2026-08-31, which found 137 broken references: module splits
(`merge.rs` -> `merge/`), renumbered guide chapters, renamed design docs, two notes that were
never written, and the WASM tree deleted 2026-07-08.

ALLOWED holds the four documents that keep dead links deliberately. Each carries a REMOVED
header saying its body is preserved unedited as a historical record, and each now also says the
file links no longer resolve. Editing those bodies would contradict the header, so they are
excluded here rather than repaired.
"""
import os, re, sys, glob

ALLOWED = {
    "docs/design/d12b-orchestrator-wasm-plan.md",
    "docs/design/d12-wasm-extensibility.md",
    "docs/guides/platform/09-wasm-components.md",
    "docs/guides/platform/10-wasm-institutions.md",
}

def main() -> int:
    broken = []
    for f in sorted(glob.glob("docs/**/*.md", recursive=True)):
        if f in ALLOWED:
            continue
        d = os.path.dirname(f)
        text = open(f, encoding="utf-8", errors="replace").read()
        for m in re.finditer(r"\]\((\.\.?/[^)]*)\)", text):
            link = m.group(1).split("#")[0]
            if not link:
                continue
            if not os.path.exists(os.path.normpath(os.path.join(d, link))):
                broken.append((f, m.group(1)))
    for f, link in broken:
        print(f"BROKEN  {f} -> {link}")
    print(f"\n{len(broken)} broken relative link(s) in docs/ "
          f"({len(ALLOWED)} historical-record files excluded)")
    return 1 if broken else 0

if __name__ == "__main__":
    sys.exit(main())
