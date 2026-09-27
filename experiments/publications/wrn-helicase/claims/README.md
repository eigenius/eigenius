# The WRN study's claims in controlled English

The hand-built chain states the paper's results with opaque predicates
(`onco:SelectiveViabilityDependence("WRN", "MSI")`). [`../docs/06-opaque-predicates-to-propositions.md`](../docs/06-opaque-predicates-to-propositions.md)
locates the sentence that states each one. This corpus renders those sentences in the controlled
register the parser covers, pairs each with the proposition it should mean, and records what the
parser makes of it over the full lexicon.

[`corpus.tsv`](corpus.tsv), one row per claim — 50 rows over the 27 predicates a conclusion uses,
the paper's definition of synthetic lethality, and one literature claim:

| Column | |
|---|---|
| predicate | the chain predicate the sentence states |
| sentence | the controlled-English rendering: one claim per sentence, hedges and qualifiers kept (style guide R1, R2) |
| source | the locator in `data/slices/PMC6580861.txt`, the PMC author manuscript |
| intended | the proposition the sentence should mean |
| outcome | over the full lexicon, cap-only: the harness outcome, and why no reading is the claim where none is |
| skeleton | the sense-erased skeleton of the reading that is the claim, reviewed against its gloss; `-` where none is |

## Measured

`2026-09-27`, cap-only, at `9f45319` on `wordnet-umls-aligned-2026-09-27` (the reseed at `6a3eabf`; the parser has not changed since):

| | Rows |
|---|---|
| a reading is the claim; its skeleton pinned | 22 |
| parses, but a qualifier is dropped | 10 |
| parses, but no reading is the claim for another reason | 4 |
| a missing lexeme | 8 |
| a grammar gap | 2 |
| open, a demonstrative awaiting its antecedent | 3 |
| an existing pin this cap-only run misses | 1 |

- **Qualifiers are dropped.** `selectively` (4 rows), `preferentially` (2), `solely`, `merely`,
  `partially` and `fully` are transparent adverbs (D62), so the proposition loses them. In four rows
  that changes the claim: `p53 activity is not solely responsible for WRN dependence` reads as `…is
  not responsible…`, and `MMR deficiency alone does not fully explain WRN dependency` as `…does not
  explain…` — both contradicting the paper's `contributes to`; `not merely a consequence of cell
  death` reads as `not a consequence`; `partially rescued` as a full rescue. Style guide R2 forbids
  dropping a qualifier to make a sentence parse; here the grammar drops it silently.
- **The cap-only run hides a verb sense.** In `WRN silencing increased 53BP1 foci`, `increased`
  seeds its adjective and `cat_measure` entries first and the transitive verb is its second sense.
  A junk reading parses, so the parser never widens the sense cap. The reranked run ranks senses in
  context first; these two rows wait for it.
- **Missing lexemes are the paper's names**: the reagents `sgWRN-EIJ` (3 rows), `shWRN1`,
  `shWRN1-C911`, the marker `γH2AX` (2), and the hyphenated measure `0.56-fold`. The reagent names
  are introduced by the paper itself; the page here has no such sentences.
- **Grammar gaps**: the genitive relative `whose`, and `greater colocalization … than`, where
  `greater dependence on WRN than` parses. SPECIALIST lists `colocalization` as `uncount` and
  Wiktionary does not, so D97's decision 4 gives it the mass reading.
- **Rewritten**, because the first wording has no parse and the rewrite says the same:
  `DSBs` → `double-strand breaks` (`DSBs` fails as a subject); `chromosomes 3 and 5` →
  `chromosome 3 and chromosome 5` (a noun with coordinated numerals); `as an exonuclease and as a
  helicase` → `as an exonuclease and a helicase` (a repeated `as`), which is also closer to the
  paper's `as both … exonuclease and … helicase`.
- **Senses are not judged here.** The skeletons erase senses, and the cap-only run has no ranker:
  `WRN` often verbalises as Werner syndrome and `MSS` as the SIL1 gene. The pins fix structure; the
  sense a claim needs is the reranker's and the selections' to fix.
- **Prepositions and meaning.** `contributes to` and `correlated with` parse, but the preposition
  does not reach the proposition (D97 decision 6).

## Reproducing

```bash
grep -v '^#' experiments/publications/wrn-helicase/claims/corpus.tsv | cut -f2 > /tmp/wrn-claims.txt
EIGENIUS_GLOSS_READINGS=1 EIGENIUS_GLOSS_MAX=200 \
  scripts/measure-parse-rate.sh --no-llm --page /tmp/wrn-claims.txt
```

`EIGENIUS_GLOSS_READINGS` prints each unit's distinct skeletons with a gloss; the pins are those
skeletons, verbatim. The parse-rate harness gates the pins in
`experiments/parsing/expected-readings.tsv` only; this corpus's are not gated yet.

## Next

- The reranked run, once the sense ranks are re-recorded for the reseed, and the pins re-checked
  under it.
- The paper's own name introductions (`sgWRN-EIJ` is an sgRNA that targets a WRN exon–intron
  junction), so the reagent claims parse.
- Qualifier semantics for the adverbs above: a question for the grammar, not for this corpus.
