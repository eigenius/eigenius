# OOV over the XIAP document model — survey §4 step 2

**Measured `2026-10-11`**, store `wordnet-umls-hpo-aligned-2026-10-10-d101` (the D101
reseed), release, cap-only (`--no-llm`), `--units-per-line`. Corpus: the document model's 450 units
(`uab-xiap-docmodel-corpus.py`), scored by `uab-xiap-score.py`. Run directory and corpus are
gitignored; only these numbers are tracked.

**Not comparable to step 4.** That measured the register's 427 claim spans — the certifier's
restatements. This measures the document's own text. Different corpora about the same report.

## Coverage

| block | n | covered | rate | outcomes |
|---|---:|---:|---:|---|
| paragraph (prose) | 256 | 8 | 3.1% | MISSING 149, GAP 94, AMBIG 7, OPEN 2, NON-PROSE 2, ENCODED 1, SCALE-BOUND 1 |
| ref annotation | 55 | 3 | 5.5% | MISSING 30, GAP 22, AMBIG 3 |
| table row | 125 | 1 | 0.8% | MISSING 88, GAP 34, AMBIG 1, NON-PROSE 1, OPEN 1 |
| paragraph (fragment) | 14 | 0 | **0.0%** | **GAP 14** |
| **all** | **450** | **12** | **2.7%** | missing-lexeme 267, grammar-gap 164 |

OOV closure: 176 grounded + injected, 10 named entities, **242 residual**, **240 distinct types**.

**All 14 fragment-paragraphs are grammar-gap, none missing-lexeme** — every token known, no parse.
These are the bold-lead units (`**What I would do:**`), the class the converter already separates
from section headings by the wholly-bold test. They need a decision about whether they are prose,
not vocabulary.

## The ceiling on vocabulary work

Of the 240 residual OOV types, **42 are words**; the rest are data:

| class | types | share |
|---|---:|---:|
| identifier / accession | 126 | 52.5% |
| other (URLs, API fields, `3₁₀`, `ac_hemi`) | 33 | 13.8% |
| hyphenated compound | 19 | 7.9% |
| acronym / tool name (`AlphaFold`, `ClinPred`, `HGMD`) | 17 | 7.1% |
| lowercase word | 15 | 6.2% |
| proper noun (author surnames) | 12 | 5.0% |
| DNA sequence | 10 | 4.2% |
| numeric / coordinate | 8 | 3.3% |

A unit unblocks only when EVERY one of its OOV tokens resolves, so one identifier keeps it blocked.
Attributing the 267 blocked units that way:

| | units | share |
|---|---:|---:|
| blocked by a datum (≥1 identifier, sequence or name) | **207** | 78% |
| closable by vocabulary (every OOV token is a word) | **36** | 13% |
| no OOV type matched — this attribution's tokenisation differs from the kernel's | 24 | 9% |

**So vocabulary work has a ceiling of about 36 units — 8% of the corpus.** Even closing every
lowercase word and hyphenated compound leaves 207 units blocked.

## What actually blocks the document

The tokens blocking the most units are the subject matter:

| units | token | what it is |
|---:|---|---|
| 20 | `Ile380Thr` | the variant the report is about |
| 11 | `p.Ile380Val` | HGVS protein notation |
| 11 | `chrX:123…` | a genomic coordinate |
| 11 | `ABE9` | the base editor |
| 9 | `p.Ala379Thr` | HGVS |
| 8 | `Arg381`, `Ala379` | residue positions |
| 7 | `2KNA` | a PDB entry |
| 6 | `v4.1`, `SIFT4G` | a database version, a predictor |
| 5 | `P98170` | a UniProt accession |

These are not missing words. They are **named entities with internal structure**, in notations with
grammars of their own — HGVS, residue positions, accessions, coordinates. The page's augmentation
already injects named individuals (10 here, and the `cat_np` named-individual route closed unit 4's
gap in step 4), and it is not catching this class: `Ile380Thr` recurs in 20 units and is still OOV.

**Step 2's work is therefore entity recognition, not lexicon addition.** The 36 closable units are
worth taking, but they are not the gap. Recognising HGVS and accession notation would reach the 207.

## Open

- Whether the 14 bold-lead fragment-paragraphs are prose the grammar should accept, or a block kind
  of their own (D101 defers figure/code/caption on the same principle).
- The 24 units this attribution could not match: the kernel's tokenisation is authoritative and
  this note's is a reconstruction. The direction does not depend on them — 207 ≫ 36 either way.

## Why `Ile380Thr` never reaches the named-individual route

Measured `2026-10-11`. The route that closed unit 4's gap in step 4 is documented as "recurrence +
non-adjective head", and `Ile380Thr` recurs in 20 units, so it should qualify. It does not, for
three independent reasons — each sufficient on its own, each pinned in
`kernel/tests/variant_notation_is_not_a_named_entity.rs` with a non-vacuous control
(`project Achilles` IS recognized in the same frame).

1. **The shape rejects it.** `name_token` admits ALL-CAPS (`DRIVE`) or Title-case with **no
   interior capital** — the rule that deliberately rejects `mRNA` and `HeLa`. An HGVS three-letter
   code is exactly that shape: the `T` of `Thr` is an interior capital.
2. **The prefixed form fails earlier still.** `p.Ile380Thr` begins lowercase, so it is rejected
   before the shape test runs.
3. **There is no noun head.** The route is **apposition** — `<common-noun> <Name>`. The real
   contexts, by frequency over the 450 units, are `an`, `for`, `of`, `than`, a table pipe, or a
   VERB (`prefers`, `confirm`, `carrying`). None is a noun head.

**Recurrence is never consulted**: candidacy fails first. So the threshold is not the lever — moving
it changes nothing.

### The parser already exists, unwired

`eigenius-pmi-registry::hgvs::parse_p` reads exactly this notation, and returns its STRUCTURE:

| notation | → | |
|---|---|---|
| `p.Ile380Thr` | start 380, end 380, ref `Ile`, alt `Thr` | |
| `p.Ile380Val`, `p.Ala379Thr`, `p.Arg381Ter` | all parse | |
| `Ile380Thr` (bare, as the report also writes it) | refused: "not a protein (p.) change" | correct for a registry — unprefixed is ambiguous between coding and protein coordinates |

So this is not a capability to build. It is a capability the document glossary does not consult.

### What a notation source would reach

Of the 243 blocked units whose OOV tokens this attribution could match:

| | units |
|---|---:|
| unblocked by notation + vocabulary | **119** (49%) |
| still blocked | 124 |

Notation classes, by units touched: HGVS protein 95, residue position 55, ClinVar VCV 24, genomic
coordinate 11, UniProt accession 10, gene/transcript 8, PDB id 7, dbSNP rsid 6.

**What remains are product names**, not notations: `ABE9` 11, `v4.1` 6, `SIFT4G` 6, `AlphaFold` 5,
`AlphaMissense` 5, `ClinPred` 5, `ABE8e` 5, `MaveDB` 4, `HGMD` 4, `BayesDel` 4, `VEST4` 4, `SiPhy`
4, `SpliceAI` 4 — base editors, predictors and databases. A separate closure, and several would
pass `name_token` already if they occurred in apposition.

`L145T` (4 units) is a **one-letter** HGVS code, which the three-letter pattern above misses; the
report writes ABE9's mutations that way (`N108Q + L145T`). A notation source needs both alphabets.

### The shape this argues for

A **fifth** document-glossary extraction source, keyed on notation rather than orthography —
`named_entity` is the fourth, and `abbrev` (Schwartz-Hearst) is already a separate source precisely
because its pattern is its own. HGVS, accessions and coordinates are published grammars; recognising
them needs no heuristic and no recurrence guard.

It should NOT mint `ni_ile380thr` as an opaque individual. The parse yields reference residue,
position and alternate residue, and an opaque name would put that meaning back in a string — the
ad-hoc-predicate failure (`xiap:HemizygousMaleCount`) that the parser exists to prevent.
