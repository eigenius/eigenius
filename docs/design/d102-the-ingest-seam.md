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

**5. The glossary's gap stream attaches to the Document.**

`enc:LexicalGap` records gained a writer on `2026-10-11` and are emitted per run. With a Document
they can carry `enc:gap_unit`, which is currently unset because an augmentation is built per
document and the blocked unit is not known at that seam.

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
- **Whether the augmentation becomes per-document and persisted.** Today it is an in-memory overlay
  rebuilt every run. A Document-keyed glossary would make a run reproducible without re-deriving it,
  and would give the 46 unevidenced closures a durable home — but it also commits doc-local
  individuals to a chain, which is a separate decision.
- **Whether `ProseUnits` and the lexicon scope vary per document.** `enc:ScopedUnit` exists for
  per-unit scope; nothing says whether a Document carries a default.
