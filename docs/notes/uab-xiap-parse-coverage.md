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

## Re-measured after the importer fixes (2026-10-10)

Same corpus, same flags (`--no-llm --units-per-line`), store
`wordnet-umls-hpo-aligned-2026-10-10-degskip`. `missing-lexeme` is arm-independent —
`encode_unit` returns it before parsing anything — so cap-only gives the real figure.

| | before | after |
|---|---:|---:|
| missing-lexeme | 335 | **334** |
| grammar-gap | 84 | 84 |
| covered (`ENCODED` + `AMBIG`) | 5 (1.2%) | **6 (1.4%)** |
| distinct OOV types | 501 | **495** |
| OOV grounded by the page's augmentation | 147 | **131** |
| residual OOV | 230 | 230 |

Six OOV types closed, one span recovered. The figure to watch is the augmentation's 147 → 131: **16
tokens no longer need the page to invent a grounding for them**, `hemizygous` among them — it was
being rescued per-document before, which is the defect that finding describes, not a fix.

The analysis below predicted TWO types would close (`likewise` from the adverb import,
`hemizygosity` from the admit gate). Six did. The partition under-counted, and the correction does
not change its conclusion: **the corpus's gap is not a vocabulary gap.** 334 of 427 spans still die
at the lexicon, and ~400 of the 495 remaining OOV types are identifiers, accessions, coordinates,
citation fragments and author names that no lexicon should carry. Importing more vocabulary moves
this by single digits; the document model (D96) is what moves it.

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

## Where the vocabulary gap comes from

Each OOV token type was checked against the three source files on disk — WordNet 3.0 `index.*`,
UMLS 2026AA `MRCONSO.RRF` (4.98M English forms), SPECIALIST `LEXICON` (672k base forms and spelling
variants) — and against the store. Partitioned by first applicable cause:

| n | cause | where the fix lives |
|---:|---|---|
| 274 | identifier, coordinate, sequence, citation fragment | not the lexicon |
| 126 | capitalised name (author, tool) | not the lexicon |
| 49 | word-like, no source has it | mostly tool names and API field names |
| 29 | hyphenated, every part known | compound morphology |
| 17 | word-like, SPECIALIST has it | D97 |
| 4 | possessive clitic not split | tokenizer |
| 2 | in UMLS, not in the store | importer |
| 1 | in WordNet, not imported | importer |

**400 of 502 (80%) are identifiers and names.** `VCV000037244`, `chrX:g.123900531A>G`,
`ENSG00000101966`, `ACCCCATTCATATAGCTTCT`, `2019;176`, `Jaganathan`. No lexicon should carry these,
and importing more vocabulary does not reduce the number. They want recognition by form and routing
by document structure, which is D96's territory, not D97's.

**3 of 502 are an importer gap**, and only two are substantive:

- `likewise` is in WordNet and not in the store. WordNet adverbs are not imported at all — D62
  §8.7.5 defers `data.adv`, and 4481 adverb lemmas are absent. This shows up as one token because
  the parser recovers adverbs by rule instead: `is_derived_adverb` takes productive `-ly` forms
  whose adjective base is known, and `is_lexicalized_adverb` holds a hand-written list of
  transitional adverbs. `likewise` is neither `-ly`-derived nor on the list.
- `hemizygosity` is UMLS `C1881036` (NCI, semantic type T080), not in the drop set, and the snapshot
  records `umls_scope: all`. Cause not established.

### The hand-written inventories have paradigm holes

`my` and `whereas` are not in WordNet or UMLS — neither carries determiners or conjunctions — so
they come from `ontologies/lexicon/closed-class.esl`, which holds **149 forms**. The paradigms in it
are partly filled:

| | present | absent |
|---|---|---|
| possessive determiners | `its`, `their` | `my`, `your`, `his`, `her`, `our` |
| subordinators | `if`, `that`, `which` | `whereas`, `while`, `although`, `because`, `when` |
| transitional adverbs (`LEXICALIZED_ADVERBS`) | `however`, `therefore`, `thus`, `also` | `likewise` |

Three hand-maintained inventories stand in for a general lexicon of English function words, and each
has holes of the same shape: a category is present, the paradigm filling it is not.

**SPECIALIST carries every one of them** — `my`, `your`, `whereas`, `while`, `although`, `because`,
`when`, `how`, `likewise`, `however`, `therefore` — and it is already provisioned and already on this
snapshot's provenance (`specialist: references/specialist/LEXICON`), used for adjective and verb
senses but not as a source of forms. Over the whole corpus it covers 57 of the 502 OOV types, of
which 17 are word-like and new: `breakpoint`, `nonsynonymous`, `oligomerization`, `tolerability`,
`multiplexed`, `pulldown`, `homopolymeric`, `unanchored`, `unsequenceable`, the British spellings
`orthologue` and `dimerisation`, and the function words above. The other 40 are surname and
initialism collisions — `Liu`, `Meyer`, `Patel`, `SJ`, `KE` — which SPECIALIST would match for the
wrong reason.

### What was fixed, and what it needed

The analysis above partitions the gap; three of its causes were importer-side and are now closed
(branch `uab-step4-parse-coverage`). All three take effect only on a reseed, and the `am` entry edits
a bootstrap ontology, so the old store is unresumable by content hash — the reseed is required, not
optional.

**Adverbs import at Luo & Shi's `ADV`** (`211cb10`). D62 §8.7.5 deferred `data.adv` for want of a
type for a predicate modifier; eigenius#270 supplied it. Per synset, one axiom at
`(e -> t) -> (e -> t)`, one `lexicon:SemTerm` holding `λV. λs. And(V(s), adv(V, s))`, and one entry
per lemma in each of the two manner positions. The conjunction is in the entry, so modifier drop
stays Luo & Shi's theorem.

The payoff is not coverage — the deferral cost one OOV type, `likewise`, because the parser recovered
adverbs by a derivational rule instead. It is CONTENT: that rule seeds **identity sem**, so the
adverb β-reduces away and the claim is exactly the unmodified one. «p.Ile380Thr is partially exposed»
asserted nothing about *partially*; 39 distinct `-ly` adverbs over 79 of the 427 spans, 90
occurrences. The rule is now the fallback for forms the lexicon does not carry, and discourse adverbs
keep identity because they attach at `S/S` and are transparent there. Its own comment had claimed
WordNet does not store productive `-ly` adverbs; 2975 of WordNet's 4481 adverb lemmas end in `-ly`.

**The closed-class lists are back in step** (`9281dd2`). `dcg::closed_class` (what the importers
withhold) and `ontologies/lexicon/closed-class.esl` (what the bootstrap supplies) are two halves of
one claim and had drifted both ways: 112 supplied forms were missing from the withholding list, 18 of
them reified by UMLS as T078/T080 cruft, and five surfaces were withheld with nothing covering them.

That second direction was live. `then`, `nor` and `any` were unknown to the lexicon in the run above
and rescued per-document by the page's OOV augmentation — a function word grounded as though it were
an unseen domain term. `then` left the withholding list (an adverb, now imported) and `am` was
supplied; `any`, `nor` and `been` are pinned as an asserted set by
`kernel/tests/closed_class_invariant.rs` because each needs a semantic decision rather than a copied
entry: `any` is free-choice/NPI so `exists_sem` is wrong under negation, `nor` denies both conjuncts
while the reserved table offers only and/or, `been` is a participle.

**The non-content filter checks its premise** (`a0bd8dd`). `is_non_content_concept` withholds every
noun entry of a concept typed only T078/T080, justified by the surface staying "known via WordNet /
the closed-class bootstrap" — a premise nothing checked. `AttestedForms` now reads WordNet's
`index.*` and SPECIALIST's `LEXICON` at import and admits a form only where the premise FAILS and
SPECIALIST attests it: 585 single ordinary words of 21494 English atoms, of which SPECIALIST attests
215, `hemizygosity` among them. It cannot reintroduce the compound-pile defect by construction —
every documented case is closed-class (`And`, `Some`, `For (preposition)`) or multiword (`Associated
with`), and the gate only reaches surfaces that currently have no reading at all.

### A measurement bug, found and fixed

`Parser::has_token` re-derived its own lemma list — raw surface, Morphy lemmas, the hyphen rule —
rather than calling `candidate_lemmas`, which the seeder uses. It had drifted: it omitted the
domain-plural fallback, so a UMLS-only plural whose singular is an entry (`indels`, `biomarkers` —
neither singular is a WordNet lemma, so Morphy cannot reduce them) was reported as a missing lexeme
while the parser seeded it. `has_token` now calls `candidate_lemmas`. Worth 4 of 506 OOV types on
this corpus; the headline is unchanged.

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
