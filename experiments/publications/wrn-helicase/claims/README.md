# The WRN study's claims in controlled English

The hand-built chain states the paper's results with opaque predicates
(`onco:SelectiveViabilityDependence("WRN", "MSI")`). [`../docs/06-opaque-predicates-to-propositions.md`](../docs/06-opaque-predicates-to-propositions.md)
locates the sentence that states each one. This corpus renders those sentences in the controlled
register ([`docs/method/controlled-english-style-guide.md`](../../../../docs/method/controlled-english-style-guide.md)),
pairs each with the proposition it should mean, and records what the parser makes of it over the
full lexicon.

[`corpus.tsv`](corpus.tsv): 62 rows, in the paper's order — the 27 predicates a conclusion uses, the
paper's definition of synthetic lethality, one literature claim, and the sentences they rest on:
the introductions of the reagent names and markers, two premises, and the context that defines the
MSI-predominant lineages.

| Column | |
|---|---|
| predicate | the chain predicate the sentence states, or what the sentence is for |
| sentence | the controlled-English rendering |
| source | the locator in `data/slices/PMC6580861.txt`, the PMC author manuscript |
| intended | the proposition the sentence should mean |
| outcome | over the full lexicon, cap-only: the harness outcome, and why no reading is the claim where none is |
| skeleton | the sense-erased skeleton of the reading that is the claim, reviewed against its gloss; `-` where none is |

## How the sentences follow the style guide

- **R1**, one claim per sentence: a contrast (`in MSI but not MSS cells`) is two rows, and a claim
  embedded in a noun phrase gets its own sentence (the organoid's origin).
- **R2**, nothing that changes the claim is dropped: the paper's hedges (`suggest`, `argue`), its
  qualifiers (`selectively`, `substantially`, `independently`, `negligible`, `significantly`) and its
  scope words (`the four other`, `all`, `in vitro and in vivo`, `MSI-predominant lineages`) are kept,
  including where the parse then fails or drops them. Where the paper states an observation
  (`Exonuclease inactivation did not attenuate the rescue`), the row states that, not what it entails.
- **Statistics stay out**: `0.56-fold`, P values and n.
- **One form per term**: `double-strand breaks` throughout, not `DSBs`.
- **Full noun phrases, not demonstratives, without an antecedent**: `The rescue data argue…`, not
  `These data argue…`. `these two events` stays: the row before it is its antecedent.
- **Names are introduced in their own sentence** (`sgWRN-EIJ is an sgRNA that targets an exon-intron
  junction of WRN.`).
- **Rephrased where the guide says to**: a genitive relative (`no gene whose loss…`) becomes `We found
  that the loss of no gene could account for…`, keeping the paper's modal.

## Measured

`2026-09-27`, cap-only, at `bb31a53` on `wordnet-umls-aligned-2026-09-27` (the reseed at `6a3eabf`;
the parser has not changed since):

| | Rows |
|---|---|
| a reading is the claim; its skeleton pinned | 25 |
| a qualifier is dropped, so no reading is the claim | 14 |
| no reading is the claim, for another reason | 6 |
| a missing lexeme | 13 |
| a grammar gap | 2 |
| open, a demonstrative awaiting its antecedent | 1 |
| an existing pin this cap-only run misses | 1 |

- **Qualifiers are dropped.** `selectively` (4 rows), `preferentially` (2), `independently` (2),
  `most commonly`, `favorably`, `solely`, `merely`, `partially` and `fully` are transparent adverbs
  (D62), so the proposition loses them. Four rows turn into a different claim: `…not solely
  responsible for WRN dependence` reads as `…not responsible…` and `…alone does not fully explain…` as
  `…does not explain…`, both contradicting the paper's `contributes to`; `not merely a consequence of
  cell death` reads as `not a consequence`; `partially rescued` as a full rescue. R2 forbids dropping
  a qualifier to make a sentence parse; here the grammar drops it silently.
- **Introductions do not ground names** (13 rows). `sgWRN-EIJ is an sgRNA that…` leaves `sgWRN-EIJ`
  unknown, and so for `sgWRN2`, `shWRN1`, `shWRN1-C911` and `γH2AX`. The named-entity extractor takes
  only a Title-case or all-caps name after a noun that recurs; the abbreviation extractor only the
  paper's own `an sgRNA targeting a WRN exon-intron junction (sgWRN-EIJ)`, which grounds `sgWRN-EIJ` as
  a count noun but cannot ground `shWRN1` (its `1` is not in the long form). The style guide tells the
  author to introduce a name in its own sentence, and nothing in the pipeline reads that sentence.
- **Faithful scope words break parses**:
  - `in all MSI cell lines` reads distributively (for every line, the dependency correlates), where a
    correlation is over the population;
  - `MSI cell lines from MSI-predominant lineages with intact p53 were more sensitive … than MSI cell
    lines from MSI-predominant lineages with impaired p53`: the standard's modifiers attach outside
    the comparison in all 12 readings, where the version without the scope words has the clean
    comparison.
- **Other constructions with no reading that is the claim**: the relative adverb `where` (read as a
  LOINC noun `Where performed`); `We found that the loss of no gene could account for…`, where `We
  found` reads as a noun and `no gene` scopes inside `the loss of`; `the synthetic lethal interaction
  between WRN and MSI`, whose `WRN and MSI` coordinates at clause level.
- **The cap-only run hides a verb sense.** In `WRN silencing increased phospho-ATM foci`, `increased`
  seeds its adjective and `cat_measure` entries first and the transitive verb is its second sense; a
  junk reading parses, so the sense cap never widens. `substantially increased` also reads as the
  UMLS qualifier `Substantially Increased`. The reranked run ranks senses in context first.
- **Grammar gaps**: `greater colocalization … than`, where `greater dependence on WRN than` parses —
  SPECIALIST lists `colocalization` as `uncount` and Wiktionary does not, so D97's decision 4 gives it
  the mass reading; and `expose X as Y`, whose `as` complement neither WordNet nor SPECIALIST attests.
- **Senses are not judged here.** The skeletons erase senses, and the cap-only run has no ranker:
  `WRN` often verbalises as Werner syndrome and `MSS` as the SIL1 gene. The pins fix structure.
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
- Qualifier semantics for the adverbs above — a grammar question, and the largest one here.
- A way for a document's introduction of a name to ground it, which the style guide already
  assumes.
