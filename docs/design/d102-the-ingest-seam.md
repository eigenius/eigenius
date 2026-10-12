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

## Cost

`enc:Document` already exists and is seeded, so this is not a reseed. It is a change to two public
surfaces — `EncodingFormalizer`'s request and `pipeline::Args` — and to `emit_resources`, whose
committed artifact fixtures regenerate. The narrowing in decision 3 touches the 13 code sites D101
measured.

## Open

- **Which of the five routes is canonical.** Markdown-from-a-register is the one in use and the row
  D96 added last; JATS-read-directly is the one the model is named for. The answer determines
  whether `enc:locator` is primarily a Text Fragment or an XPointer.
- **Whether the augmentation becomes per-document and persisted.** Today it is an in-memory overlay
  rebuilt every run. A Document-keyed glossary would make a run reproducible without re-deriving it,
  and would give the 46 unevidenced closures a durable home — but it also commits doc-local
  individuals to a chain, which is a separate decision.
- **Whether `ProseUnits` and the lexicon scope vary per document.** `enc:ScopedUnit` exists for
  per-unit scope; nothing says whether a Document carries a default.
