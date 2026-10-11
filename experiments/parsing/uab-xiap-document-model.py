#!/usr/bin/env python3
"""Convert the XIAP report into D101's document model — a JATS tree, not a flat list of units.

    python3 experiments/parsing/uab-xiap-document-model.py <report.md> <out-dir>

The report is gitignored material, so this takes its path and writes OUTSIDE the tracked tree.
Only this script and the aggregate counts are committed (`.gitignore:117`).

OUTPUT (D101 "What the output is"): one `enc:Document` holding an ordered `enc:body` of
`enc:Section`s and an `enc:back` holding the bibliography. Sections nest by heading depth and carry
`enc:title` + `enc:section_kind`; their `enc:children` are `enc:Paragraph`s and `enc:Table`s, whose
`enc:units` are the `enc:DiscourseUnit`s. The h1 is the document's title, not a section. A first
version of this script emitted 467 units and no tree at all, which compiled and validated clean
because the vocabulary it needed did not exist yet.

THE RULE (D96 "Alignment"): **assignment follows the element, never a heuristic.** Markdown states
its own structure — `#` a heading, `|` a table row, a blank line a paragraph boundary, `- ` a list
item, a wholly-bold line a sub-heading. An earlier attempt classified by regex and counted 77 "list
items" that were bold-LEAD paragraphs (`**Lead.** Then prose…`); the register independently calls
those locations `¶`. The wholly-bold test below is what separates the two, and §References contains
one of each.

NO REFERENCE SPANS. Markdown has no `<xref>`, so a citation there is ordinary prose and recognising
one would be a derivation inside an otherwise observed transcription (D101). None is emitted; the
count of what a later derivation faces is printed instead.
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
BULLET = re.compile(r"^\s*-\s+(.*)$")
ALL_BOLD = re.compile(r"^\*\*(.+)\*\*$")          # a sub-heading
BOLD_LEAD = re.compile(r"^\*\*(.+?)\*\*[^*]")      # a paragraph that OPENS bold — not a heading


def blocks(text):
    """The source's blocks in document order, as (kind, lines, first_line_no).

    `frontmatter` / `comment` / `heading` / `table` / `para`. A blank line ends a paragraph; a run
    of `|` lines is one table; a `#` line is its own block. Nothing else is inferred here — a
    `para` block's INTERNAL shape (bullets, wholly-bold) is read by the caller that knows where it
    sits.

    Front matter and comments are blocks of their OWN KIND, not prose that a later filter has to
    recognise by content. Both are elements of the source format: a `---` fence at the very start
    of the file is YAML front matter, and `%%` fences are Obsidian comments. This report has 21
    lines of the first and a 3-line comment, which an earlier version emitted as four prose
    paragraphs of units — feeding YAML and a private authoring note to the parser.
    """
    out, buf, start = [], [], None
    lines = text.split("\n")

    # YAML front matter: only at the very start, only `---`-fenced.
    i0 = 0
    if lines and lines[0].strip() == "---":
        for j in range(1, len(lines)):
            if lines[j].strip() == "---":
                out.append(("frontmatter", lines[1:j], 1))
                i0 = j + 1
                break

    def flush():
        nonlocal buf, start
        if buf:
            out.append(("para", buf, start))
        buf, start = [], None

    i = i0
    while i < len(lines):
        line = lines[i]
        if line.strip() == "%%":
            flush()
            j = i + 1
            while j < len(lines) and lines[j].strip() != "%%":
                j += 1
            out.append(("comment", lines[i + 1:j], i + 1))
            i = j + 1
            continue
        if HEADING.match(line):
            flush()
            out.append(("heading", [line], i + 1))
        elif TABLE_ROW.match(line):
            flush()
            t, s = [], i + 1
            while i < len(lines) and TABLE_ROW.match(lines[i]):
                t.append(lines[i])
                i += 1
            out.append(("table", t, s))
            continue
        elif not line.strip():
            flush()
        else:
            if start is None:
                start = i + 1
            buf.append(line)
        i += 1
    flush()
    return out


SENT_END = re.compile(r"(?<=[.!?])\s+(?=[A-Z(\[«“])")


def sentences(lines):
    """Split prose into units: only on sentence punctuation followed by an uppercase start, which
    leaves `p.Ile380Thr` and `et al.` intact."""
    one = " ".join(l.strip() for l in lines).strip()
    return [s.strip() for s in SENT_END.split(one) if s.strip()]


# ── locators (D96 "Locators are fragment identifiers") ────────────────────────────────────────────


def normalized(doc):
    """The document's TEXT CONTENT in document order, whitespace collapsed. Text Fragments match
    rendered text content, not source bytes, so this is the base a locator resolves against —
    matching raw source leaves every multi-line paragraph unresolvable (18 of 422 on a first run)."""
    return re.sub(r"\s+", " ", doc)


def text_fragment(doc, needle, ctx=24):
    """A WICG Text Fragment for `needle`, with prefix/suffix only where the text repeats. Returns
    (fragment, (start, end)) so the span is DERIVED from the locator, never authored beside it.
    (None, None) if absent — which the caller treats as an error, not as offset 0."""
    first = doc.find(needle)
    if first < 0:
        return None, None
    enc = lambda s: re.sub(r"[-,&]", lambda m: "%%%02X" % ord(m.group()), s.strip())
    if doc.find(needle, first + 1) < 0:
        return f"#:~:text={enc(needle)}", (first, first + len(needle))
    pre = doc[max(0, first - ctx):first]
    suf = doc[first + len(needle):first + len(needle) + ctx]
    return (f"#:~:text={enc(pre)}-,{enc(needle)},-{enc(suf)}", (first, first + len(needle)))


# ── section kinds (D101 "Section types: JATS's values, in an open class") ─────────────────────────
#
# JATS's values, by exact title or by the one documented prefix. An unrecognised heading gets NO
# kind — `enc:section_kind` is `recommends`, so the section keeps its identity and title and the
# gap is counted and printed, rather than guessed at.

SECTION_KINDS = {
    "bottom line up front": "sec_conclusions",   # a BLUF is the conclusion, placed first
    "the variant and its setting": "sec_intro",
    "methods": "sec_methods",
}
FINDING = re.compile(r"^finding\s+\d+\b")
BACK_MATTER = {"references"}


def section_kind(title):
    key = title.strip().lower()
    if key in SECTION_KINDS:
        return SECTION_KINDS[key]
    if FINDING.match(key):
        return "sec_results"
    return None


def slug(s, n=48):
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")[:n]


# ── bibliography entries ──────────────────────────────────────────────────────────────────────────
#
# Parsed by MARKUP: the title is what is in italics, the PMID is what `**PMID n**` holds. Everything
# before the italics is the creator list, everything between the italics and the PMID is the
# container + year, and what follows is the author's annotation. All 63 of this report's entries
# have an italic title and a PMID; 29 carry annotation prose.

ENTRY = re.compile(r"^(?P<creators>.*?)\*(?P<title>[^*]+)\*(?P<tail>.*)$")
PMID = re.compile(r"\*\*PMID\s+(?P<pmid>\d+)\.?\*\*")
YEAR = re.compile(r"\b(19|20)\d{2}\b")


def parse_entry(text):
    """One bibliography entry → its bibliographic fields plus the annotation prose that follows."""
    m = ENTRY.match(text)
    if not m:
        return None
    creators = m.group("creators").strip().rstrip(".").strip()
    tail = m.group("tail")
    pm = PMID.search(tail)
    pmid = pm.group("pmid") if pm else None
    head = tail[: pm.start()] if pm else tail
    annotation = tail[pm.end():] if pm else ""
    annotation = re.sub(r"^\s*(PMC\d+\.)?\s*", "", annotation).strip()
    y = YEAR.search(head)
    container = head[: y.start()].strip().rstrip(".,;").strip() if y else head.strip()
    first = creators.split(",")[0].split()[0] if creators else "anon"
    return {
        "creators": creators,
        "title": m.group("title").strip().rstrip("."),
        "container_title": container,
        "issued_year": int(y.group()) if y else None,
        "pmid": pmid,
        "annotation": annotation,
        "key": f"{slug(first, 24)}_{y.group() if y else 'nd'}",
    }


# ── the tree ──────────────────────────────────────────────────────────────────────────────────────


class Builder:
    """Accumulates the resource tree. IDs are assigned in document order so the emitted ESL reads
    top-to-bottom like the source."""

    def __init__(self):
        self.units, self.paras, self.tables = [], [], []
        self.sections, self.reflists, self.refs = [], [], []
        self.works, self.seen_keys = [], {}
        self.unkinded, self.unresolved, self.bad_entries = [], 0, []
        self.excluded, self.frontmatter = [], []

    def unit(self, text, kind):
        i = len(self.units)
        self.units.append({"id": f"xdoc:u{i:04d}", "prose": text, "kind": kind})
        return self.units[-1]["id"]

    def paragraph(self, lines, kind="kind_prose"):
        ids = [self.unit(s, kind) for s in sentences(lines)]
        if not ids:
            return None
        i = len(self.paras)
        self.paras.append({"id": f"xdoc:p{i:04d}", "units": ids})
        return self.paras[-1]["id"]

    def table(self, lines):
        ids = [self.unit(" ".join(l.split()), "kind_table")
               for l in lines if not TABLE_RULE.match(l)]
        if not ids:
            return None
        i = len(self.tables)
        self.tables.append({"id": f"xdoc:t{i:04d}", "units": ids})
        return self.tables[-1]["id"]

    def work(self, e):
        """A reference:Reference, under reference.esl's documented IRI convention
        (`urn:eigenius:reference:lit:<first-author>_<year>`), de-duplicated by that key."""
        key = e["key"]
        if key in self.seen_keys:
            n = sum(1 for k in self.seen_keys if k.split("__")[0] == key) + 1
            key = f"{key}__{n}"
        self.seen_keys[key] = True
        iri = f"lit:{key}"
        self.works.append({"id": iri, **e})
        return iri

    def ref(self, e):
        i = len(self.refs)
        ann = self.paragraph([e["annotation"]]) if e["annotation"] else None
        units = self.paras[-1]["units"] if ann else []
        if ann:                      # a Ref carries its annotation units directly, not a Paragraph
            self.paras.pop()
        self.refs.append({"id": f"xdoc:r{i:04d}", "work": self.work(e), "units": units})
        return self.refs[-1]["id"]

    def reflist(self, title, children=None, refs=None):
        i = len(self.reflists)
        self.reflists.append({"id": f"xdoc:rl{i:02d}", "title": title,
                              "children": children or [], "refs": refs or []})
        return self.reflists[-1]["id"]

    def section(self, title, kind, children):
        self.sections.append({"id": f"xdoc:sec_{slug(title)}", "title": title,
                              "kind": kind, "children": children})
        if kind is None:
            self.unkinded.append(title)
        return self.sections[-1]["id"]


def build(doc):
    """The report's blocks → the tree. Section depth drives nesting; `## References` goes to back
    matter, where a wholly-bold line opens a titled group and a run of `- ` lines fills it."""
    b = Builder()
    title, body, back = None, [], []

    # Group the blocks by the heading that precedes them, keeping document order.
    groups, cur = [], None
    for kind, lines, lineno in blocks(doc):
        if kind == "heading":
            m = HEADING.match(lines[0])
            cur = {"depth": len(m.group(1)), "title": m.group(2).strip(), "blocks": []}
            groups.append(cur)
        elif cur is None:
            groups.append({"depth": 0, "title": None, "blocks": [(kind, lines, lineno)]})
            cur = groups[-1]
            if kind == "frontmatter":
                b.frontmatter = lines
        else:
            cur["blocks"].append((kind, lines, lineno))

    for g in groups:
        if g["depth"] == 1 and title is None:
            # The h1 is the document's title (JATS <article-title>), not a section. Any blocks
            # under it before the first h2 are body paragraphs with no section of their own.
            title = g["title"]
            body.extend(x for x in (content(b, g["blocks"])) if x)
            continue
        if g["title"] is None:
            # Blocks before any heading: body content with no section of its own.
            body.extend(x for x in content(b, g["blocks"]) if x)
            continue
        if g["title"].strip().lower() in BACK_MATTER:
            back.append(back_matter(b, g["title"], g["blocks"]))
            continue
        kids = [x for x in content(b, g["blocks"]) if x]
        body.append(b.section(g["title"], section_kind(g["title"]), kids))

    return b, title, body, back


NON_CONTENT = {"frontmatter", "comment"}


def content(b, blks):
    """A section's blocks → Paragraphs and Tables, in document order. Front matter and comments are
    not content: metadata and a private authoring note are not things the document asserts."""
    for kind, lines, _ in blks:
        if kind in NON_CONTENT:
            b.excluded.append((kind, len(lines)))
            continue
        yield b.table(lines) if kind == "table" else b.paragraph(lines)


def back_matter(b, title, blks):
    """§References → a titled enc:RefList. A wholly-bold single line opens a group; a block whose
    every line is a `- ` bullet fills the open group (or the root list, if none is open); anything
    else is prose. A block MIXING bullets and prose is recorded as a defect, not split by guess."""
    children, open_group = [], None
    for kind, lines, _ in blks:
        if kind in NON_CONTENT:
            b.excluded.append((kind, len(lines)))
            continue
        if kind == "table":
            children.append(b.table(lines))
            continue
        if len(lines) == 1 and ALL_BOLD.match(lines[0].strip()):
            open_group = b.reflist(ALL_BOLD.match(lines[0].strip()).group(1).strip())
            children.append(open_group)
            continue
        bullets = [BULLET.match(l) for l in lines]
        if all(bullets):
            target = next(r for r in b.reflists if r["id"] == open_group) if open_group else None
            if target is None:
                open_group = b.reflist(None)
                children.append(open_group)
                target = b.reflists[-1]
            for m in bullets:
                e = parse_entry(m.group(1).strip())
                if e is None:
                    b.bad_entries.append(m.group(1)[:60])
                    continue
                target["refs"].append(b.ref(e))
            continue
        if any(bullets):
            b.bad_entries.append("MIXED BLOCK: " + lines[0][:60])
            continue
        children.append(b.paragraph(lines))
    return b.reflist(title, children=[c for c in children if c])


# ── ESL emission ──────────────────────────────────────────────────────────────────────────────────

ESL_HEADER = '''// The XIAP report as D101's document model — a JATS tree.
//
// GENERATED by experiments/parsing/uab-xiap-document-model.py. Derived from gitignored material
// (`experiments/uab`), so this file is gitignored too (.gitignore:117): an encoding of the report
// is the report in another notation.
//
// enc:Document -> enc:body -> enc:Section -> enc:children -> enc:Paragraph | enc:Table
//                                                         -> enc:units    -> enc:DiscourseUnit
//              -> enc:back -> enc:RefList -> enc:refs     -> enc:Ref      -> reference:Reference
//
// Kinds and structure follow the ELEMENT, never a heuristic (D96). The h1 is enc:document_title,
// not a section; a heading is enc:title on the section it heads, not a unit.
//
// enc:span_start / enc:span_end are DERIVED from each unit's enc:locator (a WICG Text Fragment),
// resolved against the document's normalized text content — never authored, and never `doc.find`
// of a bare string, which maps a repeated sentence to its first occurrence.
//
// No enc:ReferenceSpan: markdown has no <xref>, so recognising a citation is a derivation and does
// not belong in a transcription (D101).

namespace core      = "urn:eigenius:core";
namespace reference = "urn:eigenius:reference";
namespace enc       = "urn:eigenius:encoding";
namespace lit        = "urn:eigenius:reference:lit";
namespace xdoc       = "urn:eigenius:uab:xiap-report";
'''


def esc(s):
    return s.replace("\\", "\\\\").replace('"', '\\"')


def arr(ids):
    return "[" + ", ".join(ids) + "]"


def emit(b, title, body, back, report, digest, out_path):
    p = [ESL_HEADER, ""]

    p.append('resource xdoc:report : enc:Document {\n'
             '    enc:document_title = "%s";\n'
             '    enc:media_type     = "text/markdown";\n'
             '    enc:source_path    = "%s";\n'
             '    enc:source_sha256  = "%s";\n'
             '    enc:body           = %s;\n'
             '    enc:back           = %s;\n'
             '    core:description   = "%d sections, %d units. Spans are offsets into the '
             'NORMALIZED text content (whitespace collapsed), the base every enc:locator resolves '
             'against.";\n}\n'
             % (esc(title), esc(os.path.basename(report)), digest, arr(body), arr(back),
                len(b.sections), len(b.units)))

    for s in b.sections:
        kind = '    enc:section_kind = enc:%s;\n' % s["kind"] if s["kind"] else ""
        p.append('resource %s : enc:Section {\n    enc:title        = "%s";\n%s'
                 '    enc:children     = %s;\n}\n'
                 % (s["id"], esc(s["title"]), kind, arr(s["children"])))

    for x in b.paras:
        p.append('resource %s : enc:Paragraph {\n    enc:units = %s;\n}\n' % (x["id"], arr(x["units"])))
    for x in b.tables:
        p.append('resource %s : enc:Table {\n    enc:units = %s;\n}\n' % (x["id"], arr(x["units"])))

    for r in b.reflists:
        t = '    enc:title    = "%s";\n' % esc(r["title"]) if r["title"] else ""
        kids = '    enc:children = %s;\n' % arr(r["children"]) if r["children"] else ""
        refs = '    enc:refs     = %s;\n' % arr(r["refs"]) if r["refs"] else ""
        p.append('resource %s : enc:RefList {\n%s%s%s}\n' % (r["id"], t, kids, refs))

    for r in b.refs:
        u = '    enc:units = %s;\n' % arr(r["units"]) if r["units"] else ""
        p.append('resource %s : enc:Ref {\n    enc:work  = %s;\n%s}\n' % (r["id"], r["work"], u))

    for w in b.works:
        f = ['    reference:title           = "%s";' % esc(w["title"])]
        if w["creators"]:
            f.append('    reference:creator         = "%s";' % esc(w["creators"]))
        if w["container_title"]:
            f.append('    reference:container_title = "%s";' % esc(w["container_title"]))
        if w["issued_year"]:
            f.append('    reference:issued_year     = %d;' % w["issued_year"])
        if w["pmid"]:
            f.append('    reference:pmid            = "%s";' % w["pmid"])
        p.append('resource %s : reference:Reference {\n%s\n}\n' % (w["id"], "\n".join(f)))

    for u in b.units:
        loc = '    enc:locator    = "%s";\n' % esc(u["locator"]) if u.get("locator") else ""
        a, c = u.get("span") or (0, 0)
        p.append('resource %s : enc:DiscourseUnit {\n'
                 '    enc:prose      = "%s";\n'
                 '    enc:unit_kind  = enc:%s;\n%s'
                 '    enc:span_start = %d;\n'
                 '    enc:span_end   = %d;\n}\n'
                 % (u["id"], esc(u["prose"]), u["kind"], loc, a, c))

    open(out_path, "w", encoding="utf-8").write("\n".join(p))


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    report, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    doc = open(report, encoding="utf-8").read()
    digest = hashlib.sha256(doc.encode("utf-8")).hexdigest()
    base = normalized(doc)

    b, title, body, back = build(doc)

    for u in b.units:
        frag, span = text_fragment(base, u["prose"])
        u["locator"], u["span"] = frag, span
        if frag is None:
            b.unresolved += 1

    emit(b, title, body, back, report, digest,
         os.path.join(out_dir, "xiap-document-model.esl"))
    json.dump({"document": os.path.basename(report), "sha256": digest,
               "media_type": "text/markdown", "title": title,
               "span_base": "normalized text content, whitespace collapsed",
               "body": body, "back": back, "sections": b.sections,
               "paragraphs": b.paras, "tables": b.tables,
               "reflists": b.reflists, "refs": b.refs, "works": b.works, "units": b.units},
              open(os.path.join(out_dir, "document-model.json"), "w", encoding="utf-8"),
              ensure_ascii=False, indent=1)

    by_kind = {}
    for u in b.units:
        by_kind[u["kind"]] = by_kind.get(u["kind"], 0) + 1
    print(f"document    {os.path.basename(report)}")
    print(f"sha256      {digest[:16]}")
    print(f"title       {title}")
    print(f"body        {len(body)} top-level blocks")
    print(f"sections    {len(b.sections)}   " + "  ".join(
        f"{k} {sum(1 for s in b.sections if s['kind'] == k)}"
        for k in sorted({s["kind"] for s in b.sections if s["kind"]})))
    print(f"paragraphs  {len(b.paras)}")
    print(f"tables      {len(b.tables)}")
    print(f"reflists    {len(b.reflists)}  refs {len(b.refs)}  works {len(b.works)}  "
          f"annotated {sum(1 for r in b.refs if r['units'])}")
    print(f"units       {len(b.units)}   " + "  ".join(f"{k} {v}" for k, v in sorted(by_kind.items())))
    print(f"locators    {len(b.units) - b.unresolved} resolved, {b.unresolved} UNRESOLVED")
    if b.unkinded:
        print(f"no sec-type {len(b.unkinded)}: " + "; ".join(b.unkinded))
    if b.excluded:
        agg = {}
        for k, n in b.excluded:
            agg[k] = agg.get(k, 0) + n
        print("excluded    " + "  ".join(f"{k} {v} lines" for k, v in sorted(agg.items()))
              + "  (not content: metadata and authoring comments)")
    if b.bad_entries:
        print(f"UNPARSED    {len(b.bad_entries)}: " + "; ".join(b.bad_entries))


if __name__ == "__main__":
    main()
