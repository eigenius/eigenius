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
| `<article>` | `enc:Document` | the manifestation: media type, path, content hash, title, an ordered `enc:body`, a `ref-list` |
| `<sec id= sec-type=>` | `enc:Section` | identity, title, an `enc:SectionKind`, ordered children (sections or blocks) |
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
whose shape is a guess. **Decided: define `Document`, `Section`, `Paragraph`, `Table` and
`ReferenceSpan` now; defer `figure`, `code` and caption units to the first document that has one.**

### Provenance has one root, and it is the document

`reference:Reference` cannot hold a document's structure, by its own ontology's design rule:
"Anything local to the citing document — the in-text reference number, the CiTO function, the
imported claim — lives on the Citation, never on the Reference (a different paper citing the same
work numbers it differently)" (`ontologies/reference/reference.esl`). A paragraph order and a byte
offset are manifestation-local in exactly that sense. `reference:Reference` is also defined as "A
published work cited by an encoding", identified by DOI or PubMed ID; the XIAP report is unpublished
and has neither.

**Decided: three resources on three axes, chained, none subsuming another.**

| | axis | carries |
|---|---|---|
| `reference:Reference` | the WORK, when published | DOI/PMID, CSL-JSON metadata; the `<ref-list>` entries |
| `enc:Document` | the MANIFESTATION — observed | media type, path, content hash, structure |
| `enc:ReasoningStructure` | the DERIVATION — derived | `enc:claims`, `prov:derivation` → one ProgramTrace |

`enc:ReasoningStructure` is already explicitly "DERIVED: this is the output of a program run over
hashed input". A document model is a transcription of what the source says. Merging them would put
an observation and a derivation in one resource, which is the kind-axis collapse the step-5 plan
note warns about.

So three properties move, because they describe the input and today sit on the output:
`enc:document` retypes from `reference:Reference` to `enc:Document`; `enc:source_path` and
`enc:source_sha256` move their domain from `enc:ReasoningStructure` to `enc:Document`. The hash
belongs there regardless — D96's amendment resolves every locator "against a hashed document", so
the hash is a property of the thing the locators address.

Per-unit provenance then follows containment. `enc:source_document` on a `DiscourseUnit` is
derivable by walking up to the `Document`, and a second home for the same fact is a disagreement
waiting to happen, so it comes off.

**Cost, measured:** 13 code sites in 3 files — `crates/eigenius-encoding/src/emit.rs`,
`crates/eigenius-pmi-registry/src/convert.rs`,
`crates/eigenius-encoding/tests/artifact_completeness.rs`. (`enc:document` 1, `enc:source_sha256` 1,
`enc:source_path` 0, `enc:source_document` 4, `enc:section` 5, `enc:span_start`/`span_end` 1 each.)
The larger grep counts — 476, 497, 473 — are the 467-unit emitted ESL, not code.

### Section types: JATS's values, in an open class

JATS's `sec-type` list is **suggested, not controlled**. The BITS 2.1 tag library says the attribute
"may take any value" and names the JATS article recommendations; JATS4R draws the same distinction
("controlled" values must come from a list, "suggested" values should) and notes that controlled
lists hurt adoption when a user has a legitimate need for an unlisted value.

So `enc:SectionKind` is a class of individuals, open by construction, carrying JATS's nine suggested
values: `cases`, `conclusions`, `discussion`, `intro`, `materials`, `methods`, `results`, `subjects`,
`supplementary-material`. Individuals, not a string, is what makes "the methods" a query. Open, not
closed, because JATS's own is — a layer that needs a tenth adds it under `core:subclass_of` rather
than through an enumerated selector.

**Decided: use JATS's values; mint no document-local section vocabulary.** The XIAP report's 15
headings:

| heading | maps to | n |
|---|---|---|
| `# XIAP p.Ile380Thr bystander edit risk assessment` | `<article-title>`, in front matter — not a `<sec>` | 1 |
| `## Bottom line up front` | `conclusions` — a BLUF is the conclusion, placed first | 1 |
| `## The variant and its setting` | `intro` | 1 |
| `## Finding 1 … 10` | `results` | 10 |
| `## Methods` | `methods` | 1 |
| `## References` | `<ref-list>`, in back matter — not a `<sec>` | 1 |

Ten of thirteen body sections are findings, and "finding" is not a JATS value. Typing them `results`
loses nothing: a query for the results returns them, and "finding 4" is answered by section IDENTITY
and title, which are a resource and a string on that resource, not a `sec-type`. A document-local
`sec-type` would buy only what identity already provides, while buying no cross-document query —
which is the entire job of `sec-type`. Document order carries the BLUF-first arrangement, so
`conclusions` appearing first is representable without a second ordering concept.

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

`encoding` is a bootstrapped ontology, so this is a reseed (~30 min, plus snapshot and baselines).
Four bootstrap edits land together rather than separately: the structure classes and `SectionKind`;
the three provenance properties retyped or re-domained; `enc:section` ranging over a resource; and
the `enc:locator` property that lets a unit carry its Text Fragment on the chain instead of in a
sidecar.

## Open

- **Whether the BLUF is `conclusions` or an `<abstract>`.** JATS puts `<abstract>` in
  `<article-meta>`, and this BLUF is body prose under its own `##` heading. Typed `conclusions`
  above; if the ingest routes later want front-matter abstracts, the two readings diverge.
- **`enc:ReferenceSpan` offsets against which base.** The spans are inside a unit, so unit-relative
  offsets are the obvious base, but D96 derives unit offsets from a document-level locator. Whether
  a span gets its own Text Fragment or an offset pair into its unit is unsettled.

## Sources

- [JATS 1.3 `sec-type`](https://jats.nlm.nih.gov/publishing/tag-library/1.3/attribute/sec-type.html)
- [BITS 2.1 `sec-type`](https://jats.nlm.nih.gov/extensions/bits/tag-library/2.1/attribute/sec-type.html) — "may take any value"
- [JATS4R attribute framework](https://jats4r.niso.org/jats4r-attribute-framework/) — controlled vs suggested
