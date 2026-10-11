#!/usr/bin/env python3
"""Convert the XIAP report into the D96 document model: sections, units, kinds, locators.

    python3 experiments/parsing/uab-xiap-document-model.py <report.md> <register-dir> <out-dir>

The report is gitignored material, so this takes its path and writes OUTSIDE the tracked tree.
Only this script and the aggregate counts are committed (`.gitignore:117`).

D96 §"Alignment": **assignment follows the element, not a heuristic.** Markdown states its own
structure — `#` is a heading, `|` a table row, a blank line ends a paragraph — so a unit's kind is
read off the syntax and never guessed. A first attempt at this classified by regex and counted 77
"list items" that were really bold-lead PARAGRAPHS (`**Lead.** Then prose…`), which is the failure
mode the rule exists to prevent; the register independently calls those locations `¶`.

D96 §"Locators are fragment identifiers": each unit carries a WICG Text Fragment
(`#:~:text=prefix-,textStart,textEnd,-suffix`), unique by construction, in place of a byte offset —
the encoder's `doc.find(text)` maps a repeated sentence to its first occurrence and yields 0 when
absent. `enc:span_start`/`span_end` are DERIVED from the locator here, never authored.

The locators go in a sidecar rather than the ESL because `enc:DiscourseUnit` has no locator property
and `encoding` is bootstrapped, so adding one costs a reseed. The sidecar is what validates against
the register meanwhile.
"""

import hashlib
import json
import os
import re
import sys

# ── block structure, read off markdown's own syntax ───────────────────────────────────────────────

HEADING = re.compile(r"^(#{1,6})\s+(.*)$")
TABLE_ROW = re.compile(r"^\s*\|")
TABLE_RULE = re.compile(r"^\s*\|[\s:|-]+\|?\s*$")


def blocks(text):
    """The report's blocks, in document order, as (kind, lines, first_line_no).

    `heading` / `table` / `para`. A blank line ends a paragraph; a run of `|` lines is one table;
    a `#` line is its own block. Nothing else is inferred.
    """
    out, buf, start = [], [], None
    lines = text.split("\n")

    def flush(kind):
        nonlocal buf, start
        if buf:
            out.append((kind, buf, start))
        buf, start = [], None

    i = 0
    while i < len(lines):
        line = lines[i]
        if HEADING.match(line):
            flush("para")
            out.append(("heading", [line], i + 1))
        elif TABLE_ROW.match(line):
            flush("para")
            t, s = [], i + 1
            while i < len(lines) and TABLE_ROW.match(lines[i]):
                t.append(lines[i])
                i += 1
            out.append(("table", t, s))
            continue
        elif not line.strip():
            flush("para")
        else:
            if start is None:
                start = i + 1
            buf.append(line)
        i += 1
    flush("para")
    return out


# ── units within a block ──────────────────────────────────────────────────────────────────────────

SENT_END = re.compile(r"(?<=[.!?])\s+(?=[A-Z(\[«“])")


def sentences(para):
    """Split a prose paragraph into units. Conservative: only on sentence punctuation followed by an
    uppercase start, which leaves `p.Ile380Thr` and `et al.` intact. The parser's own
    `segment_sentences` is not reused here because it is tuned for running prose over a whole
    document, and this already has the paragraph boundaries that make its inference unnecessary."""
    one = " ".join(l.strip() for l in para).strip()
    return [s.strip() for s in SENT_END.split(one) if s.strip()]


def normalized(doc):
    """The document's TEXT CONTENT in document order, whitespace collapsed.

    Text Fragments match rendered text content, not source bytes, so this is the base a locator
    resolves against — and it is the base D96 calls "the natural one". Matching raw source instead
    leaves every multi-line paragraph unresolvable, because a unit's text joins its lines with
    spaces; that cost 18 of 422 units on the first run."""
    return re.sub(r"\s+", " ", doc)


def text_fragment(doc, needle, ctx=24):
    """A WICG Text Fragment for `needle`, with prefix/suffix only where the text repeats.

    Returns the fragment string and the (start, end) span it resolves to, so the span is DERIVED
    from the locator rather than authored beside it. `None` if the text is not found — which the
    caller must treat as an error, not as offset 0 (the defect D96 records)."""
    first = doc.find(needle)
    if first < 0:
        return None, None
    enc = lambda s: re.sub(r"[-,&]", lambda m: "%%%02X" % ord(m.group()), s.strip())
    if doc.find(needle, first + 1) < 0:
        return f"#:~:text={enc(needle)}", (first, first + len(needle))
    pre = doc[max(0, first - ctx):first]
    suf = doc[first + len(needle):first + len(needle) + ctx]
    frag = f"#:~:text={enc(pre)}-,{enc(needle)},-{enc(suf)}"
    return frag, (first, first + len(needle))


def slug(s):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")[:48]


# ── ESL emission ──────────────────────────────────────────────────────────────────────────────────

ESL_HEADER = """// The XIAP report as D96's document model — sections, units, kinds.
//
// GENERATED by experiments/parsing/uab-xiap-document-model.py. Derived from gitignored material
// (`experiments/uab`), so this file is gitignored too (.gitignore:117): an encoding of the logs is
// the logs in another notation.
//
// Unit kinds follow the ELEMENT, never a heuristic (D96 Alignment): markdown states its own
// structure — `#` a heading, `|` a table row, a blank line a paragraph boundary.
//
// A heading is NOT a unit. It is the section's title, carried as `enc:section` on the units beneath
// it, which is JATS's shape: `<title>` is a property of `<sec>`. `enc:section` is a free string
// today; D96 records that it should carry section identity and `sec-type`, so that "the methods" is
// a query rather than a string match.
//
// `enc:span_start`/`enc:span_end` are DERIVED from each unit's Text Fragment locator (D96
// "Locators are fragment identifiers"), resolved against the document's normalized text content —
// never authored, and never `doc.find` of a bare string, which maps a repeated sentence to its
// first occurrence. The locators live in document-model.json: `enc:DiscourseUnit` has no locator
// property and `encoding` is bootstrapped, so adding one costs a reseed.

// `enc:source_document` is constrained `class_types reference:Reference` and described as "the
// published work this unit is drawn from — the bibliographic provenance root". This report is an
// UNPUBLISHED clinical assessment, so the class is a loose fit: it is used because the constraint
// requires it and every reference field is optional, not because the report is a published work.
// D96 plans to extend `reference:Reference` with the manifestation read; the same seam.

namespace core      = "urn:eigenius:core";
namespace reference = "urn:eigenius:reference";
namespace enc  = "urn:eigenius:encoding";
namespace xdoc = "urn:eigenius:uab:xiap-report";
"""


def esl_escape(s):
    return s.replace("\\", "\\\\").replace('"', '\\"')


def emit_esl(report, digest, sections, units, out_path):
    parts = [ESL_HEADER, ""]
    desc = ("%s — sha256:%s, media type text/markdown. %d sections, %d discourse units. Spans are "
            "offsets into the NORMALIZED text content (whitespace collapsed), the base every "
            "locator in document-model.json resolves against."
            % (esl_escape(os.path.basename(report)), digest, len(sections), len(units)))
    parts.append('resource xdoc:report : reference:Reference {\n'
                 '    reference:title  = "XIAP p.Ile380Thr bystander edit risk assessment";\n'
                 '    core:description = "%s";\n}\n' % desc)
    for i, u in enumerate(units):
        a, b = u["span"] or (0, 0)
        parts.append(
            'resource xdoc:u%04d : enc:DiscourseUnit {\n'
            '    enc:prose           = "%s";\n'
            '    enc:unit_kind       = enc:%s;\n'
            '    enc:source_document = xdoc:report;\n'
            '    enc:section         = "%s";\n'
            '    enc:span_start      = %d;\n'
            '    enc:span_end        = %d;\n}\n'
            % (i, esl_escape(u["text"]), u["kind"], esl_escape(u["section"]), a, b))
    open(out_path, "w", encoding="utf-8").write("\n".join(parts))



def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    report, register_dir, out_dir = sys.argv[1], sys.argv[2], sys.argv[3]
    os.makedirs(out_dir, exist_ok=True)
    doc = open(report, encoding="utf-8").read()
    digest = hashlib.sha256(doc.encode("utf-8")).hexdigest()
    base = normalized(doc)

    section, units, sections = None, [], []
    for kind, lines, lineno in blocks(doc):
        if kind == "heading":
            m = HEADING.match(lines[0])
            depth, title = len(m.group(1)), m.group(2).strip()
            section = {"depth": depth, "title": title, "id": slug(title), "line": lineno}
            sections.append(section)
            continue
        sec = section["id"] if section else "preamble"
        in_refs = bool(section and section["id"].startswith("references"))
        if kind == "table":
            body = [l for l in lines if not TABLE_RULE.match(l)]
            for n, row in enumerate(body):
                units.append({"kind": "kind_table", "section": sec, "text": row.strip(),
                              "block_line": lineno, "row": n})
        elif in_refs:
            # A reference list is one entry PER LINE, not per sentence: an entry does not end in
            # sentence punctuation followed by a capital, so the sentence splitter leaves a whole
            # block merged. Measured: 24 units against the register's 83 claims in §References
            # before this, which is how the defect surfaced.
            for n, line in enumerate(l for l in lines if l.strip()):
                units.append({"kind": "kind_citation", "section": sec,
                              "text": " ".join(line.split()), "block_line": lineno, "entry": n})
        else:
            for n, s in enumerate(sentences(lines)):
                units.append({"kind": "kind_prose", "section": sec, "text": s,
                              "block_line": lineno, "sent": n})

    # Locators, and spans derived from them.
    unresolved = 0
    for u in units:
        frag, span = text_fragment(base, u["text"])
        if frag is None:
            unresolved += 1
            u["locator"], u["span"] = None, None
        else:
            u["locator"], u["span"] = frag, span

    json.dump({"document": os.path.basename(report), "sha256": digest,
               "media_type": "text/markdown", "span_base": "normalized text content, whitespace collapsed",
               "sections": sections, "units": units},
              open(os.path.join(out_dir, "document-model.json"), "w", encoding="utf-8"),
              ensure_ascii=False, indent=1)

    emit_esl(report, digest, sections, units,
             os.path.join(out_dir, "xiap-document-model.esl"))

    by_kind = {}
    for u in units:
        by_kind[u["kind"]] = by_kind.get(u["kind"], 0) + 1
    print(f"document   {os.path.basename(report)}")
    print(f"sha256     {digest[:16]}")
    print(f"sections   {len(sections)}")
    print(f"units      {len(units)}   " + "  ".join(f"{k} {v}" for k, v in sorted(by_kind.items())))
    print(f"locators   {len(units) - unresolved} resolved, {unresolved} UNRESOLVED")


if __name__ == "__main__":
    main()
