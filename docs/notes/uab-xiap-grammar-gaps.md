# Parse success and the shape of the gaps — survey §4 step 3

**Measured `2026-10-11`**, store `wordnet-umls-hpo-aligned-2026-10-10-d101`, release,
cap-only (`--no-llm`), `--units-per-line`, over the document model's **325 prose units** (D102
decision 2 routes the 125 `kind_table` units out). Run directories and corpora are gitignored; only
these numbers and the synthetic probes are tracked.

Step 2 took `missing-lexeme` to 0 on the parser's own signal, so every unit reaches the grammar.

## Parse success: 16 of 325 (4.9%) — the step-3 baseline

Superseded by the five items below; **19 of 325 (5.85%)** as of 2026-10-14. The table is kept as the
measurement every later delta is taken against.

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

## What core-en has, and what it does not

The kernel ports rule SHAPES from core-en (`references/openccg/grammars/core-en`) — `bnp`, the
`conj.xsl` list-with-operator model, `rrel`, `adv.xsl`'s Transitional-Adverb, `auxv.xsl`'s Negation
family. It is worth knowing which of the measured gaps have a reference there. core-en is **85
dictionary entries and 54 families**, a core grammar rather than a coverage target.

| gap | core-en |
|---|---|
| zero object relative | **YES** — the `rrel` type-changing rule, `s[dcl]/np → n\n` with a `GenRel` relation. This is the half `reduced_relative`'s comment says was NOT taken: "we take the SUBJECT-gap (backward) form rather than its object-gap (forward) one, because that is the participial case". The forward form is the zero object relative, and it is specified. |
| bare quantifier subject | **YES** — `both`, `neither`, `none` are `QNP`, family `QuantNP`. `Nothing in the study …` and `None of the methods …` already parse here; `Neither is evidence …` and `Both of those …` do not. |
| `where` | **YES** — `WhNP`, alongside `what`. (A free relative is a different use of `what`; core-en's is the wh one.) |
| fronted adverbial | **PARTIAL** — `form-vals` has `fronted` beside `dcl`, and there is a topicalization rule `tpc`. |
| `but` | **PARTIAL** — `Conj-Sentential-Binary` (S, and S-VP) only, with an HLDS `Arg1`/`Arg2` LF. The polymorphic `but not` that `but_not_coord` builds, recursing on the denotation to `And(a, ¬b)`, is Eigenius's own and goes beyond core-en. |
| **`rather than`** | **NO.** There is no `rather` stem, no `instead`, no `without`. Nothing to port — the semantics has to be argued, and the argument is that it is `but not` (above). |
| comparative `than` | **NO.** No `than` stem and no comparative/superlative machinery at all. `cat_pp_than`, `elided_than` and the adjective comparatives are entirely Eigenius's. |
| imperative | **NO.** `form-vals` is `dcl`/`fronted`, `q`/`wh`, `base`, `emb`, `inf`, `adj`, `ng`. So D63 Slice 5's deferral matches core-en's own scope rather than departing from it. |

## Item 2 landed: `rather than`

`X rather than Y` is `X ∧ ¬Y` — `but not`'s truth condition — so this needed a TRIGGER and no
semantics. `build_coord_excluding` (the renamed `build_but_not`) is reused verbatim; what differs
between the two is the correction `rather than` conveys, and that is not truth-conditional.

Declared as a PAIR rather than two kinds. `but not` is two tokens each reserved in their own right;
neither half of `rather than` carries the meaning alone — `rather` is a degree adverb (`rather
clear`), `than` the comparative marker (`larger than`). So `lexicon:construct_form` now admits a
multi-token form and `ReservedTable` gained a `by_phrase` map with longest-match `phrase_at`. Both
halves stay unreserved, which a test asserts.

It is ONE operator with two triggers, and the probe counts say so — identical in every position:

| | `rather than` | `but not` |
|---|---|---|
| ADJ vs ADJ | CLOSED×8 | CLOSED×8 |
| V vs V | CLOSED×8 | CLOSED×8 |
| PP vs PP | CLOSED×4 | CLOSED×4 |
| NP vs NP | GAP | GAP |

The NP case fails for both, and for plain `and` in subject position — the pre-existing coordination
gap, which this does not reach.

### Measured, on a store reseeded after the `rather than` edit

A bootstrap edit, so the store was rebuilt. All three provenance counts reproduced exactly —
38,391 merges, 20,533 HPO resources, 864 equivalences — and `umls_scope: all`, UMLS 2026AA and the
three input sha256 prefixes match the previous store.

| | before | after |
|---|---:|---:|
| **CNL baseline, 62 units** | | |
| grammar-gap / missing-lexeme | 0 / 0 | 0 / 0 |
| readings / skeletons | 1264 / 304 | 1264 / 304 |
| expected-hits | 53 | 53 |
| **construction battery, 39 probes** | 12 parsed, 24 gaps | **15 parsed, 21 gaps** |
| **XIAP page, 325 units** | | |
| grammar-gap | 281 | **280** |
| open | 4 | 5 |
| readings | 2440 | 2440 |
| skeletons | 467 | 491 |

The baseline page is unchanged to the figure, which is two results at once: the reseed is faithful
in EFFECT and not only in provenance, and the new construct perturbs nothing where it does not
occur.

**One page unit again.** It moved GAP → OPEN, and `total_readings` counts closed parses only, so it
contributes 0 readings and 24 skeletons — hence readings flat at 2440. Two constructions have now
been closed and each converted exactly one unit, against families of 41 and comparable incidence.
That is the number to plan with: incidence ranks the families, it does not predict conversions,
because a 20-to-40-token sentence fails for several reasons at once.

## Item 3: the closed-class batch — zero units, and why that is the result that matters

44 entries over three bootstrap edits: 20 possessive determiners (`my`/`your`/`our`/`his`/`her` ×
sg|pl × subj|obj), `unless` and `whereas`, and 22 indefinite pronouns. Measured on a store reseeded
after the indefinite-pronoun edit, all three provenance counts reproduced.

| | before | after |
|---|---:|---:|
| **CNL baseline, 62 units** | 1264 / 304, hits 53 | **identical**, coverage PASS |
| **XIAP page** grammar-gap | 280 | 281 |
| ambiguous / open / encoded | 12 / 5 / 1 | 11 / 5 / 1 |
| readings / skeletons | 2440 / 491 | 2303 / 489 |
| unevidenced fallback | 64 | **60** |

**Zero units converted, and one lost — correctly.** Exactly one unit changed, and diffing the
outcome SETS rather than the totals identified it immediately: a 29-token unit that is really TWO
sentences (a bold verbless fragment followed by a sentence, a converter segmentation artifact). It
had carried `Your`, which the UNEVIDENCED FALLBACK had minted as an opaque doc-local individual — a
plain NP — and that NP was what let the parser find an analysis spanning both sentences, via
`WIDEN [Ranked] attempts=4 cap=Some(16)`. With a real possessive determiner there the spurious
analysis is gone and the unit fails, which is what it should do.

So the gap count went UP by one while the grammar got more correct. This is the warning recorded
with the 64 unevidenced entries, now measured: an opaque individual does not merely fail to help —
it can MANUFACTURE a parse. Four surfaces left the fallback here (`your`, `my`, `our` and one more),
which is the provenance gain the batch actually delivered.

### What works now, and what does not

| paradigm | before | after |
|---|---|---|
| possessive determiners | 1 of 7 | **6 of 7** — `their` is the pre-existing one-number gap |
| subordinating conjunctions | 3 of 14 | **5 of 14** — `unless`, `whereas` |
| indefinite pronouns | 1 of 11 OK, 7 absent | **0 absent**, still 1 OK |

**The indefinites are entries without an analysis, and the probe ladder says exactly why:**

| | |
|---|---|
| «Something is clear.» | parses |
| «Something resolves the question.» | parses |
| «Something **in the study** is clear.» | GAP |

A post-modifier blocks them. The entries are type-raised subjects `S/(S\NP)`, which cannot take a
PP because they are not NPs; in a determiner analysis the modifier belongs on the silent noun,
"some [thing in the study]". The corpus uses modified indefinites almost exclusively — "Nothing
here", "Everything in Findings 1–7", "something specific to say", "anything I could not check" — so
these will fire on few real occurrences until the modifier route exists. `nothing` still reports OK
only because WordNet's NOUN entry takes the PP as a plain NP, which is the wrong reading the new
entry does not displace.

`cannot` was NOT added: the split-vs-reseed decision named in the order of work was never taken, and
it is 6 gaps.

### Four fixes, four measurements

| fix | incidence | units converted |
|---|---:|---:|
| short-passive participial lift | — | 1 |
| `rather than` | 41 gaps | 1 |
| closed-class batch | 51 gaps, 44 sole-blocker | **0** |
| active `ger` participial lift | — | **2** |

The closed-class batch was chosen precisely because it was a different KIND of fix with the best
sole-blocker count, and it converted fewer units than the two constructions. Grammar work on this
corpus does not close the gate: the units fail for several reasons at once, and the measured yield
per fix is at best one. Step 4's rewrite addresses length, which the step-3 measurement identified
as the dominant variable, and the clause-splice A/B already showed it recovers 11 pieces where a
construction recovers one.

## Item 4: Fork A — zero units, and the measurement that explains all four

The factive subordinators (`because`/`although`/`while`/`though`/`whilst`), `cannot` split in the
lexer, `its genes`/`their gene`, and `enc:ResolutionMethod` on the gap stream. Store reseeded after
the `cannot` split, provenance reproduced for the fourth consecutive time.

| | before | after |
|---|---:|---:|
| **CNL baseline, 62 units** | 1264 / 304, hits 53 | **identical**, coverage PASS |
| **XIAP page** — every outcome | 1 / 11 / 5, gap 281 | **unchanged** |
| readings / skeletons | 2303 / 489 | 2303 / 521 |

**Zero units. Not one outcome changed.** The baseline holding identical is worth noting on its own:
this batch added an axiom family, a hole kind, an arm to the felicity gate's traversal, and a
TOKENIZER change — and moved no figure on the gated page.

Subordinating conjunctions went 3 of 14 → **10 of 14** (the four left are `since`/`until`/`when`/
`once`, temporal with no account). So the construction works; it converts nothing.

### Why: a binary connective converts at the SQUARE of the clause rate

Splitting the 18 gap units that carry a factive subordinator at the subordinator gives 37 clause
halves. Measured on the same store:

| | |
|---|---:|
| halves that parse alone | **8 of 37 (22%)** |
| halves that are still gaps | 29 |

A subordinator needs BOTH sides. At a 22% clause rate the unit rate is `p² ≈ 4.7%`, so over 18
units the expected yield is **0.84 conversions** — and zero is exactly what that predicts. Fork A
did not underperform; the base clause rate makes binary constructions nearly unconvertible.

The same lens explains the earlier results rather than being fitted to them: `rather than` is also
binary, over 41 gaps, and returned 1.

**This is the quantitative case for step 4.** A rewrite that splits a unit into independent clauses
converts a `p²` problem into a `p` problem — that is the whole of its advantage, and it is why the
clause-splice A/B recovered 11 pieces where every construction fix recovered one or none. Closing
constructions raises `p` slowly; splitting changes the exponent.

## Item 5 landed: the active `ger` participial lift

**The gap count understated this one, and the sems say why.** Three probes appeared to parse. What
they parsed as (2026-10-14, pre-change):

```text
"The gene encoding a helicase is essential."     CLOSED×3
  gt(deg_a01580306(the(ΣG#0:C0920283. compound_kind(G#0,
    ΣG#1:C0439234. compound_kind(G#1, ΣG#2:C2700640. compound_kind(G#2, n05436752)))).1), std_a01580306)
"A patient waiting was found."                   CLOSED×2
  ΠG#0:Prop. ΠG#1:C0542399. gt(deg_a01450713(G#1), std_a01450713) → G#0 → G#0
```

The first is a three-deep COMPOUND NOUN PILE — gene-encoding-helicase. The second resolved
`waiting` as an ADJECTIVE degree. Neither is a post-modifier reading, so the construction had no
route at any verb, and the earlier claim here that "the intransitive and adjunct-PP forms already
parse" was wrong.

### The fix is the active mirror of item 1

`reduced_relative` maps a subject-gapped VP to `cat_pp` with the sem unchanged — both denote
`Entity → Prop`. It fired on `fin = pass` only; it now fires on `fin ∈ {pass, ger}`. What differs
between the two is which slot the modified noun fills: for `pass` the PATIENT, for `ger` the AGENT,
and English licenses both ("the DNA induced by WRN", "the man eating the sandwich"). The argument
that keeps `pss` out does not reach `ger`: `pss` saturated is a finite-LOOKING active VP, so shifting
it builds "*the man ate the food" for "the man that ate", while `carrying` has no finite homograph.

Work splits by span length. The TRANSITIVE reaches the shift unaided — `carrying` is
`(S[dcl,ger]\NP)/cat_np` and saturates over `carrying the variant`, two tokens or more. The
INTRANSITIVE is saturated at ONE token, and both chart drivers run the `UnaryShift` table inside
`for len in 2..=n`, so `ger_participial_lifts` emits it at seed time — calling `reduced_relative`
rather than restating the mapping, which is what item 1 does for `pass`.

| | before | after | restrictor now |
|---|---|---|---|
| «A patient carrying the variant was found.» | GAP | CLOSED×2 | `ΣG#1:n10405694. v01449974_t(the(n07366627), G#1)` |
| «A method resolving the question is reliable.» | GAP | CLOSED×4 | `ΣG#1:n05660268. v00698855_t(the(n07193596), G#1)` |
| «A method that resolves the question is reliable.» | CLOSED×12 | CLOSED×12 | `ΣG#1:n05660268. v00698855_t(the(n07193596), G#1)` |
| «A patient arriving was found.» | CLOSED×2 | CLOSED×6 | `ΣG#1:n10405694. v02005948_i(G#1)` |

The reduced form derives a restrictor IDENTICAL to the `that`-ful form's. That is the correctness
check; the reading count alone would not have caught the compound pile.

### Two probes stay shadowed, and it is lexical

«A patient waiting …» (all three forms) and «The gene encoding a helicase …» are unchanged, at the
same reading counts. The sems still show the adjective (`deg_a01450713`) and the noun inside the
compound pile, so the `ger` verb entry is not reaching the chart for those surfaces at all. The run
config is `SENSE_CAP=2 CELL_BEAM=64`; which of the two levers cuts it is not localized.

### What it cost, and what it bought

The XIAP page, 325 prose units, same snapshot and arms:

| | before | after |
|---|---:|---:|
| grammar-gap | 281 | **279** |
| ambiguous | 11 | **13** |
| encoded / open | 1 / 5 | 1 / 5 |
| readings | 2303 | 4015 |
| skeletons | 521 | 1190 |
| wall-clock | 814s | 884s |

**The whole reading increase is the two new units.** 4015 − 2303 = 1712, and the two added entries
are `AMBIG×1658` and `AMBIG×54` — 1712 exactly. The histogram says the same thing by bucket: every
bucket is unchanged except `31-100` (2 → 3) and `>100` (4 → 5), the two the new units land in. No
existing unit gained a reading or moved a bucket, so the doubled skeleton count is those two units'
own ambiguity in units that previously produced ZERO, not a tax on the document. The spread cost is
the 9% wall-clock: 52 units seed extra leaf items that never reach a reading.

The two units converted, by construction (the page itself is not reproduced here — see
`docs/method/`'s data rule):

| unit | tokens | the `ger` span | readings |
|---:|---:|---|---:|
| 3 | 16 | a post-posed `, <adverb> -ing <NP>` adjunct after a copular predicate nominal | 1658 |
| 65 | 8 | a bare plural head + `-ing <NP>` + two coordinated PPs | 54 |

Two units, against one each for items 1 and 2 — the highest per-construction yield measured on this
page so far, and still 2 of 325.

## The DON'T-row census: no row is a lever, and length still is

Prompted by asking what item 5 was worth to the CNL rewrite. Step 4's rewrite is mechanically the
guide's DON'T rows applied at every occurrence, so a row's cost is its OCCURRENCE count, not its row
count. Item 5's row stood over ~15 of the 325 units while converting 2 — and its own remedy warned
that the obvious rewrite (splitting) changes the claim.

That invites the question for the rows that remain. Over the same 325 units, against the post-item-5
outcome set (279 gaps):

| DON'T row (mechanically countable) | gap units | all units | gap-rate |
|---|---:|---:|---:|
| clause-position `:` or `;` | 78 | 93 | 84% |
| em-dash appositive | 66 | 76 | 87% |
| parenthetical aside `(…)` | 61 | 73 | 84% |
| en-dash inside a word | 21 | 25 | 84% |
| sentence-initial `So` | 4 | 4 | 100% |
| `without` + gerund / parenthetical adverb / fronted conditional / free relative | 2 each | 2 each | 100% |
| **base rate** | **279** | **325** | **86%** |

**Every countable row sits AT the base rate.** Carrying one of these patterns makes a unit no more
likely to fail than a unit without it, so none of them is a lever — they are frequent, not
blocking. The 100% rows are n = 2-4 and carry nothing.

Overlap says the same thing from the other side. Counting how many of the five most frequent
patterns each gap unit carries:

| patterns carried | gap units | |
|---:|---:|---|
| 0 | 128 | **46%** |
| 1 | 71 | 25% |
| 2 | 36 | 13% |
| 3 | 34 | 12% |
| 4-5 | 10 | 4% |

**46% of gap units carry none of them**, at a median 21 tokens against 22 for all gap units — so
they are not the short, simple residue either. They fail for reasons the DON'T rows do not name.

What does discriminate is length, and the separation is clean:

```text
PARSED units:  median 11 tokens, range  5-31
GAP    units:  median 22 tokens, range  0-59
```

Exactly 2x, and no parsed unit exceeds 31 tokens while gaps run to 59.

**Consequence for step 4.** The rewrite is driven by LENGTH, not by the DON'T-row checklist; the rows
are a correctness filter to apply WHILE rewriting, not the thing that decides what to rewrite. This
is also why the v1 splitter was the right instrument failing on execution — splitting is the correct
lever, and it regressed 17 -> 14 only because label colons are not clause boundaries.

It also fixes what a grammar fix is worth here. Not its converted-unit count (0-2), but the number
of obligatory, meaning-risky edits it deletes from a rewrite whose real job is elsewhere.

## Survey §4 step 4, as specified: per-kind over the 427 gold claim spans

Step 4 says "run the harness on the XIAP markdown's claim SPANS as a corpus — the register gives 427
gold spans with kinds, so the measurement is PER-KIND". Every measurement up to now used the 325
narrative prose units of the page instead. Run `2026-10-14` on
`wordnet-umls-hpo-aligned-2026-10-13-fa`, cap-only, same arms:

| kind | gold | non-prose | prose | parsed | rate on prose |
|---|---:|---:|---:|---:|---:|
| `observed` | 220 | 61 | 159 | 6 | 3.8% |
| `declared` | 70 | 1 | 69 | 7 | **10.1%** |
| `derived` | 69 | 2 | 66 | 3 | 4.5% |
| `marked` | 43 | 0 | 43 | 6 | **14.0%** |
| `compressed` | 15 | 0 | 15 | 0 | 0% |
| `exempt` | 10 | 0 | 10 | 0 | 0% |
| **all** | **427** | **64** | **362** | **22** | **6.1%** |

**The corpus choice is immaterial, which was worth measuring because the lengths differ.** Claim
spans run median 18 tokens against 23 for page units, p75 26 against 37, and 88% sit under the
31-token ceiling against 69%. On the prose denominator they parse at 6.1% against the page's 6.4% —
the same. Length is NECESSARY and not sufficient, and this is the measurement that says so: a corpus
a quarter shorter at the median converts nothing extra.

**`missing-lexeme` is 0 over all 427.** The lexicon is not the blocker on any claim in the register.

### 64 of the 427 gold spans are bibliographic records

`non-prose` concentrates almost entirely in `observed` (61 of 64), and the cause is one shape: 56 of
the 64 carry a `PMID`, inside a reference-list entry — author list, italic title, journal, year,
pages, PMID. The fact-check skill registers these as `observed` claims because it certifies that the
citation RESOLVES, which is a real obligation, but a bibliographic record is not a proposition. The
harness classifies them `non-prose` correctly.

**This is a document-model finding, and it belongs to step 5.** A citation is PROVENANCE — it is what
the certificate's `resource:` / `data:` fields already hold for every other `observed` claim — not an
assertion with a truth condition. D96 has to separate the two, or 13% of the register is permanently
unparseable by construction and counted as failure.

### What the per-kind spread says

`marked` is the best-covered kind (14.0%) and `observed` the worst (3.8%) while being 52% of the
register. `compressed` and `exempt` are 0 of 25 — by their own definitions a compression spans
several claims and an exemption states why a claim is not checkable, so neither is a single
assertion. Those two kinds are candidates for the same treatment as the bibliography: not prose to be
parsed, but structure to be represented.

## Step 4 executed: the CNL rewrite of the XIAP page, and what it bought

The whole page rewritten against the `2026-10-15` guide — R1's token budget, R2 faithfulness, R3
citations to provenance — then measured on `wordnet-umls-hpo-aligned-2026-10-13-fa`, cap-only.

| | source | CNL rewrite |
|---|---:|---:|
| units / sentences | 325 | **1,056** (3.84x) |
| median tokens | 23 | **9** |
| max tokens | 103 | **22** |
| over the 31-token budget | 102 | **0** |

275 source units rewrote as claims; 50 went to provenance under R3 (the reference list and the
document header).

### The headline is 2x, not 8x

| | raw prose | CNL |
|---|---:|---:|
| **source units FULLY covered** (every piece parses) | 19 / 298 = 6.4% | **37 / 275 = 13.5%** |
| partially covered (some pieces parse) | — | 191 (69.5%) |
| no piece parses | — | 47 (17.1%) |
| *sentence level* | *5.85%* | *45.9%* |

The sentence-level 45.9% runs on a denominator the rewrite itself inflated 3.84x, so the honest
metric is original units fully covered. That is 6.4% → 13.5%.

`missing-lexeme` 0, `scale-bound` 12 → **0**, `non-prose` 15 → **7** — and all 7 survivors are one
token, `Å`.

### Two hypotheses, A/B tested; one died

The gap analysis against the 53.8% base rate ranked mid-sentence `therefore`/`also` at +15.3pp over
97 sentences. **Refuted** by a 28-sentence A/B that moves the adverb to the sentence-initial position
DO §11 licenses and changes nothing else:

```text
  MID   half: 1 parsed of 14
  FRONT half: 1 parsed of 14      identical, and the same sentence in both forms
```

Correlation, not cause. Acting on the ranked table would have rewritten 97 sentences for nothing —
the same trap the DON'T-row census documents, hit again on this page's own gap set.

**A bare decimal numeral does not parse.** Confirmed by the same method, swapping the decimal for an
integer and touching nothing else:

```text
  DECIMAL half: 0 parsed of 15
  INTEGER half: 7 parsed of 15
```

The 8 that stay gap carry a second blocker (scientific notation, or a comparative over a quantity).
This is a D95 gap, not a style problem, and it falls hardest on the methods and predictor-score
material where nearly every claim carries a number.

### An A/B corpus FEEDS the recurrence-based named-entity miner

Found while checking a sentence that disagreed between runs — `GRAMMAR-GAP` in the 1,056-unit run,
`AMBIG` in the 28-unit A/B, same snapshot and flags, 0.7s against 5.7s. The augmentation lines say
why:

```text
  full run (1,056)  named entities: ["map Ile380", "Ensembl VEP", "protein HGVS", "MANE CDS", …]
  A/B  run (   28)  named entities: ["named XLP-2"]
```

The miner requires RECURRENCE across the document, and an A/B corpus holds every sentence twice,
once per arm — so each arm's content words recur by construction. The A/B minted a named entity the
real document does not grant, and that is what parsed the sentence.

**Both arms share the corrupted augmentation, so a within-run A/B comparison stays valid** — MID vs
FRONT and DECIMAL vs INTEGER are unaffected. What does not transfer is the ABSOLUTE rate in an A/B
run, and a premise carried in from the document's context ("all 14 of these were gaps") is not true
in the A/B's own context. Read an A/B here as a contrast and never as a coverage figure.

The same property means a parse outcome on this corpus is DOCUMENT-SCOPED. Comparing two runs
requires either the same corpus or the knowledge that the augmentation moved with it; the page vs
CNL comparison above is a comparison of two real documents, each with its own augmentation, which is
the intended reading.

### The quantity gap is decisive per sentence and NOT the lever per unit

| | fully-covered units |
|---|---:|
| now | 37 (13.5%) |
| blocked only by decimal-bearing lines | +7 |
| blocked only by decimal- and `Å`-bearing lines | +3 |
| **upper bound if both parsed** | **47 (17.1%)** |
| blocked by something else | **228** |

An upper bound, since the A/B recovered 7 of 15 rather than all. **228 of 275 units are blocked by a
diffuse tail** — the same shape the DON'T-row census found, now measured on the rewritten corpus:
no single cause dominates, and the residue is long.

## Step 5 item 1: a dimensionless measured value is a claim (`cat_numpred`)

The decimal gap, localized. **No combinator consumes `cat_num`** — the grep over
`kernel/src/dcg/rules/` is empty. A numeral is usable only through the cardinal determiners, and
those are gated on `value.is_integer()`, so a decimal had no route in any position:

| position | integer | decimal |
|---|---|---|
| determiner, `expects N variants` | AMBIG | GAP |
| copula complement, `The ratio is N.` | ENCODED | GAP |
| verb object, `The score reaches N.` | GAP | GAP |

### Lifting the integer gate would have been the wrong fix

`has_count(…, 21.3)` asserts a set has 21.3 members. "21.3 expected missense variants" states a
model's EXPECTATION. That is a parse meaning something false — the `compound_kind` failure mode
again — and R2 forbids it. The honest encoding is `has_quantity`, which D95 decision 1 already
defines, and the position for it is the copula complement.

### A provenance guard failed, and a category did not

D95 slice 7 decision 6 had already built and removed the numeral's measure-phrase seeding: a bound
marker took it, `mod_lifts` made it prenominal, and `at least 1,000 cells` gained a second reading.
A first cut restored only the predicative adjective `S[adj]\NP`, tagged, with `mod_lifts` refusing
the tag. **It measured the defect straight back** (`2026-10-21`):

| | baseline | provenance tag | `cat_numpred` |
|---|---:|---:|---:|
| `At least 1,000 cells were scored.` | ×2 | **×4** | **×2** |
| `At least 1,000 cells per sample were scored.` | ×4 | **×12** | **×4** |
| `The ratio is 0.94.` | GAP | ENCODED | ENCODED |
| `21 variants` / `Five cell lines` / `517 cell lines` / `37 °C` | ×6/×6/×4/×2 | unchanged | unchanged |
| total readings | 25 | 37 | **27** |

The sems named the cause: `has_quantity` inside the noun's Σ restrictor, reached because `at least`
composed with the leaf FIRST and the composed adjective carried the combinator's provenance, not the
leaf's. **A provenance tag cannot survive the composition that precedes the lift.**

`cat_numpred` is a distinct category with ⟦·⟧ = `Entity → Prop` and exactly four consumers, the
is/are/was/were copula entries. `mod_lifts` keys on `is_adjective_cat` and does not match it; no
adjective-modifying adverb composes with it; no measure consumer or bound marker sees it. The
defect is unreachable rather than blocked, and the sems confirm it — the Σ-restrictor readings are
ABSENT, not outnumbered. The +2 readings over baseline are exactly the two intended ones.

### What it bought, one variable, matched bootstraps

The baseline arm also settled a standing question: on the cleaned bootstrap the CNL corpus measures
**byte-identical** to the retired `fa` store (12/325/148/564, 37,954 readings, 2,878 skeletons), so
the repo cleaning and the LLM removal were inert for parsing and only invalidated the manifest stamp.

| CNL corpus, 1,056 sentences | baseline | `cat_numpred` |
|---|---:|---:|
| parsed | 485 | **495** |
| grammar-gap | 564 | **554** |
| readings | 37,954 | 38,311 (+0.9%) |
| skeletons | 2,878 | 2,903 (+0.9%) |
| source units fully covered | 37 (13.5%) | **38 (13.8%)** |

All 10 conversions are `GRAMMAR-GAP →` parsed, and every one is the target class: a copular
sentence whose complement is a bare dimensionless measured value — a ratio, a predictor score, a
model delta. The +1 unit against
+10 sentences is the diffuse tail again: a unit needs every piece to parse, and 228 of 275 are
blocked by several things at once.

## Order of work

By units unblocked per unit of work, on these measurements:

1. ~~The reduced relative + PP~~ — **done**, above. Measured: 1 page unit.
2. ~~`rather than`~~ — **done**, above. Measured: 1 page unit.
3. ~~The participial post-modifier with an object~~ — **done**, item 5 above. Measured: 2 page units.
4. **The zero relative and the zero complementizer.** Both `that`-ful forms parse, so this is one
   omission in two places rather than two constructions.
5. **VP coordination under an auxiliary** («has been settled and is recorded»).
6. **The closed-class omissions** — `where`, `rather`, `so`, `neither`, `what`, `unless`, the
   reflexives, `cannot` — batched into one reseed. 17 + 12 + 6 + 4 + 4 gaps carry one.
7. **The comma appositive**, the noun comparative, the free relative, `both … and`, the
   demonstrative partitive, the parenthetical adverb, unlike-category coordination.

**But the measured yield is 0-2 units per construction** (1, 1, 0, 0, 2 for items 1-5), so the order
above closes the gate slowly if at all: five constructions moved the page 16 → 19 of 325. The leverage is step 4: the clause splice alone recovers 11 of 145 pieces, and a rewrite
addresses length — the dominant variable — directly, where a grammar rule does not.

Nothing above the lexicon is in doubt: `missing-lexeme` is 0 over the corpus, and the one lexical
gap the battery found (`cannot`) is a closed-class omission rather than a content word.
