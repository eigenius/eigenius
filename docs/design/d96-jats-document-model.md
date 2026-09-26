# D96 — JATS as the document model

**Status: decided** (2026-09-26) — the model. Its realisation in the encoding vocabulary and the
ingest routes below is design, not yet built.

## The gap

The pipeline sees a paper as text. For the WRN corpus that text comes from the PDF's text layer,
selected by geometry: `references/publications/WRN-Helicase-Nature-OCR/extract-section.py` keeps
lines by type-size band, because "content rules for 'is this a figure caption' were tried and
abandoned". Whatever structure the paper had is inferred or lost, and three things follow:

- **References read as content.** D95's numeral/unit split reads the unbracketed `Fig. 2d` in the
  WRN methods as *two days*, and would read `Fig. 2h` as two hours. Nothing in plain text says
  `Fig. 2d` is a reference to a figure panel rather than a quantity.
- **The encoding vocabulary has kinds nothing assigns.** `enc:unit_kind` routes a `DiscourseUnit`
  — only `prose` enters the parser — but the encoder hard-codes every unit to `kind_prose`
  (`crates/eigenius-encoding/src/emit.rs:649`, D95), and `kind_table` is described as "tabular/
  figure content", conflating two things.
- **Provenance cannot name the version.** `reference:Reference` records a DOI, PMID, title,
  creators and URL. The WRN paper's two texts differ — Nature's version of record says "spun at
  **931g** for **2 h** at 30 °C", the PubMed Central author manuscript "spun at **931 RCF** for **2
  hours** at 30°C" — and nothing on the chain says which one a claim was read from.

## The decision

**JATS — the Journal Article Tag Suite, ANSI/NISO Z39.96 — is the reference model for document
structure.** Its element set and its reference semantics, not necessarily its XML as a storage
format: the pipeline's notion of "a figure", "a caption", "a reference to a table" is JATS's.

It is the archive format of PubMed Central and what most biomedical publishers produce, so for the
literature this system reads it is the structure that already exists, rather than one to invent.

## What JATS supplies

| Structure | JATS | What the pipeline gains |
|---|---|---|
| figure, and a multi-panel figure | `<fig>`, `<fig-group>` | a unit that is not prose |
| table | `<table-wrap>` | the same, distinct from a figure |
| display formula | `<disp-formula>` | routing to formula capture |
| program listing | `<code>` (with `language`), `<preformat>` | a unit that is not prose |
| supplementary material | `<supplementary-material>` | the same |
| label and caption | `<label>` ("Fig. 2"), `<caption>` | the caption as its own unit |
| a reference to any of these | `<xref ref-type="fig" rid="F2">Fig. 2d</xref>`, also `table`, `disp-formula`, `bibr`, `supplementary-material` | **a reference span, known by markup** |
| section | `<sec sec-type="methods">` with `<title>` | a section by identity, not a string |

The last-but-one row is the one that removes a class of error rather than a case. `Fig. 2d` inside
an `<xref>` is a reference because the markup says so; the quantity split never sees it, and no rule
has to guess.

## Alignment with the encoding vocabulary

- **Discourse kinds.** `enc:DiscourseUnitKind` has prose, equation, citation and table. JATS adds a
  **figure** kind, split out of `kind_table`, and a **code** kind for listings; a caption is a unit
  of its own, linked to its figure or table. Assignment follows the element, not a heuristic: `<p>`
  text is prose, `<disp-formula>` equation, `<table-wrap>` table, `<fig>` figure, `<code>` code.
- **Reference spans.** New: an `<xref>` inside a prose unit becomes a span — offsets, reference
  type, target identifier — that D95's preprocessor treats as one reference token, neither a
  quantity nor lexical content. D95's open question about figure references is answered here for
  any source with markup.
- **Sections.** `enc:section` is a free string ("Results §2.1"). With JATS it can carry the section's
  identity and `sec-type`, so "the methods" is a query, not a string match.
- **Offsets.** `enc:span_start`/`span_end` are offsets "in the source document", which today means
  the extracted text. With JATS they need a defined base — the document's text content in document
  order is the natural one, and it has to be fixed before anything records offsets against it.
- **Version.** `reference:Reference` gains the manifestation read: a PMCID beside the DOI and PMID,
  and whether the text is the version of record or an accepted manuscript. JATS carries this in
  `<article-meta>`.

## Three routes to one model

| Source | Route | Inline references |
|---|---|---|
| PMC or publisher JATS, where licensed | read directly | marked (`<xref>`) |
| BioC (NCBI's text-mining form of PMC) | typed passages with offsets | **dropped** — flattened to text |
| a PDF with no JATS | GROBID → TEI, mapped onto the model | marked (`<ref type="figure">`) |
| plain text, as today's corpus | none | none — D95's heuristic question remains |

GROBID is the established extractor for scientific PDFs; it would replace the geometric
`extract-section.py`, which infers from type size what TEI states. Plain text keeps the need for a
fallback rule in D95's preprocessor, but only there.

## Found checking the WRN paper

- **It is in PubMed Central**, PMC6580861 (PMID 30971823). Its BioC form types every passage:
  1 title, 2 abstract, 16 introduction, 57 methods, 28 figure captions, 8 supplementary, 50
  references — the separation `extract-section.py` recovers from type sizes.
- **BioC loses inline markup**: `(Fig. 2d)` arrives as plain text. The full JATS from Europe PMC's
  `fullTextXML` returned a server error (HTTP 500) on 2026-09-26, so whether this paper's JATS marks
  its figure references is not yet confirmed.
- **The PMC text is the author manuscript and is not CC-licensed** (`license: NO-CC CODE`). It can be
  fetched and processed; it cannot be committed to this repository.
- **The two versions word the same step differently**, and one of D95's hazards is an artefact of
  that: `931g` is Nature's typesetting of what the manuscript writes as `931 RCF`.

## Scope

**In.** The model: JATS's structure and reference semantics as the pipeline's. The alignment above
as the specification for revising `ontologies/encoding/encoding.esl` and `reference:Reference`.

**Out, as separate work.** The ingest crates for each route; deploying GROBID; the choice of storage
format for fetched documents.

## Open questions

- **The offset base** — what `span_start` counts in when the source is JATS, and whether offsets
  already recorded against extracted text are migrated or left as they are.
- **Captions** — whether a caption is parsed as prose. D95 records that figure legends use an
  elliptical register the grammar does not cover; a caption unit could be captured and not parsed,
  as a table is.
- **Licence and `enc:prose`.** `enc:prose` stores the verbatim source text on the chain. For a text
  that is not openly licensed, a shared chain redistributes it. Whether `enc:prose` holds the text,
  a hash and offsets into a document fetched separately, or depends on the licence, is undecided.
- **Which version is canonical** when a paper exists as both version of record and author
  manuscript and they differ in wording.
