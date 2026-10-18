#!/usr/bin/env python3
# Copyright 2026 The Eigenius Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""Emit a SLASH-MODE patch layer from an already-emitted lexicon ESL.

A slash mode (`lexicon:Mode`, D63 multimodal slashes) is denotation-transparent — it changes
only which combinatory rules may consume a slash, never what the entry means. So a mode
experiment adds and removes NO entries: it rewrites the `lexicon:cat` of entries that already
exist. That makes it a pure SHADOWING patch — same resource IRI, new category — and the base
snapshot never has to be rebuilt.

This is why that matters: a reseed re-imports UMLS, which is the ~30-minute part of it
(`experiments/parsing/probes/frame2223-whole-lexicon.esl` says so), and a mode change touches
NO UMLS entry — UMLS emits only `cat_n`, which has no slash at all, as does the WordNet↔UMLS
aligner. Three mode experiments were run as full reseeds on 2026-08-01 before this was noticed.

Unlike the frame-22/23 patch, this needs no second importer run: the mode token is the only
thing that differs, so the transformation is applied to the emitted text directly. Only blocks
whose `lexicon:cat` actually CHANGED are emitted, so the layer stays as small as the experiment.

Usage:
  scripts/make-mode-patch.py --in wn.esl --out patch.esl \\
      --match 'lexicon:cat_pp_arg' --from m_all --to m_app [--slash-only]

  --match       only rewrite a cat containing this substring (the experiment's scope)
  --from/--to   the mode ctor to rewrite (bare names, e.g. m_all -> m_app)
  --slash-only  rewrite ONLY the slash whose ARGUMENT matches `--match` (the innermost
                enclosing fwd/bwd), not every mode in the category. Without it, every
                `--from` in a matching cat is rewritten.
"""

import argparse
import re
import sys

BLOCK = re.compile(r"resource\s+[\w:]+\s*:\s*lexicon:LexicalEntry\s*\{.*?\n\}\n", re.S)
CAT_LINE = re.compile(r"(\s*lexicon:cat\s*=\s*type_expr\()(.*?)(\);)", re.S)
# Every emitted part must carry the namespace preamble: a layer with bare `resource` blocks and no
# `namespace` bindings does not parse, and the kernel's failure arrives as an opaque h2 stream reset
# (`Reset(StreamId(1), PROTOCOL_ERROR)`), NOT as an ESL diagnostic — so a missing preamble looks like
# a transport bug. Copied verbatim from the source file rather than hardcoded, so it tracks the
# importer.
NS_LINE = re.compile(r"^\s*namespace\s+\w+\s*=\s*\"[^\"]+\";\s*$", re.M)


def rewrite_slash_only(cat: str, match: str, frm: str, to: str) -> str:
    """Rewrite the mode of the slash whose ARGUMENT contains `match`.

    Scans for `lexicon:fwd(lexicon:<frm>, ` / `bwd(...)` and rewrites one only when `match`
    occurs inside that slash's own argument list at depth 1 — i.e. it is that slash's argument,
    not something nested deeper in its result. Rewriting by naive substring would also hit an
    OUTER slash that merely CONTAINS the marker in its result, which is the essive bug: its
    outer `fwd` takes an NP object and only its result mentions `cat_pp_arg`.
    """
    out = cat
    for slash in ("fwd", "bwd"):
        needle = f"lexicon:{slash}(lexicon:{frm}, "
        pos = 0
        while True:
            i = out.find(needle, pos)
            if i < 0:
                break
            # Walk this constructor's argument list, tracking depth; collect the LAST argument.
            j = i + len(needle)
            depth = 1
            arg_start = j
            last_arg = None
            while j < len(out) and depth > 0:
                c = out[j]
                if c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
                    if depth == 0:
                        last_arg = out[arg_start:j]
                elif c == "," and depth == 1:
                    arg_start = j + 1
                j += 1
            if last_arg is not None and match in last_arg:
                out = out[:i] + f"lexicon:{slash}(lexicon:{to}, " + out[i + len(needle) :]
                pos = i + len(f"lexicon:{slash}(lexicon:{to}, ")
            else:
                pos = i + len(needle)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--in", dest="inp", required=True)
    ap.add_argument("--out", dest="out", required=True)
    ap.add_argument("--match", required=True)
    ap.add_argument("--from", dest="frm", required=True)
    ap.add_argument("--to", dest="to", required=True)
    ap.add_argument("--slash-only", action="store_true")
    ap.add_argument("--note", default="")
    # A single Load of many resources trips the h2 CONTINUATION-frame guard
    # (`too_many_continuations` / ENHANCE_YOUR_CALM) long before the kernel's 128 MiB gRPC limit —
    # 13 022 resources in one call is enough. Partition, exactly as the importer's `--out-dir` does.
    ap.add_argument("--split", type=int, default=2000, help="max entries per emitted file (0 = one file)")
    a = ap.parse_args()

    src = open(a.inp).read()
    preamble = "\n".join(m.group(0).strip() for m in NS_LINE.finditer(src))
    if not preamble:
        print("ERROR: no `namespace` declarations in --in; the layer would not parse", file=sys.stderr)
        return 1
    changed, scanned = [], 0
    for m in BLOCK.finditer(src):
        block = m.group(0)
        scanned += 1
        cm = CAT_LINE.search(block)
        if not cm or a.match not in cm.group(2):
            continue
        old = cm.group(2)
        new = (
            rewrite_slash_only(old, a.match, a.frm, a.to)
            if a.slash_only
            else old.replace(f"lexicon:{a.frm}", f"lexicon:{a.to}")
        )
        if new == old:
            continue
        changed.append(block[: cm.start(2)] + new + block[cm.end(2) :])

    if not changed:
        print("ERROR: 0 entries changed — the experiment would be a silent no-op", file=sys.stderr)
        return 1

    def header(part: int, total: int, n: int) -> str:
        part_line = f"//   part {part + 1} of {total} ({n} entries)\n" if total > 1 else ""
        return (
            "// ════════════════════════════════════════════════════════════════════\n"
            f"// SLASH-MODE PATCH LAYER — {a.frm} -> {a.to} on slashes matching {a.match!r}\n"
            "//\n"
            "// Generated by scripts/make-mode-patch.py. SHADOWING ONLY: every resource here already\n"
            "// exists in the base with the same IRI; only `lexicon:cat` differs, and a slash mode is\n"
            "// denotation-transparent, so no entry is added or removed and `sem`/`sem_type` are\n"
            "// untouched. Measure against the base snapshot to isolate the mode change.\n"
            f"//\n//   entries scanned {scanned}\n//   entries rewritten {len(changed)}\n"
            + part_line
            + (f"//\n// {a.note}\n" if a.note else "")
            + "// ════════════════════════════════════════════════════════════════════\n\n"
            + preamble
            + "\n\n"
        )

    size = a.split if a.split > 0 else len(changed)
    parts = [changed[i : i + size] for i in range(0, len(changed), size)]
    written = []
    for k, part in enumerate(parts):
        path = a.out if len(parts) == 1 else re.sub(r"(\.esl)?$", f".{k:03d}.esl", a.out, count=1)
        with open(path, "w") as f:
            f.write(header(k, len(parts), len(part)))
            f.writelines(part)
        written.append(path)

    print(f"  scanned {scanned} entries, rewrote {len(changed)} -> {len(written)} file(s)")
    for w in written:
        print(f"    {w}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
