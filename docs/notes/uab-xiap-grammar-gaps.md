# Parse success and the shape of the gaps — survey §4 step 3

**Measured `2026-10-11`**, store `wordnet-umls-hpo-aligned-2026-10-10-d101`, release,
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

## Order of work

By units unblocked per unit of work, on these measurements:

1. **The reduced relative + PP** (`queried in the study`, `used in the study`) and the
   **participial post-modifier with an object** (`carrying the variant`). The adverb form already
   parses, so the rule exists and its PP/object extension is missing. These appear in the verbless-NP
   units, in NP subjects, and inside long sentences.
2. **The comma appositive.** Two probes, and it is pervasive in the annotation and caption lines.
3. **`cannot`**, once the split-vs-reseed decision is made. One token, 7 uses.
4. **The clause splice**, as part of step 4's CNL rewrite rather than a lexer change: measured at
   11 of 145 pieces recovered, so it is worth doing and will not be the step that closes the gate.
5. **`rather than` over participles**, and the **demonstrative partitive**. One probe each.

Nothing above the lexicon is in doubt: `missing-lexeme` is 0 over the corpus, and the one lexical
gap the battery found (`cannot`) is a closed-class omission rather than a content word.
