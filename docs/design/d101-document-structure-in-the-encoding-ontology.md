# D101 — Document structure in the encoding ontology

**Status: proposed** (`2026-10-10`). Realises D96's scope — "the alignment above as the
specification for revising `ontologies/encoding/encoding.esl`" — by giving the encoding ontology the
document structure it has never had. Written for the XIAP report (survey §4 step 5), but the
vocabulary is the model's, not that document's.

## The gap

`ontologies/encoding/encoding.esl` has no class for a document's structure. Its classes are
`DiscourseUnit`, `ScopedUnit`, `LexicalGap`, `DecisionPoint`, `SelectionAuthority`, `AnaphorBinding`,
`CutItem`, `EncodedClaim`, `Claim` and its kinds — every one about the PARSE PIPELINE. The document
appears only as `enc:source_document`, a single resource reference, and the unit's place in it as
`enc:section`, a free string.

So a converter can emit a flat list of units tagged with a section slug and nothing will object. One
did: `experiments/parsing/uab-xiap-document-model.py` produced 467 `enc:DiscourseUnit`s for the XIAP
report, which compiled and validated clean, and contains no article, no sections as objects, no
containment, no reference list. **It is the leaves without the tree, and it validated because there
is nothing to validate against.**

That is the gap. D96 decided JATS as the model a fortnight ago; this is the vocabulary that decision
implies.

## What the output is

A JATS element set, as ESL classes. JATS's structure and reference semantics, "not necessarily its
XML as a storage format" (D96).

| JATS | class | carries |
|---|---|---|
| `<article>` | `enc:Document` | the work, its manifestation, an ordered `enc:body`, an `enc:back` |
| `<sec id= sec-type=>` | `enc:Section` | identity, title, `sec-type`, ordered children (sections or blocks) |
| `<p>` | `enc:Paragraph` | an ordered run of `DiscourseUnit`s |
| `<table-wrap>` | `enc:Table` | ordered rows; a caption when one exists |
| `<ref-list>` / `<ref>` | existing `reference:Reference` | the cited works, as the bibliography |
| `<xref>` | `enc:ReferenceSpan` | a span inside a prose unit: offsets, reference type, target |

**Containment is explicit and ordered.** `core:resource_array` is the existing means —
`enc:scope`, `enc:antecedent_resources` and `enc:claims` all use it — so a section's children are an
array, in document order, and the tree is walkable rather than reconstructed from a string.

**`DiscourseUnit` is derived, not authored.** It stays exactly what it is, "one unit of source text
the pipeline attempts to encode", and hangs off a `Paragraph` or a `Table` row. Its `enc:section`
becomes a resource reference rather than a slug, which is what makes "the methods" a query instead
of a string match (D96 §Alignment).

**What this document does not need.** Measured on the XIAP report: 0 figures, 0 code blocks, 0
captions. D96's alignment lists a `figure` kind split out of `kind_table`, a `code` kind, and
captions as units of their own; none is exercised here. They belong in the vocabulary because the
model is JATS's, but nothing in this corpus tests them, and a class no instance exercises is a class
whose shape is a guess. **Proposed: define `Document`, `Section`, `Paragraph`, `Table` and
`ReferenceSpan` now; defer `figure`, `code` and caption units to the first document that has one.**

## What the conversion process is

Markdown in, the structure above out. One rule governs it, from D96: **assignment follows the
element, never a heuristic.** Markdown states its own structure — `#` a heading, `|` a table row, a
blank line a paragraph boundary — so nothing is inferred. A first attempt that classified by regex
counted 77 "list items" that were bold-lead paragraphs; the register independently calls those
locations `¶`.

1. **Blocks.** Walk the source once. A `#` line opens a section at its depth; `|` lines accumulate
   into one table; blank lines close a paragraph. Sections nest by heading depth.
2. **Units.** A paragraph's text splits into sentences; a table yields one unit per row; a reference
   list yields one unit per LINE — an entry does not end in sentence punctuation followed by a
   capital, so a sentence splitter leaves whole blocks merged (measured: 24 units against 83 claims
   in §References before this was fixed).
3. **Locators.** Each unit gets a WICG Text Fragment resolved against the document's normalized text
   content, per D96's amendment; `span_start`/`span_end` are DERIVED from it, never authored.
4. **Reference spans.** An inline citation (`(Rigaud et al. 2006)` — 9 of them, over 8 works)
   becomes an `enc:ReferenceSpan` inside its prose unit, targeting the `reference:Reference` its
   bibliography entry mints.
5. **Emit and load through the kernel.** A rejection is a finding, not a silent drop.

### The register is not an input

The certification log joins to the result BY LOCATION, and is not consulted during conversion. Its
`text:` field is not a span of the document — 120 of 427 appear verbatim in any version, split by
extraction pass (pass 3 is 28/28 under its rule R12; passes 1–2 are ~23%) — so the earlier claims are
the certifier's canonical restatements. A claim and a unit are different artifacts about the same
place. 427 of 427 locators resolve to a section given three rules read off 475 uses: a trailing
sub-element is appended without a comma (`§Finding 4 table`), sections are abbreviated (`BLUF`,
`setting`), and `Finding N` matches by number because titles continue past it.

## Cost

`encoding` is a bootstrapped ontology, so this is a reseed. Three bootstrap edits should land
together rather than separately: these structure classes; `enc:section` ranging over a resource; and
the `enc:locator` property that would let a unit carry its Text Fragment on the chain instead of in
a sidecar.

## Open

- **`sec-type` values.** JATS has a suggested list (`intro`, `methods`, `results`, `discussion`).
  This report's sections are a title, a BLUF, a setting, ten findings, methods and references —
  "finding" is not a JATS `sec-type`. Whether to use JATS's vocabulary and type the findings as
  `results`, or to carry the document's own section vocabulary, is undecided.
- **Where a manifestation lives.** `enc:source_document` is `class_types reference:Reference`,
  described as "the published work this unit is drawn from". This report is unpublished. D96 already
  plans the manifestation read for `reference:Reference`; `enc:Document` may be the better home,
  with `reference:Reference` left to the bibliography.
- **Whether `enc:Document` subsumes `enc:ReasoningStructure`**, which already carries
  `enc:document` and `enc:claims` over one source work.
