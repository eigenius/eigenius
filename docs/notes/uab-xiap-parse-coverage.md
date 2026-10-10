# Parse coverage over the XIAP certification log — survey §4 step 4

**Measured 2026-10-09** at `a83ac28`, snapshot `wordnet-umls-hpo-aligned-2026-10-07-adv`, release,
cap-only (`--no-llm`), `--units-per-line`. Run
`experiments/parsing/results/2026-10-09-1805-a83ac28-claims-page-caponly-perline/` (gitignored, as is
the corpus — the log is gitignored material; only this note's numbers are tracked).

Corpus: the 427 live claim spans of the certification log, verbatim, one per unit
(`experiments/parsing/uab-xiap-corpus.py`), scored per kind by
`experiments/parsing/uab-xiap-score.py`.

Coverage counts a span whose parse produced at least one reading — `ENCODED` or `AMBIG`. Choosing
among readings is the selection stage, which a cap-only run does not reach.

## The result

**5 of 427 spans are covered (1.2%). `encoded` is 0: not one span reached a unique reading.**

| | n | covered | rate | outcomes |
|---|---:|---:|---:|---|
| prose | 281 | 5 | 1.8% | missing-lexeme 220, grammar-gap 56, ambiguous 5 |
| fragments | 146 | 0 | 0.0% | missing-lexeme 115, grammar-gap 28, non-prose 2, open 1 |
| **all** | **427** | **5** | **1.2%** | missing-lexeme 335, grammar-gap 84, ambiguous 5, non-prose 2, open 1 |

Per kind, over prose spans:

| kind | n | covered | rate |
|---|---:|---:|---:|
| observed | 145 | 1 | 0.7% |
| declared | 57 | 2 | 3.5% |
| marked | 42 | 1 | 2.4% |
| derived | 27 | 1 | 3.7% |
| exempt | 10 | 0 | 0.0% |

**The kind does not predict the outcome.** It is a property of a claim's warrant, not of its
sentence, so this is the expected result and it is worth stating: a per-kind coverage target is not
a thing to steer by.

By section, which does carry information:

| | n | covered | missing-lexeme | grammar-gap |
|---|---:|---:|---:|---:|
| body | 218 | 5 (2.3%) | 144 | 66 |
| tables | 126 | 0 | 109 | 17 |
| §References | 83 | 0 | 82 | 1 |

Excluding §References changes 1.2% to 1.5%. The bibliography is not what is holding the number down.

## The gap is lexical, not grammatical

**335 of 427 spans (78%) never reach the grammar.** The page's own OOV closure grounded 147 tokens
and injected 15 named entities; **230 residual OOV** remained, over **501 distinct OOV token
types**:

| n | bucket |
|---:|---|
| 126 | surnames |
| 67 | journal citation fragments (`2019;176`, `e28511`) |
| 37 | HGVS / genomic coordinates (`p.Ile380Thr`, `chrX:g.123900531A>G`) |
| 36 | residues and positions (`Ala379`, `Val376Ala`) |
| 36 | tool and resource names (`AlphaMissense`, `SpliceAI`, `dbNSFP`) |
| 34 | accessions (`ENSG00000101966`, `VCV000037244`, `P98170`) |
| 15 | DNA/protein sequence |
| 13 | URL and API fragments |
| 12 | initials |
| 125 | everything else |

**205 of 501 (41%) are bibliography** — surnames, initials, journal-volume fragments. D96's first
gap, "references read as content", measured.

### Ordinary English is missing too

Three OOV tokens are not domain vocabulary at all: **`my`, `likewise`, `whereas` each return 0
entries** from the full WordNet+UMLS lexicon (`probe_form_entries`; `screens` returns 39 in the same
probe, so the lemmatizer is working and `missing-lexeme` is not inflated by plurals). `my` is a
possessive determiner, `whereas` a subordinating conjunction, `likewise` an adverb WordNet carries.

Another 28 lowercase single words and 18 lowercase hyphenated compounds are OOV, most of them
productive morphology over stems the lexicon has: `nonsynonymous`, `oligomerization`, `dimerisation`,
`homodimers`, `hemizygosity`, `tolerability`, `orthologue`, `multiplexed`, `indels`, `in-frame`,
`side-chain`, `single-submitter`, `re-run`, `splice-site`.

This is D97's territory, and it says the specialist lexicon is not the whole answer: a domain lexicon
would not supply `whereas`.

## The instrument had to change first

The harness infers units from punctuation. On these 427 spans `segment_sentences` returns **653
units** — 90 spans split, 5 pairs merged. Each span carries an id and a kind, so after
re-segmentation no outcome can be attributed to the claim that produced it and the per-kind
measurement is not available at all.

`segment_given_lines` (`kernel/src/dcg/segment.rs`) takes the units as given, one per non-blank line;
the sweep chooses on `EIGENIUS_WRN_UNITS_PER_LINE` / `--units-per-line`, recorded in the provenance
header and the run id. The default is unchanged.

The first attempt estimated the splits in the corpus generator and mapped units back by cumulative
count — a reimplementation of the segmenter in another language, wrong by 6x on its first
measurement (30 predicted extra units against 226 actual). A corpus from a register, a claim log, a
table or a LIMS export arrives already segmented; the harness was missing the ability to say so.

## What this says about step 5

Survey §4 step 5 is "design the document model (depends on 4's evidence)". The evidence:

1. **Units are given, not inferred** — handled above, and the same requirement one layer below the
   span-location one D96 already records.
2. **34% of the corpus is not prose.** 146 spans have no sentence-final punctuation: 88 are table
   cells, 14 are section headings (every one of them `kind: compressed`), 44 are other fragments —
   datelines, labelled list items. 126 spans in all sit in a table location. D96 notes that the
   encoder hard-codes every unit to `kind_prose` and that `kind_table` "conflates two things"; here
   the two things are headings and rows, and they want different treatment — a heading is a claim
   about what the rows establish (D100 rung 3), a row is a record.
3. **§References is 19% of the corpus and 41% of the lexical gap.** It is a citation list, and it
   should be routed to `reference:Reference` rather than parsed.

## What was not measured

Reading correctness. With `encoded` 0 and 5 ambiguous spans, there is nothing for the selection
stage to be right or wrong about. That measurement waits on the lexical gap.
