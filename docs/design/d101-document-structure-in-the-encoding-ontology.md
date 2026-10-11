# D101 — Document structure in the encoding ontology

**Status: implemented** (`2026-10-10`). Realises D96's scope — "the alignment above as the
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
| `<article>` | `enc:Document` | the manifestation: `enc:media_type`, `enc:source_path`, `enc:source_sha256`, `enc:document_title`, an ordered `enc:body`, an `enc:back`, and `enc:work` when the source is published |
| — | `enc:Block` | the abstract element type of `enc:body` and `enc:children`: one type to walk, and a later `figure` or `code` needs no edit to containment |
| `<sec id= sec-type=>` | `enc:Section` | `enc:title`, an `enc:section_kind`, and ordered `enc:children` (sub-sections and blocks interleaved) |
| `<p>` | `enc:Paragraph` | `enc:units` — an ordered run of `DiscourseUnit`s, one per sentence |
| `<table-wrap>` | `enc:Table` | `enc:units` — one per row; separate from `Paragraph` because its units route to `kind_table`, by element and not by guess |
| `sec-type` | `enc:SectionKind` | JATS's nine suggested values, as individuals |
| `<ref-list>` | `enc:RefList` | `enc:title`, `enc:refs` (the entries), `enc:children` (sub-groups and prose) — it nests, because real bibliographies do |
| `<ref>` | existing `reference:Reference` | one cited work, reached through `enc:refs` |
| `<xref>` | `enc:ReferenceSpan` | `enc:span_unit`, `enc:ref_target`, `enc:ref_type`, `enc:ref_surface`, `enc:locator` |
| `ref-type` | `enc:ReferenceType` | the five values D96 names: `bibr`, `table`, `fig`, `disp-formula`, `supplementary-material` |
| — | `enc:locator` | the fragment identifier, on a `DiscourseUnit` or a `ReferenceSpan` |

A reference span is a text region, so D96's locator rule applies to it unchanged: it is addressed
by its own fragment identifier, not by an offset pair into its unit. That keeps one addressing
scheme for every region in the document.

**Containment is explicit and ordered.** `core:resource_array` is the existing means —
`enc:scope`, `enc:antecedent_resources` and `enc:claims` all use it — so a section's children are an
array, in document order, and the tree is walkable rather than reconstructed from a string.

**`DiscourseUnit` is derived, not authored.** It stays exactly what it is, "one unit of source text
the pipeline attempts to encode", and hangs off a `Paragraph` or a `Table` row.

**`enc:section` is not retyped.** D96 reads as an upgrade of that field — "`enc:section` is a free
string ... with JATS it can carry the section's identity and `sec-type`". It is realised instead by
Sections BEING resources. Inside a `Document` a unit's section is reached by walking DOWN
(`document → body → section → children → paragraph → units`), so a back-pointer would be
denormalization, and "the methods" is a query over `enc:section_kind` either way. Retyping would
also break the one caller that uses the field for what it says it is:
`crates/eigenius-pmi-registry/src/convert.rs:329` writes a case id, from a source that is a JSON
record and not a document. The field keeps that one honest job.

**What this document does not need.** Measured on the XIAP report: 0 figures, 0 code blocks, 0
captions. D96's alignment lists a `figure` kind split out of `kind_table`, a `code` kind, and
captions as units of their own; none is exercised here. They belong in the vocabulary because the
model is JATS's, but nothing in this corpus tests them, and a class no instance exercises is a class
whose shape is a guess. **Decided: define `Document`, `Section`, `Paragraph`, `Table` and
`ReferenceSpan` now; defer `figure`, `code` and caption units to the first document that has one.**

### Back matter is part of the model

The first cut gave `enc:Document` a flat `enc:ref_list` of `reference:Reference`. The report's
bibliography does not fit in one: it is a leading provenance paragraph, **four** titled groups
totalling **63** entries, and a trailing paragraph of database accessions — prose and nesting and
titles, none of which an array of works can carry. (The plan note's "69 reference entries" counted
the paragraphs as entries.)

So `enc:RefList` is a `Block`, it nests (JATS's `<ref-list>` nests too), and it sits in
`enc:back` rather than in `enc:body`: a `## References` heading at the same markdown depth as the
argument's sections is not a section of the argument. `enc:title` replaced `enc:section_title` at
the same time, because a `RefList` has a title for the same reason a `Section` does, and JATS calls
the element `<title>` in both places.

The group headings are bold-only lines (`**Base editing**`), which an "entirely bold" test
separates from the **77** bold-LEAD paragraphs (`**Lead.** Then prose…`) elsewhere in the report —
including one inside §References itself (`**Database records consulted directly:** dbSNP …`). Read
off the markup, not guessed; the same distinction that caught the earlier 77 miscount.

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

**What the code already does instead.** `emit_resources` mints a `reference:Reference` for a plain
text file whose own description reads "Minted document-locally — the file carries no DOI or PMID"
(`crates/eigenius-encoding/src/emit.rs:251-262`). The workaround for `enc:Document`'s absence is
already in the tree; this vocabulary is what it stands in for.

**Migration posture.** `enc:document` gains `enc:Document` in its `class_types`;
`enc:source_path` and `enc:source_sha256` gain `enc:Document` in their domains. Widened, not moved:
the plain-text route emits no `Document`, so narrowing now would strand it with nowhere to record
the bytes it read. Each description names `enc:Document` as canonical and says the narrowing
follows once every route emits one. `enc:source_document` likewise stays on `DiscourseUnit` for
those routes, with its description recording that it is redundant inside a `Document` and must not
be set alongside containment.

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
4. **No reference spans, because markdown has no `<xref>`.** D96's rule is that a reference is
   known by markup; markdown writes a citation as ordinary prose, so there is no element to follow.
   Recognising one is a DERIVATION over the document, and a derivation does not belong inside a
   transcription that is otherwise entirely observed. `enc:ReferenceSpan` stays in the vocabulary
   for the JATS and XML routes, where `<xref>` exists, and the markdown route emits none.

   What a later derivation step has to handle, measured on this report's body: **11** parenthesised
   author-year citations over **8** works — one parenthesis carrying two
   (`(Damgaard et al. 2012; Aguilar et al. 2014)`); **23** bare `Author et al. YEAR` over **13**
   works; and **30+** `Author et al.` with no year at all, resolvable only against the
   bibliography. An earlier count of "9 over 8 works" saw one of the three forms, and missed
   `Gyrd-Hansen` because the pattern's character class excluded the hyphen.
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

`encoding` sits at the tip of `BOOTSTRAP_CHAIN` (`kernel/src/bootstrap/mod.rs:441`), so this is a
reseed (~30 min, plus snapshot and baselines), and `bootstrap_manifest_is_pinned` is updated in the
same commit — one line, `encoding:a7ce37f8…` → `encoding:94f5d43c…`, confirming nothing else moved.

**The file's own header denied this** until `2026-10-10`: it read "NOT in
`kernel/src/bootstrap` BOOTSTRAP_CHAIN, and no longer planned for it ... which keeps a vocabulary
change off the bootstrap manifest and therefore out of the reseed path." The chain entry's comment
records the reversal — "D71 slice 2 argued the other way to keep vocabulary edits off the reseed
path, and was wrong on the merits — the dependency was already there and unchecked (`2026-08-20`)"
— and the header was never updated. A reader planning a vocabulary edit off that header would have
budgeted nothing for a reseed. Corrected in the same commit as the vocabulary.

One edit, landing together: the structure classes, `SectionKind`, `ReferenceType`, `enc:locator`,
and the three widened provenance properties.

## Open

- **Whether the BLUF is `conclusions` or an `<abstract>`.** JATS puts `<abstract>` in
  `<article-meta>`, and this BLUF is body prose under its own `##` heading. Typed `conclusions`
  above; if the ingest routes later want front-matter abstracts, the two readings diverge.
- **Every route emitting an `enc:Document`.** Until the plain-text route does, three properties
  stay widened rather than moved (below), and `enc:source_document` stays on `DiscourseUnit`. This
  is the one piece of the one-root end-state that is not yet built.

## Sources

- [JATS 1.3 `sec-type`](https://jats.nlm.nih.gov/publishing/tag-library/1.3/attribute/sec-type.html)
- [BITS 2.1 `sec-type`](https://jats.nlm.nih.gov/extensions/bits/tag-library/2.1/attribute/sec-type.html) — "may take any value"
- [JATS4R attribute framework](https://jats4r.niso.org/jats4r-attribute-framework/) — controlled vs suggested
