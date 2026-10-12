# D102 — The ingest seam: how the document model reaches the parser

**Status: proposed** (`2026-10-11`). D96 chose JATS as the document model and D101 gave the encoding
ontology the vocabulary for it. Neither says how a document reaches the parser, and today it does
not: the model is built at one end, the pipeline reads a text file at the other, and the two never
meet. This specifies the seam.

## The gap

| | |
|---|---|
| D96 | the model, and five ingest routes — and states plainly that "the ingest routes below is design, not yet built" |
| D101 | the vocabulary: `enc:Document`, `Section`, `Paragraph`, `Table`, `Ref`, `unit_kind`, `locator`. Built, reseeded, in `BOOTSTRAP_CHAIN` |
| the CLI | `pipeline.rs:174` — `read_to_string(&args.source)`, a flat `String` |
| the served path | `EncodingFormalizer` takes `source_text: String` + `source_path: String`; `encode_with_layer(&req.source_text)` |
| the artifact | `enc:ReasoningStructure.enc:document`, which nothing populates with an `enc:Document` |

So there is a model, a vocabulary, and a pipeline that consumes bytes.

**What the flattening costs, measured.** The XIAP converter produced 450 `enc:DiscourseUnit`s in a
tree, each carrying `enc:unit_kind`, `enc:locator` and its position. The measurement then ran over
`docmodel-page.txt` — the same 450 units, one per line, with everything else dropped. The units
exist twice and the tree's knowledge is discarded in between.

`enc:unit_kind` is documented as the routing switch: "What kind of unit this is (prose / equation /
citation / table) — selects the routing: only `prose` enters the DCG parser." The converter sets
`kind_table` on all 125 table rows. The parser never sees it, because its input is a text file with
no kinds. **110 table rows and 14 non-prose fragments were therefore scored as grammar failures —
124 units, 37% of `grammar-gap 392`** (`uab-xiap-grammar-gaps.md`). The switch is specified and
unreachable.

## The seam

**The pipeline consumes an `enc:Document`, not bytes.** A document is chain-resident (D101), so an
IRI is the platform's own idiom, and it makes `enc:ReasoningStructure.enc:document` correct by
construction rather than by convention.

```
  bytes → <route> → enc:Document ──→ parse ──→ enc:ReasoningStructure
                         │                          └─ enc:document → that Document
                         └─ enc:body/back → units, each with unit_kind + locator
```

**Every route mints a Document first, including plain text.** That removes the workaround already in
the tree: `emit_resources` mints a `reference:Reference` for a text file whose own description reads
"Minted document-locally — the file carries no DOI or PMID" (`emit.rs:251-262`). A plain text file
is a document with one implicit section; saying so is cheaper than the fiction that it is a
published work.

## Decisions

**1. The document model segments. The pipeline does not.**

Re-segmenting is not idempotent. `segment_sentences` over the certification log's 427 claim spans
returns 653 units, splitting 90 and merging 5 pairs, "after which no outcome can be attributed to
the claim that produced it" (`uab-xiap-corpus.py`). `--units-per-line` exists precisely to suppress
the second segmentation — the flag is today's fragile statement of this decision, passed by hand and
silently wrong if forgotten. With a Document the units ARE the units and the flag goes away.

**2. `enc:unit_kind` routes, as its description already says.**

Only `kind_prose` enters the DCG. `kind_table` is captured structurally, `kind_equation` routes to
FormulaTerm, `kind_citation` to `reference:Citation`. Nothing about the grammar changes; 124 units
leave the gap count because they were never the grammar's to answer for.

**3. The artifact names the Document, and the provenance narrows to it.**

D101 widened three properties rather than moving them, because the plain-text route emitted no
Document. Once route 4 mints one, they narrow: `enc:document` to `class_types enc:Document` alone,
`enc:source_path` and `enc:source_sha256` to `domain enc:Document`. `enc:source_document` comes off
`DiscourseUnit` — containment reaches the work, and a second home for one fact disagrees.

**4. Locators resolve against the Document, and spans stay derived.**

D96's amendment already requires this: a unit's fragment identifier is relative to its document's
text content, read through `enc:media_type`, pinned by `enc:source_sha256`. With the Document in
hand the pipeline can recompute `enc:span_start`/`span_end` instead of trusting an authored pair.

**5. The glossary attaches to the Document, and is reusable from it.**

Today the augmentation is an in-memory overlay rebuilt every run — `glossary.rs` states it commits
no persistent layer. Attaching it to the Document makes a run's lexical closure a property of the
document rather than of the run:

- **Capture.** The minted individuals, their `lexicon:LexicalEntry` aliases and the `enc:LexicalGap`
  records become resources the Document names. The gap records already exist (written `2026-10-11`);
  what is missing is a home. With one they can carry `enc:gap_unit`, currently unset because an
  augmentation is built per document and the blocked unit is not known at that seam.
- **Reuse, keyed on the content hash.** A second run over the same `enc:source_sha256` loads the
  glossary instead of re-deriving it. That is what makes a run reproducible without re-deriving
  closure — and it is the only way an LLM-synthesised entry can be replayed rather than re-asked,
  which matters because `ResolutionMethod::LlmSynthesized` is the arm the remaining absent words
  need and it has no producer yet.
- **A durable home for what rests on nothing.** The 46 `UnevidencedFallback` entries on the XIAP
  report are reproducible but currently discarded. Committed and named, they can be reviewed,
  replaced by a better source later, or cited as the reason a claim is weak.

**This commits doc-local individuals to a chain**, which the in-memory overlay deliberately avoided.
The gap stream is what makes it acceptable: every entry carries the method that produced it, so a
reader can see which of a document's glossary rests on a published grammar and which on nothing. A
glossary without that record would be a chain full of unexplained individuals.

## Prior art

The unified-internal-representation shape is established, not novel, which lowers the risk on D96's
five-routes-to-one-model. Verified `2026-10-11`:

- **`allenai/s2orc-doc2json`** describes itself as PDF2JSON, TEX2JSON and **JATS2JSON** into one
  target schema, PDFs via Grobid. CORD-19's paper states it parses PMC's JATS "generating the same
  target S2ORC JSON format". Three input routes, one representation — D96's architecture, built and
  used at scale. Its PDF path is Grobid→TEI, which is the route D96 already names.
- **`titipata/pubmed_parser`** (cited as Achakulvisut et al. 2020) handles the PMC open-access JATS
  subset with per-element handlers.

**Two places this supports a decision above.** `parse_pubmed_table` returns caption, label, column
names and cell values — STRUCTURED capture, not prose parsing, which is what decision 2 routes
`kind_table` to. And it marks the field set D101 deferred as unexercised (0 captions, 0 labels in
the XIAP report): caption and label are the right fields when a document has them.

**The difference that matters is not a field, it is where the representation LIVES.** Every tool
above produces a document-shaped JSON — an intermediate artifact, checked by its own code, handed to
downstream NLP. There is no intermediate here. `enc:Document` and everything under it ARE chain
resources, so the unified representation is the knowledge graph itself.

Three consequences, and each is already visible in this session's work:

- **The representation is validated, not conventional.** `enc:body` and `enc:children` are
  `class_types enc:Block`, so a non-Block in a body is a `ClassTypeMismatch` from the kernel. A JSON
  schema is enforced by whatever reads it; this is enforced before the document exists.
- **Structure is queryable by the same mechanism as everything else.** "The methods" is a query over
  `enc:section_kind` because a section is a resource on a chain — not a string field a consumer must
  agree to interpret the same way.
- **Provenance, locators and the gap stream are in the same graph as the content.** `enc:LexicalGap`
  records, `enc:locator`, `variant:Allele` fields and the units they describe are one addressable
  structure, not a JSON document beside a separate record of what happened to it.

That is also why decision 3 above is cheap for us and would not be for them: passing an
`enc:Document` IRI to the pipeline is the platform's own idiom, where a JSON pipeline must pass the
blob.

**And it means no XML stage on the live route.** `lxml`/XPath is the right tool for querying a JATS
tree, and the route in use reads markdown, whose structure is line-oriented. When route 1 is built,
JATS maps onto these resources directly — there is no document-JSON in between to query.

**Two places we are deliberately stricter, and should stay so.**

| | prior art | here |
|---|---|---|
| a unit's section | `parse_pubmed_paragraph` returns a `section` STRING ("Background", "Discussion") | `enc:Section` is a resource with an `enc:section_kind`, so "the methods" is a query (D96, D101) |
| an inline citation | `reference_ids`, a list per PARAGRAPH | `enc:ReferenceSpan` — the unit, the target, the ref-type, the surface, a locator |

The paragraph-level list is cheaper and loses which mention cited what. D101 keeps the span and
declines to emit one from markdown at all, because markdown has no `<xref>` to read.

**And it sharpens the canonical-route question rather than answering it.** Every tool above treats
JATS as primary with Grobid→TEI as the fallback, because their corpora are PMC. Ours is
markdown-from-a-register. The conclusion is not to pick one: D96's amendment already made locators
**media-type-appropriate**, so JATS gets XPointer and markdown gets a Text Fragment without either
being canonical. That is the hedge paying off, and the open question below narrows to which route is
built next rather than which one the model privileges.

## Markdown through JATS, rather than a second graph emitter

**Decided: convert markdown-from-a-register to JATS, then ingest it by route 1.** One ingest route
is built and maintained instead of two, and JATS is the shape the model is named for. Two
constraints make it honest, and one sequencing rule stops it costing a measured result.

**Constraint 1 — the converter marks what it can identify RELIABLY, and records the rule.**

An earlier draft of this section argued the converter must emit only elements markdown licenses,
and must not synthesise `<xref>`, on the grounds that reading back our own inference as markup is
circular. That misread D96. Its rule is stated with its purpose attached: "`Fig. 2d` inside an
`<xref>` is a reference because the markup says so; **the quantity split never sees it, and no rule
has to guess**", and an `<xref>` becomes "one reference token, neither a quantity nor lexical
content". The class of error being removed is the PARSER guessing mid-parse — `2006` read as a
quantity, `Rigaud` as lexical content. "No rule has to guess" means no rule inside the parse.

A conversion step that identifies citations once, before parsing, and marks them is therefore what
the rule asks for. Whether the mark was authored or derived does not change that the quantity split
never sees the number. Provenance is handled at the document level, not per element: the JATS
Document is derived from the markdown one, so every `<xref>` in it is known to be inferred without
annotating each.

**What this recovers.** Markdown yields NO reference spans today, discarding every citation in the
document — measured on the XIAP report: 11 parenthesised author-year citations over 8 works (one
parenthesis carrying two), 23 bare `Author et al. YEAR` over 13 works, and 30+ `Author et al.` with
no year, resolvable only against the bibliography.

**The real constraint is precision, which differs by form.** The parenthesised form is a clean
pattern and the bibliography is in the same document to resolve against. The bare author-year form
is weaker, and the no-year form needs the bibliography to disambiguate. The converter should mark
the forms it can do reliably, and the `enc:Document` should record WHICH rule produced its marks —
the same discipline the glossary's `ResolutionMethod` applies to lexical closure, where a surface
closed by a published grammar is distinguished from one closed on nothing. A derived document whose
marks are uniform makes that one statement rather than per-element provenance.

**Constraint 2 — the JATS is a DERIVED manifestation; the markdown one is kept.** A locator is
media-type-appropriate (D96), so units ingested from JATS carry XPointers into the JATS — which is
not the file the author wrote. Dropping the markdown Document would break resolution back to the
author's bytes and make `enc:source_sha256` the hash of a file nobody authored. So:

```
  markdown Document ──prov:derivation──▶ JATS Document ──▶ parse ──▶ ReasoningStructure
   (author's bytes,                       (generated,
    Text Fragment locators)                XPointer locators)
```

Two manifestations of one work, related by derivation — the same shape D101 already uses to keep the
WORK, the MANIFESTATION and the DERIVATION apart. Which locator a consumer wants decides which
Document it resolves against.

**Sequencing — the direct path stays until route 1 is measured equivalent.** `uab-xiap-document-
model.py` emits the graph directly today and is measured: 450 units, 427 of 427 register locators
resolving, 702 resources validating. Replacing a measured path with an unmeasured one is how a
regression becomes invisible. Route 1 is equivalent when it reproduces those three figures from
generated JATS; until then the direct emitter is the reference, not the legacy.

## Cost

`enc:Document` already exists and is seeded, so this is not a reseed. It is a change to two public
surfaces — `EncodingFormalizer`'s request and `pipeline::Args` — and to `emit_resources`, whose
committed artifact fixtures regenerate. The narrowing in decision 3 touches the 13 code sites D101
measured.

## Open

- **Which route is built next** — not which is canonical, which the media-type-appropriate locator
  already settles (see Prior art). Markdown-from-a-register is the one in use; JATS-read-directly
  is what the prior art implements and what would exercise `enc:ReferenceSpan`, which markdown
  cannot.
- **Whether a reused glossary may be AMENDED.** Decision 5 keys reuse on the content hash, so the
  same bytes get the same closure. It does not say what happens when a better source later claims a
  surface the fallback took — whether the stored glossary is superseded, versioned, or left alone
  with the improvement visible only in a fresh run.
- **Whether `ProseUnits` and the lexicon scope vary per document.** `enc:ScopedUnit` exists for
  per-unit scope; nothing says whether a Document carries a default.
