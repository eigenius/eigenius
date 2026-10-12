# Parse success and the shape of the gaps — survey §4 step 3

**Measured `2026-10-11`** at `488d690`, store `wordnet-umls-hpo-aligned-2026-10-10-d101`, release,
cap-only (`--no-llm`), `--units-per-line`, over the document model's **325 prose units** (D102
decision 2 routes the 125 `kind_table` units out). Run directories and corpora are gitignored; only
these numbers and the synthetic probes are tracked.

Step 2 took `missing-lexeme` to 0 on the parser's own signal, so every unit reaches the grammar.

## Parse success: 16 of 325 (4.9%)

| outcome | units |
|---|---:|
| encoded (single reading) | 1 |
| ambiguous | 12 |
| open (referent holes, D64) | 3 |
| **produced a reading** | **16** |
| grammar gap | 282 |
| non-prose (unseedable token) | 15 |
| scale-bound (> 60 tokens) | 12 |

2057 readings over 458 skeletons (sense× 4.49). No pins exist for this corpus, so
`expected-hits` is 0 by construction and says nothing.

## Length is the dominant variable

| tokens | units | produced a reading | rate |
|---|---:|---:|---:|
| 1–4 | 20 | 0 | 0% |
| 5–9 | 31 | 7 | 22% |
| 10–14 | 41 | 4 | 9% |
| 15–19 | 41 | 2 | 4% |
| 20–29 | 78 | 2 | 2% |
| 30–39 | 46 | 1 | 2% |
| 40–59 | 54 | 0 | 0% |
| 60+ | 13 | 0 | 0% |

Nothing over 40 tokens parses. The 13 units at 60+ are 12 scale-bound (refused before parsing, above
`PARSE_BUDGET`) plus one non-prose.

## The 282 gaps are established, not truncated

All 282 report `WIDEN [Exhausted]` — the ladder ran every attempt and the final rung carried no
sense cap and no beam, so the chart was built unpruned and returned zero closed and zero open
parses. Of the 282, **7** had items discarded by the Lever-B per-cell beam (3,786,632 items across
them), so those 7 are the only ones whose gap status rests on a truncated chart. No unit that
produced a reading had any cell-beam drop.

## Five buckets

First matching bucket, so the counts partition:

| | | |
|---:|---:|---|
| 148 | 52% | **plain prose** — no embedded structure |
| 45 | 15% | **paired interruption removed** — the lexer strips a `(…)` or `— … —` span before tokens exist |
| 43 | 15% | **clause-position colon** |
| 24 | 8% | **clause-position semicolon** |
| 21 | 7% | **under 5 tokens** |
| 1 | 0% | **wiki-link deleted** |

A clause-position colon or semicolon is one with a word character before and whitespace after, which
excludes an HGVS coding change, a journal volume like `2024;15`, a `chr:pos` coordinate and `https://`. 68 gaps (24%) carry one somewhere; their
median length is 35 tokens against 19 for the rest.

**Markdown markup is not a cause.** 92 gaps contain `**bold**` or `*italic*` and 63 an inline code
span, but the lexer already strips all three, along with `-` bullet markers and `##` headings:
`The **dominant-negative** route is *plausible*` tokenizes to `[The, dominant-negative, route, is,
plausible]`. Reporting those counts as a cause would have been wrong. The one exception is the
wiki-link — `[[Date]]` and `[[Name|display]]` are deleted outright, leaving a dangling comma — which
affects 1 unit.

## Experiment 1 — the clause splice is real, and not dominant

The lexer emits `,` as a non-word token but DROPS `:` and `;`, so a unit containing either reaches
the grammar as two independent clauses spliced with no connective. A/B on the 68 affected gaps:

| | units | produced a reading | grammar gap | non-prose |
|---|---:|---:|---:|---:|
| A — verbatim (control) | 68 | 0 | 67 | 1 |
| B — split at each clause-position `:` / `;` | 145 | 11 | 125 | 9 |

The control reproduces the reference run. Splitting takes 11 pieces from 0 to parsed (8 ambiguous,
3 open) — so the splice does block parses — but **125 of 145 pieces still fail**, and 9 turn out to
be non-prose. Removing the splice is necessary and nowhere near sufficient. The parsing pieces run
4–19 tokens; the failing ones have median 15, so this is not simply the length effect.

## Experiment 2 — "the gap is really a verbless noun phrase" is FALSE

The shortest plain-prose gaps look like headings, captions and annotation labels rather than
sentences, and `Parser::parse` returns only full-span `S` derivations, so a noun phrase is a gap by
construction. Framing each of the 148 plain-prose gaps as `<unit> is relevant.` tests whether its
content composes as an NP — all 148 are gaps unframed, so the control is 0%.

| tokens (framed) | framed rate | corpus baseline at that length |
|---|---:|---:|
| 1–9 | 23% | 22% |
| 10–14 | 8% | 9% |
| 15–19 | 0% | 4% |
| 20–29 | 2% | 2% |
| 30+ | 0% | 0% |

6 of 148 parse, at a rate indistinguishable from the corpus baseline at the same length. **The
category mismatch is not what blocks them** — what blocks them is inside the noun phrase, and the
probe battery below names it. A `kind_heading` / `kind_caption` routing like D102 decision 2's would
be honest bookkeeping and would not recover a single parse.

## Experiment 3 — the construction battery

29 synthetic probes (`probe_step5_construction_families` in the encoding harness), built to isolate
each family the short gaps exhibit, over ordinary vocabulary. **17 parsed, 11 are constructional
gaps, 1 is lexical.** Each failing family is paired with the nearest probe that parses, which is
what localizes it.

The battery runs CHAIN-ONLY and tags a probe whose vocabulary the served lexicon lacks as `LEX-GAP`
rather than `GAP`. That distinction corrected two readings taken from the page, where the
augmentation had supplied the missing word and the unit still failed — so a lexical cause had
presented as a constructional one:

- A passive with an adverb and an agent by-phrase was read as a failure of the AGENT phrase. Its
  adverb (a derived `-ly` form) has no entry; with plain vocabulary the agent phrase parses
  («The question is resolved by the assay.», CLOSED×8). **Retracted.**
- `cannot` was read as a wrong-category entry. It has no entry in the served chain at all.

| | fails | parses |
|---|---|---|
| reduced relative + PP | «The resource queried in the study is reliable.» | «The release queried here is current.» |
| | «The method used in the study is simple.» | |
| participial post-modifier taking an object | «A patient carrying the variant was found.» | — |
| comma appositive | «The study, the first of its kind, reports three results.» | — |
| | «The gene, a known target, is expressed.» | |
| bare negative subject | «Neither is evidence that the variant is tolerated.» | «Nothing in the study contradicts the result.» |
| partitive with a demonstrative | «Both of those are worth having.» | «None of the methods resolves the question.» |
| `rather than` coordinating participles | «The gene was tested rather than assumed.» | — |
| verbless NP as a whole unit | «The source of the results.» | — |
| | «The canonical citation for the method.» | |
| | «Two caveats on reading the table.» | |

What already works, and therefore is not the problem: SVO, copular predication, passive with a
locative PP, with an adverb, and **with an agent by-phrase**; gerund objects (`deserves naming`,
`wants testing`); `worth` + gerund with a plural subject; negative-quantifier subjects
(`No database`, `None of the methods`, `Nothing in the study`); VP ellipsis (`and the second does`);
modals; and `could not` / `did not`.

### `cannot` is a lexical gap, not a grammar gap

`cannot` is one token (the lexer does not split it) with **no entry in the served chain** —
`Parser::unknown_words` reports it unknown. It occurs 7 times on the page, and the run still showed
`missing-lexeme 0`, so the augmentation supplied it: an entry that composes with nothing, which then
scores as a grammar gap. `could not` and `did not` both parse.

It is absent from `closed_class::GRAMMAR_OWNED_ADVERBS`, which holds only `not`, `also` and `too`.
Either the closed-class layer owns it — a reseed, batchable with `while`, `whereas`, `although`,
`because`, `during`, `whether` — or the lexer splits it into `can` + `not`, which needs no reseed.
That is a design decision, not a measurement.

This is also the general warning for the 64 unevidenced entries: a surface the fallback grounds
with an opaque individual clears `missing-lexeme` and then fails the grammar, under a label that
says "construction". `cannot` is the one case measured here; the battery's `LEX-GAP` tag is how the
rest would be caught.

### The comma appositive has no rule

`rules::constructions::appose_group` handles the `<common noun> <Name>` designation («genes MSH2»).
The comma-delimited NP appositive is a different construction and nothing covers it.

## Item 1 landed: the short-passive participial lift

**`pass` had exactly one producer.** `reduced_relative` (`S[dcl,pass]\NP → cat_pp`) fires on nothing
else, and the only route to `pass` was `gq_prep_passive_agent` — the long passive's by-phrase. So
"the deficiency predicted by the model" had a reduced relative and the same construction with the
agent left out did not.

Localized before building anything. The failure is not "adjunct PP", which is what the probe table
above suggested:

| | before |
|---|---|
| «The gene expressed in the liver is current.» | CLOSED×2 |
| «The resource queried in the study is reliable.» | GAP |

`express` has a WordNet PP frame, so `expressed in …` went through
`oblique_participial_lifts`; `query` has none. Confirmed by the frame being
preposition-agnostic — `expressed for …` and `expressed with …` parse too. And
«The resource was queried in the study.» parses finitely, so VP-adjunct attachment was never the
problem.

`short_passive_participial_lifts` adds the second producer: `(S[dcl,pss]\NP)/cat_np` →
`S[dcl,pass]\NP`, with the agent existentially closed by `lexicon:passive_sem` — the same term the
finite short passive (`is`/`was` + unsaturated TV) carries, resolved from the chain rather than
rebuilt, so the two cannot drift apart. Taking the UNSATURATED functor is the guard: lifting the
saturated `pss` is what built relativizer-less subject relatives, and once the object slot is filled
this no longer matches. The leaf emits the shifted `cat_pp` as well, by calling `reduced_relative`
rather than repeating its mapping, because a `UnaryShift` never sees a one-token cell.

Every passive-participial probe now parses — bare, with an adverb, and with an adjunct PP, for verbs
with no PP frame:

| | before | after |
|---|---|---|
| «The resource queried is reliable.» | GAP | CLOSED×2 |
| «The resource queried recently is reliable.» | GAP | CLOSED×2 |
| «The resource queried in the study is reliable.» | GAP | CLOSED×4 |
| «The method used in the study is simple.» | GAP | CLOSED×4 |

### What it cost, and what it bought

The 62-unit CNL baseline page (`baseline.json`'s corpus), cap-only, same store, A/B against the
unmodified tree:

| | before | after |
|---|---:|---:|
| grammar-gap / missing-lexeme | 0 / 0 | 0 / 0 |
| ambiguous / open | 41 / 21 | 41 / 21 |
| expected-hits | 53 | 53 |
| readings | 1258 | 1264 |
| skeletons | 296 | 304 |

Coverage holds, the outcome distribution is identical and no pin moved; the cost is 8 skeletons.

The probe battery goes 17 → **19** of 29 parsed, 11 → **9** constructional gaps.

The XIAP page, 325 prose units:

| | before | after |
|---|---:|---:|
| grammar-gap | 282 | **281** |
| open | 3 | 4 |
| readings | 2057 | 2440 |
| skeletons | 458 | 467 |

**One unit.** That is the measurement that matters for planning: closing a construction converts two
probes and one page unit, because a 20-to-40-token sentence fails for several reasons at once and
fixing one of them leaves it failing. The unit that moved went to OPEN — it parses, awaiting
referent resolution — not to encoded.

The agentive by-phrase probe went CLOSED×4 → CLOSED×8. That is genuine ambiguity, not a
re-derivation: `for the study` gives 4 and `by the study` 8, and the extra four are the agentive
analysis, whose semantics differs from the agentless one plus an adjunct (agent bound vs.
existentially closed). The same `by` ambiguity English has in "written by candlelight".

## The second battery — what the 10-16 token band shows

The first battery was built from the 5-9 token gaps and never reached the constructions that
dominate the rest. 32 more probes (`probe_step5_families_medium`): **8 parse, 21 are constructional
gaps, 3 are lexical.**

| | fails | parses |
|---|---|---|
| **`rather than`, EVERY position** | «The study measured the result rather than the control.» (NP) | — |
| | «The result is clear rather than ambiguous.» (ADJ) | |
| | «The study measured rather than assumed the result.» (V) | |
| | «The gene is expressed in the liver rather than in the kidney.» (PP) | |
| zero object relative | «The resource I could query is reliable.» | «The resource that I could query is reliable.» |
| zero complementizer | «The study mentioned the result was clear.» | «The study mentioned that the result was clear.» |
| VP coordination under an auxiliary | «The result has been settled and is recorded below.» | — |
| noun comparative | «The study showed greater dependence than the control.» | «The result is clearer than the control.» |
| | | «The result is more conservative than the control.» |
| object-predicative comparative | «The predictors rate the variant worse than the control.» | «The predictors rate the variant damaging.» |
| free relative + stranding | «Absence is what a neutral variant looks like.» | — |
| `both … and` over VPs | «The result both bounds the argument and reinforces the conclusion.» | — |
| `without` + gerund | «The study searched without finding the variant.» | — |
| imperative | «Report the result.» / «Do not report the result.» | — |
| fronted conditional | «If the assay fails, the result is void.» | — |
| sentence-initial `So` | «So this is the complete set.» | «Yet the predictors rate the variant worse.» |
| parenthetical adverb | «It is therefore, empirically, compatible with the result.» | — |
| unlike-category coordination | «It is the same residue, carried by four men, and benign.» | — |
| absolute participial | — | «The study reports the result, having built the assay.» |

**Imperatives are a declared deferral, not an oversight.** `lexicon:Mood` has `imp`, and the
ontology states that ⟦S[imp,_]⟧ is deferred to D63 Slice 5, where "⟦·⟧ fails closed rather than
denoting `Prop`". Only 3 of the 281 gaps open with an imperative verb, and 3 with a subordinator, so
on this page both are small.

Three LEXICAL gaps surfaced alongside, all closed-class omissions —
`is_closed_class_surface` owns none of `where`, `itself`/`himself`/`themselves`, `rather`, `so`,
`neither`, `what`, `unless`, though it does own `both`, `if` and `without`.

### Sized against the 281

Incidence of the battery-proven families (overlapping):

| | | |
|---:|---:|---|
| 41 | 14% | **`rather than`** |
| 17 | 6% | `where` / `unless` / `neither` |
| 12 | 4% | free relative `what` |
| 11 | 3% | comparative `than` (excluding `rather than`) |
| 6 | 2% | `both … and` |
| 6 | 2% | `cannot` |
| 4 | 1% | sentence-initial `So` |
| 4 | 1% | reflexive `-self` / `-selves` |
| 2 | 0% | `without` + gerund |
| 2 | 0% | parenthetical `, …ly,` |
| **92** | **32%** | carry at least one |

Incidence is not cause — a unit carrying `rather than` fails for its other constructions too, which
is why item 1 converted one unit, not forty. These numbers rank the families; they do not predict
how many units each closes.

## Order of work

By units unblocked per unit of work, on these measurements:

1. ~~The reduced relative + PP~~ — **done**, above. Measured: 1 page unit.
2. **`rather than`.** Not in the grammar in ANY position — NP, adjective, verb or PP — and in 41 of
   the 281 gaps, the largest proven family by a factor of two.
3. **The participial post-modifier with an object** (`carrying the variant`). The passive half is
   now built; this is the active `ger` half, and the intransitive and adjunct-PP forms already parse
   («A patient waiting in the study was found.»).
4. **The zero relative and the zero complementizer.** Both `that`-ful forms parse, so this is one
   omission in two places rather than two constructions.
5. **VP coordination under an auxiliary** («has been settled and is recorded»).
6. **The closed-class omissions** — `where`, `rather`, `so`, `neither`, `what`, `unless`, the
   reflexives, `cannot` — batched into one reseed. 17 + 12 + 6 + 4 + 4 gaps carry one.
7. **The comma appositive**, the noun comparative, the free relative, `both … and`, the
   demonstrative partitive, the parenthetical adverb, unlike-category coordination.

**But the measured yield is ~1 unit per construction**, so the order above closes the gate slowly if
at all. The leverage is step 4: the clause splice alone recovers 11 of 145 pieces, and a rewrite
addresses length — the dominant variable — directly, where a grammar rule does not.

Nothing above the lexicon is in doubt: `missing-lexeme` is 0 over the corpus, and the one lexical
gap the battery found (`cannot`) is a closed-class omission rather than a content word.
