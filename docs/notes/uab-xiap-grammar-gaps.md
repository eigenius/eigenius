# The shape of the grammar gaps — survey §4 step 3

**Measured `2026-10-11`**, store `wordnet-umls-hpo-aligned-2026-10-10-d101`, release,
cap-only (`--no-llm`), `--units-per-line`, over the document model's 450 units. Run directory and
corpus are gitignored; only these numbers are tracked.

Step 2 took `missing-lexeme` to 0, so every unit now reaches the grammar and `grammar-gap 392` is
the whole of what stands between this document and `baseline.json`'s coverage gate.

## 392 is not one number

| | | |
|---:|---:|---|
| 110 | 28% | **not prose** — markdown table rows |
| 14 | 4% | **not prose** — fragments with no sentence-final punctuation |
| 20 | 5% | **converter segmentation** — under 5 tokens, not sentences |
| 162 | 41% | **real** — ordinary sentences, 5–30 tokens |
| 86 | 22% | **real** — long sentences, 31+ tokens |

**248 (63%) are genuine grammar coverage gaps.** The remaining 144 belong to the document model.

## The closure did not relocate the problem

Of the 392, **29 (7%)** contain a token the unevidenced fallback minted. That was the risk in
closing `missing-lexeme` with opaque individuals — that a unit would clear the lexicon only to fail
the grammar on an entry that composes with nothing — and it did not materialise: **363 would be
gaps regardless**.

## 124 units are not prose and should never have reached the grammar

Table rows alone are 28% of every gap. Scoring them as grammar failures makes the grammar look
worse than it is, and the routing information already exists: D101 types them `enc:Table` and the
converter sets `enc:kind_table`, which `enc:unit_kind` is documented to route — "only `prose`
enters the DCG parser". The measurement is not using it.

Fixing this is a measurement-validity fix, not a grammar fix. Until it lands, 37% of the gap count
is mislabelling.

## 20 are the converter's bad splits

`Compara.` is a one-word unit. Its source is a bibliography annotation —
`… **PMID 26896847.** Compara. (PMID 27141089 is the published …` — which the sentence splitter cut
at the period before a capital. Same class as the `et al.` defect fixed earlier the same day: the
splitter cuts on punctuation-then-capital, and an annotation defeats it.

## What the real 248 look like

Structural features, overlapping, over all 392:

| | |
|---:|---|
| 25% | em- or en-dash clause |
| 24% | parenthetical |
| 21% | inline code span |
| 14% | colon-introduced list |
| 12% | semicolon series |
| 6% | quoted speech |

That is the embedded-structure profile of this prose. The plain-prose residue — none of the above —
is 108 units, and includes sentences as simple as «The dominant-negative route that XIAP
self-association would otherwise open is not physically available.» (a relative clause) and «It is
the same residue, carried by 4 hemizygous men, and ClinVar-Benign.» (coordination with a
participial adjunct).

## Order of work

1. **Route non-prose out** (124 units). `enc:unit_kind` already says how; the harness does not read
   it. Nothing about the grammar changes, and the gap count becomes attributable.
2. **Fix the annotation splits** (20 units) in `uab-xiap-document-model.py`.
3. **Then characterise the 248** by what the grammar actually lacks, measuring against a number
   that means what it says.

Taking (3) first would attribute 144 document-model defects to grammar coverage.
