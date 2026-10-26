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

## Step 5 item 2: the ångström — a missing UNIT, not a grammar gap

`units.esl` defined 64 symbols and ångström was not among them, so `3.46 Å` had no seeding at all —
the whole of the corpus's residual `non-prose` class.

```text
resource units:angstrom : units:NamedUnit {
    units:symbol = "Å"; units:dimension = u"m"; units:factor = 0.0000000001r;
    units:factor_pi = 0; units:prefixable = false;
}
```

Three judgements, each of which could have gone the other way:

- **U+00C5, not U+212B.** The corpus uses U+00C5 exclusively (14 occurrences) and it is the
  canonical form; U+212B ANGSTROM SIGN is the Unicode compatibility character that normalises to it.
  Same code-point discipline `units:symbol` already states for μ and Ω.
- **Not prefixable.** `mÅ` and `kÅ` are not written, and admitting them mints surfaces nothing uses.
- **`bp` / `nt` / `kb` deliberately EXCLUDED.** They surfaced in the same survey, but a base pair is
  a countable thing rather than a dimension, so per D95 they belong in `has_count`, not
  `has_quantity` — and they cause no failures. Adding them as units would have been the easy wrong
  move.

**Verified without a database, before the reseed.** `quantity_corpus.rs` parses over the bootstrap
chain and the corpus's own content words, so a row (`The distance was 3.46 Å.`) proved the unit
tokenizes, the sentence parses, the reading contains `has_quantity`, and the value normalises to
`173/500000000000 m` exactly — an hour before a store existed to use it.

| CNL corpus, 1,056 sentences | `cat_numpred` | + `Å` |
|---|---:|---:|
| encoded | 13 | **17** |
| parsed | 495 | **501** |
| **non-prose** | **7** | **0** |
| grammar-gap | 554 | 555 |
| source units fully covered | 38 | **41 (14.9%)** |

All 7 resolved: four to ENCODED with a UNIQUE reading, two to AMBIG, one to grammar-gap. The net
`+6` hides that last one, which is why the per-sentence diff matters — its near-identical sibling
parses, so the residual cause is the gerund-subject construction, not the unit.

**Cumulative source-unit coverage**: raw prose 19/298 (6.4%) → CNL rewrite 37 (13.5%) →
`cat_numpred` 38 (13.8%) → `Å` **41 (14.9%)**.

## The gap breakdown, and two corrections to it

**Gaps are not equal, and the useful cut is by VALUE.** Of the grammar gaps, only those that are
the LAST failing sentence of their source unit convert a unit when fixed. Measured after the
identity copula (274 claim units):

| sentences still failing | units | |
|---:|---:|---|
| 0 | **51** | fully covered |
| 1 | **94** | ONE away — each of these 94 gap sentences is worth a whole unit |
| 2 | 55 | |
| 3 | 34 | |
| 4+ | 40 | |

Fixing all 94 would take coverage 51 → 145 of 274. Every other gap is worth zero on its own.

### Classification of the last-blockers, by hand

A regex census was run first and was useless — the best lift was +10.3pp on n=45 against a 16%
base, and 52 of 91 went unclassified. Reading them and assigning a type by hand gave the table
below. **Every row is a CANDIDATE cause until a one-variable A/B confirms it**; the adverb-placement
row is the standing warning, since that shape scored +15.3pp over 97 sentences and died at 1/14
against 1/14.

| construction type | units | status |
|---|---:|---|
| definite predicate nominal (`X is the N`) | 12 | **BUILT** — the identity copula, below |
| adverb placement (mid-sentence / fronted) | 10 | candidate |
| free relative / embedded question (`what`, `which`, `how much`) | 8 | candidate |
| degree / scalar (`very likely`, `slightly above`, `more X than`) | 6 | candidate |
| `both` / `all` / partitive as SUBJECT | 6 | candidate |
| negative / indefinite subject | 5 | candidate |
| bare mass or gerund subject | 5 | candidate |
| modal + bare subject | 3 | candidate |
| `both X and Y` as an argument | 2 | candidate |
| gerund as complement | 2 | candidate |
| imperative · `neither … nor` · `rather than` + participle | 1 each | candidate |
| one-offs with no shared type | 19 | — |

### Correction 1: `name-as-copula-subj` was not a construction at all

Seven units were filed under "a proper name cannot be a copula subject", marked CONFIRMED on a
ladder. **Withdrawn.** The ladder used `dbSNP` in every arm, so it confirmed a property of ONE
word's grounding and the result was generalised to proper names. Widening it refutes the reading:

```text
  GRAMMAR-GAP  dbSNP is informative.          GRAMMAR-GAP  dbSNP returns three records.
  GRAMMAR-GAP  dbSNP matters.                 AMBIG        dbSNP has assigned no identifier to c.1139.
  ENCODED      MaveDB is empty.               AMBIG        Lynch syndrome is common.
```

`be` is not the discriminator (`MaveDB is empty` parses), transitivity is not (`has assigned` parses
and `returns` does not), and the `Num` feature is not (`Lynch syndrome`, the lattice's own `name`
example, parses). What varies is the OOV GROUNDING: these are resource names absent from the
lexicon, and the augmentation injects an entry bound to whatever the text index matched, so whether
one works as a subject depends on what it grounded TO. The seven units leave the construction table
and become an OOV-handling question — plausibly cheaper, since they are names the document uses
throughout and the glossary already has recurrent-name machinery.

### Correction 2: the definite predicate nominal was never blocked on D98 slice 1

It was recorded here as blocked, needing `Id`, which "ESL cannot write yet". **Wrong in both
parts.** `eigentt:Eq(T, x, y)` is the ESL surface spelling of the kernel's `Id` and has existed
since D99 §10 — `variant.esl` already uses it. The grep that "established" its absence searched for
`Id(` in `ontologies/`, which the surface spelling could never match: the kernel's name was searched
for, ESL's was not, and a 2026-09-29 statement in D95 was treated as current. That is the
"'open' is a claim about absence" trap.

The real obstacle was narrower: the natural shape needs `the(T) : T` at `Entity`, and coercive
subtyping cannot reach `Entity` from a BOUND variable. Putting the identity on the COPULA instead
avoids needing it — `fwd(bwd(cat_s(dcl,fin), cat_np(Entity,sg)), cat_np(Entity,num_any))` over
`λy. λx. eigentt:Eq(Entity, x, y)`, so both sides arrive at concrete classes and coerce at the
argument slots.

### Correction 3: built, kept, and ENF traded against coverage on purpose

It works. Measured over the 1,056-sentence CNL corpus on a reseeded store
(`2026-10-24-identity`, provenance 38,391 / 20,533 / 864), all four finite forms:

| | base | identity copula |
|---|---:|---:|
| parsed | 501 | **557** |
| grammar-gap | 555 | **499** |
| source units fully covered | 41 | **51** of 274 |
| sentences LOST | — | **0** |

Sems verified on its targets — `ni_p_ile380thr = the(Σsubject. prep_of(…)).1`,
`the(Σaxis. compound(…)).1 = the(Σarm. compound_kind(…)).1`.

**It broke two pinned uniqueness invariants, and the pins were relaxed rather than the entry
dropped.** `HeLa is a cell line` goes from 1 parse to 2:

```text
  [0] is_a(hela, CellLine)                               the pinned reading
  [1] ΠG#0:Prop. ΠG#1:CellLine. hela = G#1 → G#0 → G#0   ∃x:CellLine. hela = x
```

The second is **logically EQUIVALENT to the first**, not a wrong reading. So the invariant at stake
is ENF — the codebase refuses a route that re-derives what another already gives, in the words
`ObliqueParticipial` uses for itself — and not correctness.

**The stance, decided 2026-10-24: ENF is a strong default; COVERAGE is the non-negotiable.** The
alternative was losing the construction entirely, at 56 sentences and 10 source units, to avoid a
redundant SPELLING of a proposition the grammar already gets right. The cost is ranking and chart
size, never a distorted claim — which is the distinction that decides it. R2 protects against
distortion (the `compound_kind` noun pile, `waiting` as an adjective degree: readings that MEAN
something false). ENF protects against redundancy. Only the second is being traded.

The first instinct here was to revert and record the construction as blocked. That was wrong, and
the reasoning that corrects it was already written down: *the RIGHT reading outranks reducing
reading/skeleton count; coverage stays non-negotiable*. The correct reading is present either way.

Narrowing it properly — so the identity route does not fire where the predicate-nominal route
already covers it — needs a **definiteness feature** in the category system (today `the_subj` and
`a_subj` are BYTE-IDENTICAL, definiteness living only in the sem), or the copula recast as a **RULE**
so it can carry a `ProvGuard`; those are per-rule and cannot key on a lexical entry. Either is real
design work, and neither blocks the construction now.

### Follow-up: converge the two spellings instead of tolerating both

The stance above trades ENF for coverage. A better answer dissolves the trade: make
`∃x:T. s = x` and `is_a(s, T)` converge on ONE representation, so the two derivations collapse to
one reading — ENF restored, coverage kept, no definiteness feature needed. Two routes, both
design-level.

**(a) THE KERNEL ROUTE — give `is_a` a resolved meaning at the checker seam.** This is D18's
pattern, not a new one: an `EigonClass` IRI stays opaque in the term and the CHECKER resolves it to
a nested Σ on demand (`CheckHooks::resolve_class`, `program::ground`). `is_a` would work the same
way — the term keeps `ontology:is_a(s, T)`, and the kernel knows it MEANS `∃x:T. Id(Entity, s, x)`,
so `conv.rs` finds the two convertible when it normalizes both sides.

Today it cannot: `EigonAxiom` evaluates to a NEUTRAL (`Neut::EigonAxiom`), axioms never δ-unfold,
and that opacity is the whole reason the two spellings stay distinct. The change is to let an axiom
that carries a chain-resident definition unfold, leaving one with none opaque as now — the exact
analogue of "a class with a Σ resolves, a primitive does not".

- **The surface does not change.** `is_a(hela, CellLine)` stays textually identical in all ~33
  pinned sems across 12 test files; what changes is that previously-distinct readings become
  IDENTIFIED. Reading counts fall — the two uniqueness pins relaxed above would return to 1, which
  is the point.
- It also buys reasoning power the opaque form cannot have: a transparent `is_a` is discharged by
  `refl` when `s : T`.
- *Cost and caution:* the opacity is deliberate — `is_a` is "grounded downstream by ChainWitness" —
  so this must not make the grounding vacuous. And δ-unfolding axioms is a kernel-wide capability,
  not a local fix: every axiom with a definition becomes transparent, which is a decision about the
  ontology's whole stance on opacity.
- *Check first:* whether `∃` here is the impredicative encoding `passive_sem` uses, since
  convergence needs the identity reading's Church form to normalize to the definition's.

(a) is the right one, and the owner's call (2026-10-24): it makes the equivalence a fact the kernel
KNOWS rather than a rewrite asserting it downstream, and it follows the architecture already in
place for classes rather than adding a pass beside it. (b) would assert the same equivalence from
outside the kernel, where nothing can check it.

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

## The gap-boundary instrument (2026-10-24)

Four of the five hypotheses in this note were formed by behavioural A/B — minimal pairs, ablations,
frequency tables — and four were refuted. The one that held (`UnaryKind::IdentityPredicate` never
fires because definites arrive type-raised) came from instrumentation. The reason A/B keeps failing
here is structural: it measures the output of the whole pipeline, so it cannot separate a missing
category from a feature clash from a normal-form guard. Those three call for different fixes.

`kernel/src/dcg/chart/boundary.rs` answers the question directly, on the real chart.

**What it computes.** A cell `[i..j]` is derivable iff the packed forest holds a node there. Tile the
token span with as few derivable cells as possible. A sentence that parses tiles in 1; a gap tiles in
*k* > 1, and the *k* − 1 seams are where the grammar ran out. A cut position is **forced** iff no
minimal tiling straddles it — a property of the grammar's coverage of the sentence rather than of
which decomposition the DP happened to reconstruct. At each seam it crosses every left-block node
against every right-block node and reports the **nearest miss**, ranked:

| Verdict | Meaning | What it points at |
|---|---|---|
| `PRUNED` | `apply` succeeds on the reps, yet the cell is empty | sense cap / cell beam, **not** the grammar |
| `NORMAL-FORM` | slot unifies, no mode objects | an Eisner `ProvGuard` |
| `MODE` | composition available, a slash mode refuses it | the mode lattice |
| `FEATURE(pos)` | same constructor, one position fails its own test | a `feat_meets` / subsumption gap, position named |
| `WRONG-ARG` | functor faces the right way, different constructor | no entry for the construction |
| `NO-FUNCTOR` | neither side is a functor facing the other | wrong category outright |

Each verdict re-runs the predicate the parser itself would have run (`unify_cat`, `feat_meets`,
`type_subsumes`, `harmonic_licenses`, the provenance guards), so the label names a cause instead of
classifying a surface shape. `feature_position` mirrors `unify_into` **per position** — `cat_s`
matches mood by equality and finiteness by the feature-meet, and one uniform predicate across all
positions would mislabel a mood clash as `WRONG-ARG`. Composition is tested `fwd` only, because
`comb_rules()` registers `forward_comp` and no backward counterpart; testing `bwd` would report a
refusal for a rule the parser never tries.

Run it:

```
EIGENIUS_DB_SNAPSHOT=… EIGENIUS_PROBE_PAGE=/abs/cnl-page.txt EIGENIUS_PROBE_FILE=/abs/gaps.txt \
  cargo test --release -p eigenius-wordnet --test db_backed_encoding \
  probe_gap_boundary -- --ignored --nocapture
```

The kernel prints the report itself under `EIGENIUS_GAP_BOUNDARY`, gated on the sentence having
produced no top-span clause, once per cap rung with repeats elided.

**Caveats the output states.** The cross product runs `apply` (the universal combinators, the
nominal-modification and other-grammar rules, the group rules) and, through a caller-supplied
callback, the *token-keyed* binary rules, which are keyed to reserved words the forest does not
carry. A token-keyed rule that succeeds yields `PRUNED` like any other. The first version instead
reported whether a seam *coincided* with a binary-rule SITE — which says nothing, since
`trig_appose_group` emits a site at every split of every cell, so the note fired at all three seams
below and meant nothing at any of them. Verdicts are exact for everything a packed node holds (the
packing invariant makes the decision a function of the signature), but the sem shown is the
representative's.

### First three results

Measured on the identity snapshot over the CNL page's augmentation.

**`The worst case is that it behaves like a null allele.`** — forced cut at 4, before «that». The
that-clause **does** build, as `cat_cp`. The left block is the copula composed to
`S[dcl,fin_any]/X` for seven different X: `S[dcl,adj]\NP`, `S[dcl,ger]\NP`, `S[dcl,pass]\NP`,
`S[dcl,pred]\NP`, `NP`, `cat_numpred`, and a `pss` functor. None of them is `cat_cp`. So this is not
a construction gap: **no copula entry takes `cat_cp` as complement.** One closed-class entry,
parallel to the `cat_numpred` copulas.

**`This is the gene.`** — forced cut at 1, before «is». `This` carries only determiner
categories, `cat_forall(sg, λT. …)` and `cat_det_premod(sg, λT. …)`. There is no `cat_np`. The right
block is a saturated `S[dcl,fin]\NP[Entity,sg]` wanting a subject. **`this` has no pronominal
`cat_np` entry.** One closed-class entry.

**`The claim is still the strongest evidence.`** — the report put a forced cut before «the» at
cap 2 (unforced at cap 4, where extra `ForwardComp` nodes appear), with the seam
`[0..3]«The claim is still»` ⊕ `[4..6]«the strongest evidence»`. The right block's six nodes are all
`cat_detmod` or type-raised. My first reading of this — that the gap is a definite NP having no
`S[pred]\NP` form, and that `still` already carried a complement-preserving category so the adverb
was not the gap — was **wrong on both counts**. `EIGENIUS_TRACE_FOREST` on the adverb-free control
and on `still` itself settles it.

The control `The claim is the strongest evidence.` derives as:

```
#15 [2..2] «is»   cat = (S[dcl,fin]\NP[Entity,sg]) / NP[Entity,num_any]
                  sem = λy. λx. x = y
#67 [3..5] «the strongest evidence»  =  the object type-raised  VP\(VP/NP)
     └ Combine@k=2, BackwardApp  →  #73 [2..5]  S[dcl,fin]\NP[Entity,sg]
#48 [0..1] «The claim»  =  subject-raised  S/(S\NP),  ForwardApp at k=1
```

So the control parses through the **identity copula** (the `ad6d253d` entry), with the raised
definite backward-applying to it. It never builds a predicate nominal, and never needs
`S[pred]\NP`. What it needs is `is` and the NP **adjacent**, because `VP\(VP/NP)` selects a `VP/NP`
immediately to its left.

`still` has ten leaf categories and **none of them is adverbial**:

| Category | Reading |
|---|---|
| `S[dcl,adj]\NP` | predicative adjective — "the water is still" |
| `S[dcl,pred]\NP` | predicate nominal (KindRaised) |
| `cat_measure`, `cat_mod` | |
| `cat_n(n04319248, mass)`, `cat_n(n04319248, sg)` | the **noun** "a still" (distillation apparatus) |
| `cat_np(n04319248, sg)` + subject/object raises | raises of that noun |

The same ten in verbal position (`The gene still affects cells.`), so it is a lexeme fact, not a
positional one. `is still` [2..3] therefore builds as a **saturated** `S[dcl,fin]\NP[Entity,sg]` —
"the claim is motionless" — `The claim is still` is a complete sentence, and the real predicate is
stranded; hence the cut before «the» rather than before «still». The `S/(S[pred]\NP)` forms at
`[0..3]` that I read as evidence of a complement-preserving adverb are `is` composing with still's
**raised noun**, not an adverb entry.

But the missing adverb entry is **not** what blocks this family. `also` has the full adverb set —

```
(S[dcl,fin]\NP[ADV#num]) \ (S[dcl,fin]\NP[ADV#num])
(S[dcl,ADV#fin]\NP[ADV#num]) / (S[dcl,ADV#fin]\NP[ADV#num])
S[dcl,fin_any] \ S[dcl,fin_any]        S[dcl,fin_any] / S[dcl,fin_any]
```

— and gaps identically. The one-variable grid:

| Sentence | Adverb's categories | Result |
|---|---|---|
| `The claim is also the strongest evidence.` | full adverb set | **GAP**, same seam, `WRONG-ARG` |
| `The claim is therefore the strongest evidence.` | full adverb set | **GAP**, same seam |
| `The claim is still the strongest evidence.` | none adverbial | **GAP**, same seam |
| `The claim is also strong.` | full adverb set | parses |
| `The claim is still strong.` | none adverbial | parses (closed=12, after widen) |

The variable is the **complement type**, not the adverb. An adjective complement parses; a definite
NP complement — the identity reading — gaps whatever the adverb. The reason is categorial: every
adverb category above modifies a **saturated** VP or S. The identity copula is `(S\NP)/NP`,
unsaturated, and its raised definite complement `VP\(VP/NP)` selects an adjacent `VP/NP`. No
VP-modifier can occupy that position, however well-typed.

**The fix is therefore a category this grammar does not have**, not a lexical entry: a
functor-modifier `((S\NP)/NP)\((S\NP)/NP)`, so an adverb modifies the transitive copula in place
and `is still` stays a `VP/NP`. Filling `S[pred]\NP` instead would give the sentence a predication
reading where the correct one is identity.

*Open, and a reason not to write that category yet:* an unrestricted `(VP/NP)\(VP/NP)` adverb also
licenses an adverb between any transitive verb and its object (`*affects still cells`). The category
needs scoping — to the copula, or to the identity copula specifically — and that is a design
decision, not a lexicon edit. Two measurements still missing before it: whether the other six units
in this family share this shape, and whether `still` lacking adverb entries is a separate real gap
(`The gene still affects cells.` parses with 4 readings, which with no adverb category available
means it parses WRONGLY — the readings have not been inspected).

The work order this implies is not the one this note carried. Two of the three are single
closed-class entries — a copula taking `cat_cp`, and a pronominal `cat_np` for `this`. The third is
a new category coupled to the identity copula, with an open over-generation question, so it is the
largest of the three rather than the smallest.

Three claims in this note were refuted by the measurements above and are withdrawn: that `still`
already had a complement-preserving category; that the blocker was the definite NP's missing
`S[pred]\NP`; and that the family is a per-lexeme adverb gap fixable by giving `still` adverb
entries. The first two came from reading the gap report's categories without asking where they came
from — they were real, their provenance was not what I assumed. The third came from stopping at the
first measurement that fit (`still` has no adverb categories) instead of running the control that
would break it (`also` does, and gaps anyway).

Two usage rules for the instrument follow. The gap report names a seam and a verdict; it does not
name a derivation, so `EIGENIUS_TRACE_FOREST` on a **passing** control is what identifies the route
the gapping sentence is missing. And a category dump explains a lexeme, not a family — the family
needs the one-variable grid across lexemes.

## The three diagnosed gaps, addressed (2026-10-25)

All three came from the gap-boundary instrument. Two are closed; the third is blocked on a design
decision it shares with work already on the books.

### 1. The `cat_cp` copula complement — a CNL rule, not an ontology commitment

`X is that P` needs the copula to take a `cat_cp`, whose denotation is `Prop`, against a subject of
type `Entity`. Bridging those means reifying propositions as entities. `judgements-and-warrants.tex`
takes no position on that — its reification is of the proof-checker operator — so this would be a new
ontological commitment, made for **one unit**.

Measured instead, on the identity snapshot:

| | |
|---|---|
| `The worst case is that it behaves like a null allele.` | GAP, forced cut before «that» |
| `In the worst case, it behaves like a null allele.` | GAP — cut before «case», `NO-FUNCTOR` |
| `It behaves like a null allele in the worst case.` | **parses** |
| `The worst case is a null allele.` | parses, closed=32 |

The postposed PP carries it. So this is a CNL rewrite rule — move the scenario PP to the end — and
the reification stays available if the construction ever earns it.

**Separate finding:** the FRONTED `In the worst case,` form gaps for an unrelated reason, splitting
`«In the worst» ⊕ «case , it behaves…»` with `NO-FUNCTOR`. That is a comma-fronted-modifier gap, and
fronted PPs are common enough to be worth more than the construction this entry was chasing.

### 2. The bare demonstrative pronoun — built, and narrowed by a one-variable arm

`this` / `that` / `these` / `those` had only DETERMINER entries (`cat_forall`, `cat_det_premod`).
Standing alone they have no category at all, which the instrument named outright: a forced cut at 1
and `WRONG-ARG` against a saturated `S[dcl,fin]\NP` wanting a subject, with no `PRUNED` pair, so the
cell was empty for want of a category and not for want of beam.

The entries are `it_pron` with different forms — `cat_np(Entity, sg|pl)` over `anaphor_sem`. A bare
demonstrative is anaphoric with no restrictor, so it takes `lexicon:anaphor : Entity` and not the
determiner's restrictor-typed `anaphor_of : ∀(A:Set) → A`.

Measured as a LAYER on `…-2026-10-24-identity`, cap-only, `--units-per-line`, same page:

| | baseline | all four | **`that` dropped** |
|---|---:|---:|---:|
| encoded | 20 | 19 | **20** |
| ambiguous | 399 | 393 | **399** |
| open | 138 | 158 | 142 |
| grammar-gap | **499** | 486 | **495** |
| total-skeletons | 3,542 | 3,895 | 3,639 |

With `that_pron` in, gaps fall 13 but SEVEN sentences go closed → open, every one of them
`That <noun> …` — `that` as a determiner, where the bare NP opens a spurious apposition. Dropping it
leaves `encoded` and `ambiguous` byte-identical to baseline: four gaps closed, nothing lost.

Of those four, **two are the target construction** and two close through the compound defect below.
The honest figure is two sentences, not four — and the aggregate counts did not show this. Only the
per-unit set diff did (`GRAMMAR-GAP → OPEN` ×13, `AMBIG → OPEN` ×6, `ENCODED → OPEN` ×1 for the
four-entry arm).

### 3. The adverb and the unsaturated copula — one decision, three symptoms

Blocked, deliberately. Modes cannot scope it: `category::unify_into` does not unify slash
modalities ("Rule licensing is enforced where rules fire, not here"), so an adverb typed
`((S\NP)/NP)\((S\NP)/NP)` would match any transitive verb's functor regardless of mode, licensing an
adverb between a verb and its object.

What it needs is a distinction the category system does not carry — and it is the third such case:

| site | distinction needed |
|---|---|
| the adverb modifier | the copula's `(S\NP)/NP` vs a transitive verb's |
| the identity copula's redundant derivation | definiteness (its own comment says so) |
| `named_compound`'s left operand | a pronominal NP vs a nominal one |

Each is individually patchable and each patch is a Band-Aid. One mechanism — a feature the category
system carries, or recasting these as rules that can hold guards — settles all three.

### Found on the way: `named_compound` accepts a pronoun

`named_compound` (D63 §8.13) matches `left_pat: Ctor("cat_np", [_, _])` — ANY `cat_np` — against a
`cat_n` head, building `Σx:C. compound(x, ⟦left⟧)`. It exists for named-entity compounds (`HeLa
cells`). But a referential pronoun is categorially a plain NP, as the closed-class file states, so
`it` matches. On the BASE snapshot with no layer applied:

```
«it scores» → cat_n(Σ__cmp_x:n05737153. compound(__cmp_x, $anaphor$3_3), pl)  prov=Compound
```

"a score compounded with ⟨whatever *it* refers to⟩". English has no such construction (`*it scores`;
the possessive `its scores` is a determiner, handled elsewhere), and `compound(x, d)` wants a
DESIGNATOR in `d` — an unresolved discourse referent designates nothing, so the restrictor
constrains nothing. The rule already restricts its left operand (`NotKindRaised`, `NotPpRefined`),
never for pronominality.

Pre-existing, and not the demonstratives' doing: the demonstrative layer only exposed it on three
more surfaces. It fires wherever a pronoun precedes a common noun, which for `it` / `they` is
constant, mostly adding a junk reading to sentences that parse anyway — and occasionally
manufacturing a parse, which is how it was found.

Fix: guards already read provenance (`Guard::NotKindRaised(op) => op.pick(…).prov != …`), and the
seed site that freshens `$anaphor$` already knows which items are pronouns. So a
`Combinator::Pronoun` provenance set there plus `Guard::NotPronoun(Operand::Left)` on
`named_compound`. A provenance tag guards the LEAF only; a pronoun compound modifier is always
consumed directly from the leaf, so it holds here, which is why this is a guard and not the general
mechanism §3 needs.

### The compound guard, measured — and what it did not fix

`Guard::NotPronoun(Operand::Left)` on `named_compound` (commit `90704bd4`). Measured on the BASE
snapshot, cap-only, same page: encoded 20, ambiguous 399, open 138, grammar-gap 499, total-readings
40,932 and the whole reading-count histogram **all byte-identical to baseline**; total-skeletons
3,542 → 3,385.

Nothing observable changes because `COMPOUND_STEP_PENALTY` already ranked these below the
`CLASSIFY_BUDGET` cut — they counted toward the skeleton pool without ever surviving as readings.
What the guard prevents is the case where the junk is the ONLY reading, which is what the
demonstrative entries hit.

**A recorded negative first.** Reusing `Guard::ProperName` looked right — its own doc claims it
"keeps close-naming apposition off a pronoun / bare-kind `cat_np(Entity)`". Measured: grammar-gap
499 → **531**, covered 419 → 390. The 32 losses are named-entity compounds built on OOV-augmented
DOC-LOCAL names, which are not concrete `EigonClass`es either, so the type test refused them for the
same reason it refused a pronoun. The type index cannot carry this distinction in this corpus.
Pronominality had to be tested directly, via provenance tagged at the seed site where a bare
pronoun's sem is still exactly `EigonAxiom(lexicon:anaphor)`.

**Re-measured with both** (demonstrative layer, `that` dropped, + the guard): encoded 20, ambiguous
399 — untouched — open 138 → 141, grammar-gap 499 → **496**.

| unit | before | after guard | what it is |
|---|---|---|---|
| bare `This is …` | gap | OPEN | the construction ✓ |
| bare `Those are …` | gap | OPEN | the construction ✓ |
| `These <noun> … those <noun> …` | OPEN | **gap** | compound misparse, correctly withdrawn |
| `<V> those <noun>` | OPEN | OPEN | **still a misparse** — see below |

The prediction was 497 and the result was 496, because that last unit had **two** junk routes, not
one. The guard killed its `compound(x, $anaphor$)` route; it then closed at cap 8 by a different
one — `λ__front_m. And(__front_m, …)(v01111816_i($anaphor$3_3))`, the head noun read as an
INTRANSITIVE VERB with the bare demonstrative as its subject. Killing one junk route for a
determiner-homograph surface does not kill the others.

**So the demonstrative entries are worth two sentences, not three or four.** Each narrowing step cut
the claim: 13 (all four forms) → 4 (`that` dropped) → 3 (compound guard) → 2 (route inspected). Only
the last two steps required looking at derivations rather than counts; the counts were consistent
with every one of the wrong figures.

## Track A: the cause census over all 94 blockers (2026-10-25)

The hand classification above grouped the blockers by what the sentences LOOK like. Three of its
rows were investigated with the instrument and all three turned out to be something other than their
label. So the census was redone on the measured cause: `probe_gap_boundary` over all 94 one-away
blockers, keyed on the verdict plus the two type-index-erased `cat_shape`s at the nearest miss.

`experiments/parsing/blockers.py` extracts the 94 (run log + `cnl-map.tsv`). It reproduces the
recorded breakdown exactly — 274 claim units, 51 / 94 / 55 / 34 / 40 — which is what licenses using
it as the input.

### What the parser is actually refusing

| verdict | sentences |
|---|---:|
| `WRONG-ARG` | 65 |
| `NO-FUNCTOR` | 7 |
| `FEATURE(type)` | 2 |
| `FEATURE(prep)` | 2 |
| `PRUNED` | **0** |
| `NORMAL-FORM` | **0** |
| `MODE` | **0** |

Three of those zeros matter. **No blocker is a pruning artifact** — the beam and sense cap are not
costing a single one of the 94, so widening them cannot help. **No Eisner normal-form guard and no
slash modality blocks any of them** either, which settles at scale what four separate mode
experiments found one at a time ("none has moved a single skeleton").

### The grouping is contaminated — conclusion withdrawn

| | |
|---|---:|
| sentences with a seam verdict | 76 |
| distinct cause groups | 68 |
| groups of 3 / 2 / 1 | 1 / 6 / **61** |

The first reading of this was "no dominant cause, so the grammar route is exhausted". **That does
not follow, and the key is the reason.**

A chart's maximal derivable spans are INFLATED BY MISPARSES. Each block grows as far as *some*
derivation reaches, including a wrong one, so the seam lands where spurious coverage ran out — not
where the missing rule is. Both sentences examined in detail today show it:

- `The claim is still the strongest evidence.` — the left block reached `cat_s(dcl, fin_any)`, a
  COMPLETE sentence, because `still` was read as the predicative adjective "motionless". The seam
  sat after `still`; the missing rule concerns a token the block had already swallowed.
- `Finding 9 reports those scores.` — the left block was a three-deep `compound_kind` pile reading
  `reports` as a noun.

Two for two. And the census key is (left block's `cat_shape` × right block's `cat_shape`) over
blocks spanning most of the sentence, so it encodes whichever accidental reading grew furthest —
close to a per-sentence hash of lexical ambiguity. 61 singletons is what that key produces whether
or not a dominant cause exists. **It measures the instrument, not the grammar.**

What survives is the verdict distribution above: zero `PRUNED`, zero `MODE`, zero `NORMAL-FORM` are
facts about the chart whatever the seam's position. So the beam, the modalities and the normal-form
guards are still ruled out for all 94. The three gaps diagnosed earlier also survive — each was
established by TRACING a derivation, never by the grouping.

### What the bottom-up instrument can and cannot do

It localizes a seam, which is a SYMPTOM. Grammar engineering works the other way: hold a sentence's
intended analysis and ask which step of that derivation the grammar fails to license. That is how
CCGbank was built — the treebank-to-CCG conversion's FAILURES are the inventory of what is missing —
and it is the DELPH-IN/`[incr tsdb()]` loop. The diagnosis it yields is keyed on the construction
("this token needs a category of shape X"), which groups across sentences; a seam key does not.

The cheap approximation, needing no gold trees: **token-level category ablation.** For each token
*i*, inject a permissive category and re-parse; if the sentence closes, *i* is implicated, and the
category the completed derivation demanded at *i* names the missing entry directly. Cost is *n*
re-parses per sentence. The exact version infers the required category at *i* by unification from
the surrounding chart instead of guessing it.

Until that is run, the size of the remaining grammar work is UNKNOWN — not zero, and not the 3 the
contaminated key suggested.

### Top-down: the token-ablation probe, and the re-test

The seam key was contaminated, so the census was re-run on a causal key. `probe_token_ablation`
deletes one token at a time and re-parses: if the sentence closes without token *i*, then *i* is
what the grammar could not use. That names a WORD rather than a chart position, which is the
property the seam key structurally could not have.

Validated on five sentences whose answers were already known from tracing:

| sentence | seam probe | ablation |
|---|---|---|
| `…is still the strongest evidence.` | seam AFTER `still` | **3:still** |
| `…is also the strongest evidence.` | seam AFTER `also` | **3:also** |
| `The claim is very strong.` | — | **3:very** |
| `This is the gene.` | cut at 1 | **none** — no deletion rescues it |
| `…is that it behaves…` | cut at 4 | **none** |

The three adverb sentences land in ONE ablation group where the seam gave three. The two negative
controls return nothing rather than inventing a repair, which is what makes the positives readable.

### The re-test gives the same answer

Over the 94 blockers:

| implicated tokens | sentences |
|---:|---:|
| **0** | **39** |
| 1 | 36 |
| 2 | 17 |
| 3 | 2 |

**39 of 94 have no single token at fault** — those need a RULE, not a lexical entry, and rules have
measured at 0–2 units apiece across five previous constructions.

The other 55 implicate 76 tokens over 54 distinct surfaces. The top surface is `the` ×7 — and it is
an ARTIFACT. Inspecting those seven: in five, `the` is implicated alongside a more plausible culprit
(`slightly`, `Only`, an OOV hyphenated compound), and deleting ANY determiner gives its noun a
bare-NP reading with far more flexible categories. Determiner deletion is a generic repair, the same
kind of systematic bias the seam key had. Removing the 8 determiner tokens:

| group size | count |
|---:|---:|
| 3 | 3 (`both`, `not`, `through`) |
| 2 | 10 |
| **1** | **39** |

**The largest defensible group is 3** — the same figure the contaminated seam census produced, now
from an independent key with a causal basis. Two instruments with different biases, different
failure modes, and the same answer.

So the conclusion stands and the earlier reasoning for it does not. The grammar route on this corpus
offers groups of 3 at best, with 41% of blockers needing rules rather than entries. What was wrong
was asserting it from the seam key; the right order was to build this probe first.

Both probes share a lesson worth keeping: each had a systematic bias that produced a confident
top signal — misparse-inflated blocks for the seam, generic determiner repair for the ablation —
and in both cases the bias was found by INSPECTING the top group rather than by reading the counts.

## Basic grammatical coverage, measured against core-en's families (2026-10-25)

Everything above ranks fixes by XIAP unit-yield, which is a property of ONE DOCUMENT. A construction
worth 1 unit there can still be core English, and that ranking cannot see it. core-en is the right
yardstick — 54 families, ~45 of them syntactic (the rest are intonation and punctuation) — and it
had never been measured against.

`experiments/parsing/battery/core-en-families.txt` is one minimal sentence per family, invented
vocabulary so it is committable and OOV-free. Run without page augmentation: this measures the
GRAMMAR, not a document's glossary.

**38 of 50 parse. 11 fail** (plus one OOV).

| construction | measured cause |
|---|---|
| NP coordination | fails **iff a conjunct is DEFINITE** |
| fronted subordinate clause (`Because X, Y`) | the fronted POSITION; comma irrelevant |
| fronted adverb (`Slowly, X`) | the fronted POSITION; comma irrelevant |
| `very` + predicative adjective | degree works ATTRIBUTIVELY, not predicatively |
| there-existential | missing (`There is a gene.`, `There are genes.`) |
| free relative, subject and object | missing |
| zero object relative | missing — core-en HAS it (`rrel`); only the subject-gap half was taken |
| to-infinitive complement | missing |
| `'s` possessive | OOV — the clitic is not lexicalized |
| superlative `most` | missing |

### Definiteness is a root cause with four symptoms

The coordination row is the one that changes the work order. One-variable grid:

| | |
|---|---|
| `A gene and a protein affect cells.` | **parses** |
| `Genes and proteins affect cells.` | **parses** |
| `The gene and the protein affect cells.` | fails |
| `The gene and the protein affects cells.` | fails — so NOT number agreement |
| `The gene and proteins …` / `Genes and the protein …` | fail — one definite suffices |

Indefinites coordinate, bare plurals coordinate, definites do not. That is the same impoverishment
found three other ways today: a definite has only TYPE-RAISED forms and no plain `cat_np`. Its
symptoms so far:

1. the identity copula's redundant derivation (needs definiteness to narrow — its own comment)
2. the adverb/copula adjacency (the raised definite selects an adjacent `VP/NP`)
3. `UnaryKind::IdentityPredicate` never firing (definites arrive raised)
4. **basic NP coordination**

Earlier in this session the definiteness feature was priced at "≈148 sites, ZERO measured coverage
value" — on the evidence then available, which was the identity copula's redundancy alone. That
pricing is **withdrawn**. It is the root of a basic-grammar failure, and the ≈148 sites buy four
symptoms rather than a ranking improvement.

### What this reorders

A construction that fails here fails in EVERY document, whereas a XIAP one-away singleton may never
recur. So basic-grammar coverage is the work order, and corpus unit-yield is a progress metric, not
a priority metric. The CNL rewrite rides on top of this: a CNL shaped to avoid `There is …`,
fronted `Because …`, free relatives and definite coordination is a CNL shaped by the parser's
defects rather than by what the domain needs to express.

The battery is also the artifact the top-down method needs — 50 sentences whose intended analysis is
known BY CONSTRUCTION, which is what neither probe built today had.

## The plain definite NP: two entries, +8 units (2026-10-25)

`ontology:the : forall (A : Set) => A`, so `the(T) : T`; and `⟦cat_np(T, num)⟧ = T`. A plain definite
NP is therefore already type-correct and expressible with the EXISTING dependent-determiner
machinery — `cat_forall(num, λT. cat_np(T, num))` over `λA. the(A)`, consumed by `CombKind::DepApply`
exactly as the raised forms are. Two lexical entries, no kernel change, no arity change, against the
≈148 sites an arity-changing definiteness FEATURE would have touched.

Measured as a layer on `…-2026-10-24-identity` (`experiments/parsing/probes/definite-plain-np.esl`):

| | base | + plain definite NP |
|---|---:|---:|
| battery (89 basic constructions) | 70 | **72** |
| page grammar-gap | 499 | **477** |
| page covered (E+A) | 419 | **439** |
| **source units fully covered** | **51 / 274** | **59 / 274** |
| one-away blockers | 94 | 87 |
| total-skeletons | 3,542 | 5,078 |

Page transitions: 17 `GRAMMAR-GAP → AMBIG`, 5 `GRAMMAR-GAP → OPEN`, 3 `OPEN → AMBIG` (an
improvement — open became closed), 3 `ENCODED → AMBIG`. **Nothing lost coverage.**

Two basic-grammar failures close: NP coordination with definite conjuncts, and — unpredicted — the
**zero object relative**, a core-en family (`rrel`) this note had listed as missing because only the
subject-gap half was taken. It needed a plain definite NP to build the gapped clause.

### The cost is derivational, not semantic

The 3 `ENCODED → AMBIG` sentences all begin `The <noun>`, so the subject now derives two ways. The
sems are IDENTICAL as printed:

```
base    closed=1   [0] has_quantity(the(n13759014), <term>, mk_quantity(<term>, 0))
+plain  closed=2   [0] has_quantity(the(n13759014), <term>, mk_quantity(<term>, 0))
                   [1] has_quantity(the(n13759014), <term>, mk_quantity(<term>, 0))
```

So this is the identity copula's bargain again — "logically equivalent, not a wrong reading; the
cost is ranking and chart size, never a distorted claim" — and here the readings are not merely
equivalent but textually identical.

**Open, and it decides whether the +43% skeleton cost is removable.** `paths.rs` already runs
`retain_distinct_sems` over the candidates, so two identical sems should not both survive. Either
they differ in subterms `pretty_term` elides as `<term>`, or the dedup is not catching them. If the
latter, the skeleton growth is a DEDUP DEFECT and disappears when fixed; if the former, the
duplicates are genuinely distinct terms and want an ENF guard of their own. Settle this before the
entries go into `closed-class.esl`.

### What it says about the work order

The five constructions this note worked through earlier — the reduced relative, `rather than`, the
closed-class batch, Fork A, the `ger` participial — moved the page **16 → 19 of 325 across all
five**. Two entries fixing BASIC grammar moved units **51 → 59**.

That is the case for ranking by basic-grammar coverage rather than by corpus unit-yield, measured
rather than argued. A corpus-specific patch is worth 0–2 units because the pattern occurs 0–2 times;
a basic construction recurs, and definiteness was sitting underneath coordination, the zero object
relative, and the identity copula's redundancy at once.

### Correction

Definiteness was described earlier today as "a root cause with four symptoms". The plain NP fixes
coordination and the zero object relative but **not** the adverb/copula case — that one's cause is
the adverb's own category, established separately. Three symptoms, not four; the fourth was folded
in on resemblance rather than evidence.

## Three instruments that were saying something untrue (2026-10-25)

Chasing the plain definite NP's skeleton cost turned up three defects in the measuring apparatus
itself. In two of the three, something downstream had built on the untruth rather than around it.

### 1. `subsume_duplicates` keyed on category SYNTAX

«The cutoff was 5.5 Å.» returned two readings whose sems were **byte-identical** — 321 bytes each,
by `format!("{:?}", …)`, the exact key the dedup uses:

```
cat[0] = cat_s(dcl, fin)       prov: BackwardApp     (plain NP, backward application)
cat[1] = cat_s(dcl, fin_any)   prov: ForwardApp      (type-raised subject, forward application)
```

`⟦cat_s(dcl, fin)⟧` and `⟦cat_s(dcl, fin_any)⟧` are both `Prop`, so these are one reading. The key
is now the category's DENOTATION. `retain_distinct_sems` upstream cannot catch it: it runs on
PRE-felicity sems, and the two converge only once felicity normalises them.

Measured. On the BASE grammar the defect is nearly dormant — readings 40,932 → 40,911, skeletons
unchanged, every outcome identical — because it needs a SECOND ROUTE to the same proposition and the
base grammar rarely offers one. With the plain definite NP it bites: readings 42,911 → 40,146,
one `ENCODED` recovered.

**Prediction corrected:** the +50% skeleton growth was expected to be mostly duplication and shrink.
Skeletons did not move at all (5,078 either way). Duplicate readings SHARE a skeleton, so collapsing
them cannot reduce the count — the growth is genuine new sem shapes from having a second NP form,
and it is inherent to the fix.

Net for definiteness + dedup against baseline: grammar-gap 499 → 477, covered 419 → 439, units
51 → 59, and **786 FEWER total readings than baseline**. More coverage at lower reading cost.

### 2. `pretty_term` elided nine `Exp` variants as `<term>`

`Exp` has 46 variants; the printer special-cased 37 and sent the rest to a bounded label, `<term>`.
`LitRat`, `LitUnit` and `Refine` were among them — so every measured-quantity sem read this session
was partly invisible, and the two readings above printed identically only because the parts that
could differ were the parts being erased. `LitRat` and `LitUnit` now print their contents (a
rational and a unit are small, so the bounded-output argument never reached them); `Refine` prints
its base; the remaining five are named rather than left to the generic label.

**A test depended on the defect.** `value_first` in `quantities_in_the_parser.rs` searched for the
literal string `{rel}(<term>, ` — hardcoding the elision. Six tests failed on the printer fix, and
the first reading of that red suite ("the dedup change removed a reading") was wrong: the failures
were the printer's. The helper now skips the unit argument generically; its assertion is unchanged.

### 3. The measurement banner misnamed its own input

`=== WRN first page over FULL lexicon: …` was HARDCODED. It stopped being true the moment `--page`
existed, so every run in `experiments/parsing/results/` — including every measurement in this note —
is labelled a WRN measurement whatever corpus it actually read. It now names the page
(`=== cnl-page.txt over FULL lexicon: …`).

**Two consumers parsed the old string**: `scripts/eval-parse-rate.sh`, which documents it as a
contract ("`grammar-gap` MUST come from the … summary line"), and `experiments/parsing/
uab-xiap-score.py`. Both patterns now match either spelling, verified against an archived log, so
old runs still parse.

### The shape of all three

Each was a tool reporting something untrue about its own output, and none surfaced from READING
output — only from checking what the output was made of. Two had downstream consumers built on the
untruth, which is what makes this class expensive: the defect propagates into things that look like
independent confirmation.

## Fronted subordinators: +3 basic constructions, +0 units (2026-10-26)

Every committed subordinator (`if`, `because`, `although`, `while`, `though`, `whilst`, `unless`,
`whereas`) is `(S\S)/S` — it takes its clause on the RIGHT, yielding `S\S`, which attaches to a
matrix clause on its LEFT. So «The study failed because the gene affects cells.» parses and
«Because the gene affects cells, the study failed.» does not. Adding the comma does not help: it is
the fronted POSITION.

The fronted category is `(S/S)/S`, and `S/S` is exactly what `category::is_sentence_premodifier`
names and what the `AbsorbComma` fronted-modifier comma already consumes. **The machinery was in
place; only the entries were missing.** The sem needs no change either — `because_sem` is
`λs2. λs1. Because(s1, s2, …)`, taking the SUBORDINATE clause first in both orders — so each fronted
entry reuses the committed sem verbatim and only the outer slash flips `bwd` → `fwd`.

Eight entries (`experiments/parsing/probes/fronted-subordinators.esl`). `but` is excluded — «But Y,
X» is a discourse connective, not a fronted subordinator. `when` has no subordinator entry at all
AND no temporal relation to give one a sem, so «When X, Y» stays a separate gap rather than being
closed by inventing semantics.

Measured, stacked on the plain definite NP:

| | before | after |
|---|---:|---:|
| battery (89 basic constructions) | 72 | **75** |
| XIAP page — every metric | — | **byte-identical** |
| source units | 59 | **59** |

Fronted and comma-less both parse, and both postposed controls still parse. Battery set diff: three
sentences gap → PARSES, nothing else moved.

**The page result is zero, and that is the point.** Readings are identical too, so the entries never
fire: this document never fronts a subordinate clause. Compare the two fixes:

| fix | battery | units |
|---|---:|---:|
| plain definite NP | +2 | **+8** |
| fronted subordinators | **+3** | 0 |

Both are correct basic English. Only one matters to this corpus. Under the old yield-ranking the
second would never have been built — and a CNL rewrite steering around fronted `Because …` would
have been steering around a parser defect rather than a domain constraint. That is the whole case
for ranking by basic-grammar coverage and treating corpus yield as a progress metric.

### Remaining battery failures (14)

to-infinitive complement (×2: `wants to`, ECM `showed … to be`) · free relative (×2: subject,
object) · there-existential · `or`-coordination with definites (`and` now works) · perfect passive
(`has been`) · gerund subject · `Neither gene …` · `'s` possessive (OOV) · predicative `very` ·
superlative `most` · fronted adverb · `when`.
