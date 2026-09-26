# D95 — Quantities in the tokenizer and parser pipeline

**Written** `2026-09-20`, against `main` at `1efd6bb`. **First draft.** The consuming side of
**D93** (units of measure), which specifies the kernel representation and states the obligations
this document works out. Depends on D93 for `units:Quantity` and on **D94** for exact magnitudes.

## The gap — measured, not inferred

Running `dcg::segment::tokenize` and `is_nonprose` over real corpus strings gives this. `[OUT]`
marks a token `is_nonprose` classifies as non-prose — which, as the next section shows, does **not**
remove it from the parse.

| input | tokens |
|---|---|
| `incubated at 37 °C for 1 h` | `incubated` `at` `37[OUT]` **`C`** `for` `1[OUT]` `h` |
| `spun at 931g for 2 h at 30 °C` | `spun` `at` `931g[OUT]` `for` `2[OUT]` `h` `at` `30[OUT]` **`C`** |
| `a dose of 5 mg/dL` | `a` `dose` `of` `5[OUT]` `mg` `dL` |
| `log2(copy number) < -1` | `log2` **`1[OUT]`** |
| `P = 4.2 × 10⁻¹³` | `P` `4.2[OUT]` `10⁻¹³[OUT]` |
| `20–30% and 0.56-fold` | `20[OUT]` `30[OUT]` `and` `0.56-fold[OUT]` |
| `45-60% of such cancers` | `45-60[OUT]` `of` `such` `cancers` |

**The failure is corruption, not omission.** Three of these produce a *wrong token* rather than a
missing one, which is the worse class of defect:

- **`°C` becomes `C`.** Edge-trimming (`trim_matches(|c| !c.is_alphanumeric())`) strips the degree
  sign, and `C` is a perfectly good lexeme — carbon, cytosine, the letter. A unit does not go
  missing; it silently becomes a different word the lexicon will happily resolve.
- **`< -1` becomes `1`.** The comparison and the **sign** are both destroyed. The MMR definition
  D93 depends on (`log2(copy number) < −1`) would arrive as the number one — not as a gap, as an
  assertion with the opposite meaning.
- **`log2(copy number)` loses its argument.** `strip_bracketed_asides` removes `(…)` as a
  parenthetical gloss, which is right for `microsatellite instability (MSI)` and wrong for function
  application. **The parenthesis has two meanings and the tokenizer assumes one.**

**The dash handling is wrong on one case, not inverted on both.** `20–30%` (en-dash) splits into two
tokens; `45-60%` (hyphen) survives joined. Splitting the en-dash range is the defect. Keeping
`45-60` together is *correct* — and it is also a counterexample to the tidy convention an earlier
draft asserted here, since `45-60% of such cancers` is a **hyphenated range**, not a catalogue
number. The corpus does use the en-dash for most ranges and the hyphen for part numbers
(`926-68021`, LI-COR), but the two are not cleanly separated by the dash character, and no earlier
document established that they were.

**What routes out cleanly**, and is therefore the easy half: bare numerals (`37`, `0.56`), and
numeral-initial compounds (`931g`, `10⁻¹³`), which survive as single tokens and are then dropped by
`is_nonprose`.

**Numerals reach the parser and seed nothing, which is worse than being routed out.** An earlier
draft said `is_nonprose` and `enc:unit_kind` route numerals away from the DCG. Neither does:

- Both CKY drivers call `seed_leaves(&tokenize(text), …)` on the **full** token stream
  (`kernel/src/dcg/parse/paths.rs:79,87,257,267`). Nothing filters.
- `is_nonprose` has two non-test callers, both inside `all_prose_tokens_known`
  (`kernel/src/dcg/parse/mod.rs:954-958`) — a lexical-coverage probe gating widen-on-failure. It
  removes no token.
- `enc:unit_kind` is a property of a whole `enc:DiscourseUnit`, not a token, and the encoder
  hard-codes every unit to `kind_prose` (`crates/eigenius-encoding/src/emit.rs:649`).

The real behaviour is a worse failure. A numeral seeds **zero items** into a chart indexed by
tokens, so its span cannot be covered and the sentence cannot parse at all. And because
`is_nonprose` reports it as non-prose, the coverage probe finds no OOV, so the widen ladder runs to
exhaustion on a sentence that could never have parsed.

One consequence for the design below: relocating `is_nonprose` into a preprocessor changes what the
widen gate sees, so that relocation is **not** behaviour-preserving and must be scoped as a change.

**Unit symbols are OOV, which breaks a gate.** `ml`, `mg`, `dL`, `h` and `ng` contain letters, so
`is_nonprose` passes them through as lexemes, and `grep` over `ontologies/lexicon/` finds no entry
for any of them. `missing_lexeme == 0` is a tracked gate (`experiments/parsing/baseline.json`), so
units cannot simply be let through. The CNL corpus passes today only because it avoids units.

## The shape: a quantity is seeded, not looked up

`Parser::seed_leaves` is "the single entry point the packed forest and the flat beamed chart both
build their leaf cells from", and it already constructs items for things with **no lexical entry of
their own** — the productive morphology (`-ly` adverbs, denominal and prefixed adjectives) and the
leaf unary shifts.

That is a quantity's shape exactly. `37 °C` is not a lexeme and should never be one; enumerating
prefix × unit as lexical entries is 24 × 29 before compounds. What has entries is the **unit
symbol**; the quantity is *constructed* at seed time from a recognised span.

So the change is: recognise a quantity span, parse its unit sub-expression, build one item, seed it.

## A numeral-adjacent unit is the inverse of multiword seeding, and harder in one respect

Multiword seeding **joins**: `cell line` is N surface tokens with a lexicon entry for the joined
span, and `seed_leaves` emits the joined item *alongside* the split ones, "carried as competing
chart edges, not resolved here."

A numeral-adjacent unit **splits**: `5mg` is one surface token that must yield two constituents.
Same lattice, opposite direction — but **not the same mechanism**, and an earlier draft claimed it
was. The chart is `vec![vec![Vec::new(); n]; n]` with `n = tokens.len()`
(`kernel/src/dcg/parse/seed.rs:568-581`). A joined span is a union of cells that already exist; a
split needs *more positions than there are tokens*, and there is no cell to seed into. So the split
must happen where `n` is still being decided — in the preprocessor, which is where this document's
own design diagram puts it — and not in seeding.

**The asymmetry that makes splitting harder: the boundary is not given.** An MWE chooses among
boundaries the tokenizer already produced. A split must *propose* one, and the proposal space is
open — `5mg` divides 1|2, `10x` 2|1, and `5-fold` and `53BP1` should not divide at all.

**The unit vocabulary is what makes it decidable.** A quantity split is
`⟨numeral⟩⟨known unit symbol⟩`, so the boundary is "the longest suffix that is a unit" — a lookup,
not a search:

| token | prefix | suffix | split? |
|---|---|---|---|
| `5mg` | `5` numeral | `mg` is a unit | **yes** |
| `931g` | `931` numeral | a bare `g` | **no — refused in v1**, see below |
| `53BP1` | `53` numeral | `BP1` is not a unit | no |
| `HEK293T` | `HEK293` not a numeral | (`T` is tesla) | no |
| `5-fold` | `5` numeral | `-fold` is not a unit | no |
| `2-2` | `2` numeral | `-2` is not a unit | no |

Both halves of the test are load-bearing. Without the numeral-prefix test, `HEK293T` would split on
tesla; without the unit-suffix test, `53BP1` would split on nothing.

So the unit vocabulary is not only what D93 needs for typing — it is what makes this tokenization
decidable at all. Before units exist, `931g` is unanalysable.

**A bare `g` is not split in v1** (D93, "A vocabulary hazard the same evidence surfaced"). It has two
readings, gram and standard gravity, and D93's vocabulary holds only the first: admitting standard
gravity opens "units science uses that the SI does not accept", a category to be opened with a
criterion rather than one symbol at a time. Splitting could therefore only produce grams, and in
this corpus neither bare-`g` token is a mass:

| token | in the WRN methods | what it is |
|---|---|---|
| `931g` | "the plates were spun at 931g for 2 h at 30 °C" | g-force |
| `2g` | "(Fig. 2g)" | a figure panel |

So a numeral with a bare `g` suffix stays one token and is routed out as non-prose — today's
behaviour, and the CNL guide's R2: *a faithful un-parsed claim beats a parsed distorted one*. A
PREFIXED gram splits: standard gravity takes no prefix, so `mg`, `μg` and `kg` are unambiguously
mass. When standard gravity is admitted, `931g` splits into competing readings as "Ambiguous unit
symbols" describes.

**Figure panels are the same trap, wider than `g`.** The rule reads the unbracketed `Fig. 2d` in the
WRN methods as two days, and would read `Fig. 2h` as two hours; panels `a`, `c`, `e` and `f` escape
only because a prefix alone is not a unit. The nine bracketed references, `(Fig. 2g)` among them,
are removed by the aside rule before the split runs; the unbracketed ones reach it. See "Open
questions".

## An existing defect the same change repairs

`is_nonprose` routes out any token starting with a digit. Measured:

| token | routed out? | correct? |
|---|---|---|
| `53BP1` | **yes** | **no** — a gene, 12 occurrences in the Letter body |
| `HEK293T` | no | yes — a cell line |
| `931g` | yes | yes, and still in v1: a bare `g` suffix is not split |
| `5-fold` | yes | arguably not — a degree modifier |

The rule's own docstring reasons about letter-initial gene symbols ("`mlh1`, `msh2`, `brca1`,
`parp` start with a letter and are NOT non-prose") and does not consider digit-initial ones.
`53BP1` is the counterexample, and it is not marginal: it is one of the two DSB markers the paper's
central mechanism rests on.

Admitting numerals for quantities requires revising this rule anyway, and a revision keyed on what
the token *is* — numeral, quantity, or nomenclature symbol — admits `53BP1` as the symbol it is.
The defect and the feature are one change.

**One honest caveat:** admitting `53BP1` only helps if the lexicon resolves it. If it does not, the
token moves from silently dropped to counted as a missing lexeme — which is worse for the gate and
better for the truth, and is the right direction on this project's own terms.

## One chart item per candidate reading

The recognised span becomes leaf items carrying a magnitude and, where present, a unit — **one per
candidate unit sense**, as competing edges. An unambiguous symbol yields exactly one; a symbol with
several senses yields one each and lets the ranker choose (see "Ambiguous unit symbols" below). A bare
`0.56` is the same item shape with no unit, which keeps cardinality and quantities on one path
rather than building two mechanisms that must later agree.

**One guard neither document mentioned.** `cat_has_selectional_slot` routes a grammar with any
index-dependent argument slot to the unpacked CKY path, because node-level packing by `cat_shape`
erases the index and would be unsound. Its test is `matches!(ty, Exp::EigonClass(iri) if iri !=
ENTITY_TOP_IRI)` (`kernel/src/dcg/category.rs:466-475`) — and a quantity index is an *application*,
not an `EigonClass`, so the guard returns `false` and the packed path is taken *with the index
erased*. Extending the guard is in scope, or quantity slots must be type variables.

**Its category is an open fork, and the composition rules change either way.** An earlier draft of
this document claimed a quantity is an NP at a quantity type, `cat_np(units:Quantity(u), n)`,
denoting the `Quantity u` itself (D93), and that composition therefore needed no change. The second
half is false: every PP category in the lexicon fixes its object to `Entity` (see "Settled" below),
so a quantity does not compose with a preposition today whatever category it is given.

The fork is whether a quantity is an NP at a quantity type or a distinct category:

- **`cat_np(Quantity(u), n)`** adds no constructor, and the existing NP machinery — coordination,
  determiner-free argument positions — applies unchanged. `37 °C is the optimal temperature` wants
  this.
- **A distinct `MP`** is what the measure-phrase literature uses uniformly (Haruta et al.'s primitive
  `D`; Schwarzschild's `⟨d,t⟩` for the interval reading), and two of the corpus constructions demand
  it: `rose 5 °C` takes the verb to `(S\NP)/MP`, and prenominal `a 5 °C increase` is `N/N`. Neither
  position admits an NP.

Both distributions are real. Schwarzschild has the measure phrase base-generated as `⟨d,t⟩` and
reaching its position by a lexically governed type-shift, which would make the seeded item the
measure phrase and `cat_np(Quantity(u), n)` what a shift produces. **For this architecture,
subcategorisation is the better answer** — see "Consumers subcategorise; they do not shift" below.
The seeded item is the measure phrase either way; what changes is whether a consumer selects it
directly or a unary rule converts it.

## The design: separate lexing from preprocessing

The tokenizer today **conflates two jobs**. `strip_bracketed_asides` is a decision — drop glosses.
Edge-trimming is a decision — discard punctuation. Both are fused into the lexer, which is why they
are irreversible and why `°C` becomes `C` with no way to recover it. Meanwhile a third decision,
`is_nonprose`, is applied by the *consumer* (`parse/mod.rs:957`) rather than by any stage.

So token-stream decisions are already being made. They are simply scattered, with no stage owning
them.

```
today:     text → [lex + decide, fused] → tokens → [is_nonprose filter at the consumer]
D95:       text → [lex: total, lossless] → token stream → [preprocess: every decision] → items
```

**The lexer becomes trivial and total.** It emits every character in some token, each carrying its
source offset and a class — word, numeral, symbol, punctuation. Nothing is deleted, so nothing is
unrecoverable.

**The preprocessor owns every decision**, including the ones that exist today:

- drop bracketed asides — but now able to ask what precedes the `(`, so `instability (MSI)` is a
  gloss and `log2(copy number)` is an argument;
- discard punctuation the grammar does not consume;
- classify a token as non-prose — the `is_nonprose` rule, relocated from the consumer and revised
  so `53BP1` is a symbol rather than a numeral;
- **merge quantity spans** — `37` `°` `C` into one item, `5mg` split on the numeral/unit boundary
  (a bare `g` suffix, as in `931g`, is not split in v1),
  `<` `−1` into a comparison — which is the new work.

**The aim is behaviour preservation, and it is a rewrite rather than something got "by
construction".** Where no new rule fires the preprocessor should reproduce what the fused lexer did,
but that is an obligation on the implementation, not a property of the decomposition. Two things
make it harder than an earlier draft allowed:

- **Classing is segmentation.** The lexer cannot be decision-free: `0.56-fold` survives as one token
  only because `-` is not a separator, and `20–30%` splits only because `–` is
  (`kernel/src/dcg/segment.rs:150-155`). So the lexer still owns segmentation, and the preprocessor
  does not own every decision.
- **The duty list above is incomplete.** Separator-to-space substitution, edge trimming, the comma's
  special handling, dangling-comma removal and run collapsing (`segment.rs:169-180`) are all
  decisions and all unlisted.

**A differential test is evidence, not a proof.** `tokenize(s) == preprocess(lex(s))` over a corpus
is finite, the two sides have different types (a `Vec<String>` against items), so the comparison
needs a projection that is unspecified — and it is undefined whether the revised `is_nonprose` and
the revised aside rule count as "quantity rules". Disable them and the test validates a
configuration that never ships; leave them on and the equality fails by design. The test is worth
having and it is not an equivalence proof.

**Why this beats protecting quantities from a destructive lexer.** The alternative — recognise
quantity spans on raw text and substitute sentinels that survive the destructive steps — leaves the
lexer destructive for everything unprotected, so the protection set grows without a natural
boundary. `log2(copy number) < −1` shows it: protecting the threshold `< −1` still loses the
function's argument, because a function application is not a quantity. Each further case is another
rule in a recogniser that works on raw text, where it has no token boundaries to lean on.

## The unit sub-grammar is not the English grammar

`mg/dL` has its own syntax, which is what UCUM specifies, and it is not English. Parsing it with
the DCG would be a category error — and would force the prefix × unit enumeration the seeding
approach exists to avoid.

So a **separate unit parser** runs on the unit portion of a recognised span, once **per candidate
sense**, returning a `Unit` term for each. It is small, closed and total: a finite symbol set,
products, quotients and rational powers. The DCG never sees it; it sees the resulting quantity
items as competing edges.

**Its surface syntax must accept the inverse-exponent form.** The corpus writes concentrations as
`μg ml⁻¹` and `ng ml⁻¹` (13 occurrences), not `μg/mL`. That is the "rational powers" clause already
above — `ml⁻¹` is `ml` to the power −1 — so it needs no new mechanism, only the superscript spelling
in the symbol table alongside `/` and `^-1`. The lexer deletes nothing (see Scope), so the exponent
reaches the sub-parser intact by construction.

## Ambiguous unit symbols are polysemy, not a special case

`M` is molar or mega. `h` is hour, and in another register the Planck constant. `931g` is g-force
where `10 g` is grams.

**In v1 the `g` case does not arise.** Standard gravity is not in D93's vocabulary and a bare `g`
suffix is not split, so `931g` never reaches seeding (see the split section above). What follows is
the mechanism for a symbol with several senses — and for `g`, once standard gravity is admitted.

A unit symbol is a **lexeme carrying several senses**, exactly as a noun carries several synsets,
and each sense denotes a different unit. It therefore routes through machinery that already exists:
the sense cap and contextual `SenseRanker`, the felicity gate, and reading selection. A wrong unit
sense is refused or down-ranked by the same path that refuses a wrong noun sense.

**Resolved: one item per candidate sense.** An earlier draft contradicted itself — one section had
the sub-parser "return a `Unit` term" at recognition time, another said resolving early "would put
unit disambiguation outside the ranker". Both cannot hold, since a `Unit` returned at recognition
decides gram-versus-g-force before the `SenseRanker` ever runs.

A quantity span therefore seeds **one item per candidate unit sense**, as competing chart edges —
the same shape as a polysemous noun, and the same shape `seed_leaves` already uses for the
MWE-versus-compositional ambiguity it carries rather than resolves.

Three consequences, which supersede what the sections above say:

- **The recogniser commits to a span, not to a unit.** It marks the quantity span and yields the
  candidate senses; it does not choose among them.
- **The unit sub-parser runs per sense**, producing one `Unit` term per candidate. For an
  unambiguous symbol that is a single item and nothing is lost.
- **Normalisation happens after selection, not at recognition.** D93's "the parser emits both the
  stated and the normalised form" is true of the *selected* reading, so the normaliser runs once the
  chart has committed, not while it is being seeded.

This is how the `g` hazard is handled once standard gravity is admitted: gram and standard gravity
compete as chart edges, the ranker scores them in context, and the felicity gate refuses a reading
that does not compose — rather than a pre-pass silently picking one. Until then v1 refuses the
split, the fail-closed half of the same choice.

## What the pipeline stages inherit

- **Stage A (preprocess / glossary).** Unchanged. Unit symbols are not document-scoped
  abbreviations; they belong to the base lexicon.
- **Stage B (parse).** Seeding gains quantity items; the unit sub-parser is invoked here; the
  sense ranker sees unit senses alongside word senses.
- **Stage C (resolve).** Unchanged — a quantity carries no referent hole.
- **The felicity gate.** Unchanged in mechanism: it checks the assembled sem against `⟦cat⟧`, and a
  quantity NP's `⟦cat⟧` is `units:Quantity(u)`. A mis-composed quantity fails there like anything
  else.

## Consequences for the measurement

`missing_lexeme == 0` and `grammar_gap == 0` are tracked gates, and the tracked corpus is the
hand-authored CNL page, which avoids units. Admitting quantities therefore changes what the corpus
*can* contain before it changes any number — the baseline must be re-established on a corpus that
exercises quantities, or the gates continue to certify a register that excludes them.

The methods material is the natural corpus: of 240 sentences, nine carry two distinct units and one
carries three (D93). It is also where the parser is weakest, so the two should not be conflated —
a quantity gap and a syntax gap must be distinguishable in the report.

## The CNL guide needs revising, not extending

`docs/method/controlled-english-style-guide.md`'s first DON'T instructs authors to drop inline
numbers: "the parser routes non-prose out; numbers are dropped, so a numeric claim is lost… state
the qualitative claim". Once quantities parse, that is wrong for quantities while remaining correct
for test statistics (`P = 4.2 × 10⁻¹³` routes to a D52 record, not into the claim).

The guide anticipates this — "expected to drift as the grammar grows; check a claim against the
baseline before relying on it" — but the fix is a rewritten rule that distinguishes a *measured
quantity* (now in the claim) from a *test statistic* (still out of it), not a new bullet.

## Settled: the category of a quantity and its consumers

Four categories in `ontologies/lexicon/lexicon-ontology.esl` fix their object type to `Entity`:

| category | denotation | line |
|---|---|---|
| `cat_pp_arg(Prep)` | `Entity` | 334, 343 |
| `cat_pp` | `Entity -> Prop` | 347 |
| `cat_pp_than` | `Entity` | 325 |
| `cat_measure` | `Entity -> core:float` | 357 |

### `Num` is not the question — decided

`cat_measure` denotes a measure *function*: it is what `dependence` contributes, `λg. μ_dep(g)`, and the
comparative operators select it. `37 °C` is a degree, not a function from entities to degrees, so
`cat_measure` is not the slot for a quantity.

What is settled is that `Num` is not the question. A measure phrase is not a noun phrase: it is `MP`,
and consumers subcategorise for it directly. `MP` has no `Num` slot, so no agreement feature is
chosen lexically. The question this document previously asked — reuse `mass`, reuse `name` (D70), or
add a variant — does not arise.

### Prepositions constrain their object's type; three categories need widening — decided

A quantity denotes `Quantity(u)` (D93) and all three PP categories denote `Entity`, so `at 37 °C` has no
parse today. The widening follows a shape the grammar already has: `cat_np : Set -> Num -> Cat` with
`⟦cat_np(T,_)⟧ = T`. The PP categories become type-indexed the same way — `cat_pp_arg : Set -> Prep ->
Cat` — rather than acquiring a new mechanism.

### The quantity is neutral; its consumer supplies point-or-difference — decided

| sentence | marker | reading |
|---|---|---|
| incubated at 37 °C | at | point |
| sampled at 5 °C intervals | at | difference |
| heated to 37 °C | to | point |
| rose from 25 °C | from | point |
| warmer by 5 °C | by | difference |
| within 2 °C of the setpoint | within | difference |
| the temperature rose 5 °C | — | difference |
| a 5 °C increase | — | difference |
| 5 °C warmer | — | difference |

`at` carries both readings, so the `Prep` feature does not discriminate them. The reading comes from
whatever consumes the measure phrase: prepositions by vector semantics (Zwarts & Winter — `at` a
zero-magnitude state vector, `within` a bidirectional set, `to` the final state, `from` the initial);
scalar-change verbs by Kennedy & Levin's measure-of-change function `m^Δ`, which builds a derived scale
whose origin is the degree at the event's start; and the comparative morpheme by arithmetic on the degree
argument.

An earlier draft argued the opposite from the three marker-less rows — that the distinction had to live
in the quantity, because no marker was present to carry it. The consumer is not the marker: each of those
rows has a head that is independently a scalar-change verb (`rose`), a nominalised scalar change
(`increase`) or a comparative (`warmer`).

One item follows, and °C needs no second quantity type: the difference reading is `m^Δ`'s derived scale,
not a distinct primitive.

### Bare numerals and quantities share a carrier, not a category — decided

A cardinal composes as a determiner; a prenominal measure phrase is `N/N` (`a 5 °C increase`, `5 °C
intervals`). The counter-argument this document recorded is the right one.

### `by` has no verbal-modifier entry, and `prep_by` is the wrong fix — decided

`by` has exactly two entries in `ontologies/lexicon/closed-class.esl`:

- `by_agent` (577-584) — the passive agent, `(S[dcl,pass]\NP \ ((S[dcl,pss]\NP)/NP)) / cat_np(Entity,
  num_any)`;
- `by_nmod` (2449-2456) — noun post-modifier, `cat_pp / cat_np(Entity, num_any)`, sem
  `λy.λx. ontology:prep_by(x, y)`.

There is no third, and line 940 says why: *"One entry per preposition; the `by`-passive agent (§8.9) is
the special built case."* `by` was excluded from the VP-adjunct set because the passive claimed it.

Adding `prep_by` to the `Prep` enum would not reach this. `Prep` indexes `cat_pp_arg`, the ARGUMENT
category, which needs a subcategorizing head; `rise`, `increase` and `warm` do not subcategorize for a
PP. Measure-`by` is an adjunct on the adjective, `(Sadj\NP)\(Sadj\NP)`. The fix is an adjunct entry.

**No passive collision.** `by_agent` selects an active past participle `(S[dcl,pss]\NP)/NP` on its left;
a differential `by` adjoins to `Sadj\NP`. The two never compete, so an adjunct `by` does not reintroduce
the over-generation that the `pass`/`pss` voice distinction exists to block.

**A measure-`by` must not lower to `ontology:prep_by(x, y)`** — the opaque prepositional conjunct
`by_nmod` emits. That is the prose-shaped predicate this work exists to replace.

### Differential comparatives are exact here, not lower bounds — decided

Monotonic degree semantics makes `5 °C warmer than B` a **lower bound**: `∀δ(warm(B,δ) → warm(A,
δ+5))` holds whenever A exceeds B by at least 5, and the exact reading is a scalar implicature rather
than an entailment. That is what Haruta et al.'s FraCaS system encodes, and it is right for a
natural-language-inference benchmark, where the question asked is whether a conclusion follows.

It is wrong here. A paper reporting that a temperature rose 5 °C is not reporting that it rose *at
least* 5 °C. Encoding a measured value as a bound weakens every quantitative claim on the chain, and
the exact rationals D94 carries would arrive at a `≥` and stop meaning the measurement.

This document takes the alternative the literature names: **interval-based exact-degree semantics**,
where a measure phrase denotes a boundary rather than a lower bound. `5 °C warmer` lowers to an
equality on `m^Δ`.

**The cost, stated.** Haruta et al.'s λ-templates for comparatives are the monotonic ones and cannot
be reused verbatim. The categories transfer; the terms do not.

### One degree space, but an affine unit's vector reading changes unit — decided

Vector Space Semantics models a scalar dimension as a one-dimensional affine space, and a unit is
functionally dependent on its argument: an absolute state maps to a coordinate on the affine scale, a
differential to a vector magnitude. One degree space suffices, which is what lets the quantity item
stay neutral.

The unit does not survive that split unchanged. A 5 °C **difference** is 5 K. So when a consumer takes
the vector reading of a measure phrase in an affine unit, the result carries the associated vector
unit, not the affine one. °C is the only affine derived unit in SI (D93), so the rule has exactly one
instance today.

### The neutral quantity item is attested — decided

Schwarzschild base-generates a measure phrase as a predicate over sets of degrees (`⟨d,t⟩`). The item
denotes a property of an interval and the consumer resolves it. One underlying quantity item is safe.

Schwarzschild reaches the consumer by lexically governed type-shift; this document takes
subcategorisation instead, for the reason given under "Consumers subcategorise". That changes the
mechanism, not the neutrality of the item, which is what this entry records.

### Telicity needs no feature — decided

An overt measure phrase bounds the event semantically, but CCG implementations derive telicity
downstream from the logical form rather than marking it on a category. The stated reason — that a
syntactic feature would mean rebuilding a statistical parser's training data — does not apply to a
hand-authored lexicon. The conclusion holds for a different reason: telicity does not need to *gate*
composition here, since `rose for 5 minutes` and `rose in 5 minutes` should both parse and differ in
what they assert. `Dcl`, `Num`, `Fin`, `Prep`, `Mode` and `Conn` are unchanged.

### The difference is unsigned; the verb carries direction — decided

`m^Δ` always yields a positive difference relative to the verb's own scale orientation. `heat` and
`cool` map to one scalar dimension and differ in ordering relation, so `cooled by 5 °C` is an advance
of 5 along a downward-oriented scale. Magnitude comes from the quantity, orientation from the verb —
and an encoded proposition that drops the orientation collapses `cooled by 5 °C` into `heated by
5 °C`.

### Consumers subcategorise; they do not shift — decided, on one held assumption

A semantic type-shift under a single syntactic category predicts that `incubated at 37 °C and at the
promoter` **coordinates cleanly**, because both adjuncts project the same category. Coordination in a
typed categorial grammar keys on category identity, so if that string degrades, the two `at`s have
different syntactic types and the shift analysis is wrong.

So each consumer carries a subcategorised entry rather than a shared entry plus a unary rule: `PP/MP`
for the scale coordinate, `PP/NP` for the spatial location. In a grammar where the kernel makes a
syntax–semantics mismatch a hard failure, this predicts the coordination facts without `n` unary
rules in the kernel.

**The diagnostic as everyone stated it does not discriminate, including in the source that supplied
it.** The claim was that a single `at` plus a type-shift predicts `incubated at 37 °C and at the
promoter` is clean, so degradation would show the two `at`s differ. But with the preposition repeated
in both conjuncts, each conjunct is a *saturated* `(S\NP)\(S\NP)` — the object category is
discharged before `and` sees anything. Under subcategorisation the results are `(S\NP)\(S\NP)` too.
Both analyses predict a clean sentence, so the judgement carries no information. Stacked adjuncts
(`stored at the core facility at −80 °C`) come out the same way, which is why the paraphrase feels
equivalent.

**The change is additive per preposition, and the existing entries show its shape.** `lexicon:at_arg`
(`closed-class.esl:1756-1763`) is `cat_pp_arg(prep_at) / cat_np(Entity, num_any)` with
`sem_type = Entity -> Entity` — the `PP/NP` of the prescription, already written. The measure sense
is a sibling entry at `PP/MP`, not a widening of this one, so no shipped entry is disturbed.

**The cost is measured: seven entries, and one new `Prep` variant.** Seven prepositions govern a
measure phrase in the full text — `with` 16, `for` 11, `in` 9, `at` 7, `of` 7, `after` 2, `into` 1,
over 53 PP-governed measure phrases. Six are declared `Prep` variants; `after` is not, so it needs
one. Seven lexical entries plus an enum variant is a local, additive change; `n` unary type-shift
rules is a change to the derivation machinery in a grammar whose kernel checks every step. The trade
is good at `n = 7`.

**The discriminating configuration shares one preposition over two unlike objects:**

> Samples were stored at **[−80 °C]** and **[the core facility]**.

Here `and` must join an `MP` with an `NP` as objects of a single `at`. The shift analysis lifts the
MP to NP first, so both conjuncts are NP and it composes; subcategorisation leaves them distinct and
blocks it. Compare the uncontroversial pair: *John lives in London and Paris* (two NPs, fine) against
*\*John lives in London and that Mary left* (NP and clause, bad).

**It is unattested, so it cannot decide.** Across the full text of both versions, **zero** of the 143
measure phrases appear in this configuration. The two near-misses do share a preposition — `with
[10 µM etoposide] and [1 mM hydroxyurea]` is exactly the shape — but both objects are NPs carrying
prenominal measures, so they are like-category and discriminate nothing. The decision therefore rests
on the cost and structure above.

**What would reverse it:** a judgement that the shared-preposition sentence above is acceptable,
collected on a verb where both objects are independently plausible so the semantics does not confound
the syntax. An earlier draft of this section proposed a test set that repeated the preposition in
both conjuncts; that set is withdrawn, since it tests the configuration where the two analyses agree.

Target types are unchanged by the mechanism: a point-denoting preposition takes an affine coordinate,
a differential modifier takes a vector magnitude, and only the second admits arithmetic.

### The tolerance derivation, repaired — decided

The survey's derivation did not compose: `within : PP/PP/MP` applied to `2 °C` yields `PP/PP`, whose
argument must be a `PP`, and `of the setpoint` was typed `PP\NP`. Two reassignments close it, and the
result needs only forward application:

```
within            2 °C     of         the setpoint
(PP/PP)/MP        MP       PP/NP      NP
----------------------->              -----------------> 
      PP/PP                                 PP
      λP.λv. (P(v) ∧ |v| ≤ 2)          λv. origin(v) = setpoint
      --------------------------------------------------->
                            PP
              λv. (origin(v) = setpoint ∧ |v| ≤ 2)
```

`of` was wrongly given the noun-modifying category `(PP\NP)/NP` (`the temperature of the room`); here
it establishes the vector space's origin directly, which is `PP/NP`. And Zwarts & Winter analyse the
measure phrase as a modifier *of the preposition*, so `within` consumes it first: `(PP/PP)/MP`.

**This `PP` is a fourth PP category, not one we have.** Its denotation is `⟨v,t⟩`, a set of vectors.
All three existing ones denote `Entity` or `Entity -> Prop`. So the widening in "Prepositions
constrain their object's type" does not reach it — a vector-denoting PP is new, on top of
type-indexing the three that exist.

`of` already carries two entries (noun post-modifier, governed argument). Whether `PP/NP` generalises
to other vector origins, or is a third `of`, is not yet checked.

**Deferred, and the frame occurs in a variant form.** The exact `within N <unit> of <ref>` frame has
zero occurrences in the full text of either version of the paper. But a bounded-deviation
construction does occur, in the figure legends the band extraction excluded:

> boxes span the interquartile range; whiskers extend to the furthest point **within 1.5× the
> interquartile range (IQR) from the box**

Once in the Nature text and five times in the author manuscript (`within 1.5*IQR from the hinge`).
It differs from the design sentence in three ways: the reference preposition is **`from`**, not `of`;
the measure is a **scaled statistic** (`1.5× the IQR`), not a unit-bearing quantity; and it states how
a plot was drawn rather than asserting a claim about WRN.

**And the shape is already decided, by the statistics institution.** A measurement stated against a
reference is what `stats:EffectSize` is (`ontologies/statistics/statistics.esl:150-166`):

| Constructor | Carries |
|---|---|
| `Absolute(core:float, core:string)` | magnitude, units |
| `Relative(core:float)` | fold-change / ratio |
| `StandardizedCohensD(core:float, core:float)` | mean_diff, pooled_sd |
| `StandardizedHedgesG(core:float, core:float, core:integer)` | mean_diff, pooled_sd, n_total |
| `EtaSquared(core:float)` / `OmegaSquared(core:float)` | variance-explained threshold |

Its own doc comment states the principle: *"Keeping the form rather than normalising to one number is
what lets a recomputation check the claim as written."* A whisker rule, a fold-change and a
standardised difference are all this shape, and D52 fixed their form before this document existed.

**The parser does not emit institution vocabulary, and an earlier draft of this section said it
did.** The parser reads the paper and produces its constituent propositions — `eigentt:Term`s. That
is its whole output. `stats:EffectSize` and the SAP machinery are chain-resident vocabulary the
statistics institution recomputes against a `SampleSet`; they are not a parse target, and there is no
phrase the parser "hands off" rather than parsing.

The two meet afterwards, through the justification layer. A parsed proposition is what the paper
**declares**; an institution-verified result is **grounds** that may be cited for it. So
`stats:EffectSize` existing does not relieve the grammar of producing a proposition for
`within 1.5× the IQR from the box` — it means that proposition can later attach to something the
institution checked.

This restores the derivation's purpose. `within : (PP/PP)/MP` with `of : PP/NP` is solving the
parser's problem, which is denoting the phrase. Whether an institution also recomputes the claim is
orthogonal, so the derivation stays as the design and the only question about building it is corpus
frequency — which one paper measures weakly.

**What it does reclassify:** nothing about parse targets. `median 0.56-fold` and `Global 18%` sit in
the "other" bucket of the table above because they are not PP-governed, not because they belong to
another system. They still need propositions.

**The collision D93 already names, with a live instance.** D93 records that "D52 punts units to
`core:string`". `Absolute(core:float, core:string)` is binary64 plus an unparsed unit string, so
D93's typed `Unit` and D94's `core:rational` have a shipped consumer that does neither. The WRN
fixture shows the wart directly — `crates/eigenius-statistics/tests/fixtures/nested_anova_wrn.esl:66`
reads `stats:effect_size = Absolute(0.0, "relative-ratio")`, a relative effect encoded with a
placeholder magnitude and the word `relative-ratio` put in the units slot because `Relative` did not
fit what the fixture meant.

### Opaque scales and measured quantities are two sorts — decided

Sassoon (2010) maps Stevens' taxonomy onto natural-language semantics, and the sorts fall out of it:

| Scale | Example | Admits |
|---|---|---|
| ordinal | `μ_dep` — `dependence on WRN` | ordering only |
| interval | °C | ordering, differences |
| ratio | kg, a percentage | ordering, differences, ratios |

Arithmetic on an ordinal scale is meaningless, so one category covering both would assert that
ordinal relations support the `+5` of a differential comparative and the measure-of-change function.
`cat_measure` stays as it is, for opaque scales needing only a strict ordering; measured quantities
get a distinct category carrying interval arithmetic.

**The grammar is sensitive to the distinction, which is what makes it categorial rather than
semantic.** `5 °C warmer` is fine and `*5 degrees more dependent` is not, over the same comparative
machinery.

**A correction to this document's earlier framing.** An unnamed unit is not a dimensionless quantity.
A dimensionless quantity — a percentage — is a *ratio* scale and admits arithmetic; an opaque scale
is ordinal and admits none. The question this document asked, whether `μ_dep` is "meaningfully a
quantity at all", has the answer no.

### Ranges need no new semantics; the obstacle is lexical — decided

`45-60%` and `≤ 2 °C` are generalized quantifiers over degrees: instead of saturating the degree
argument with a point or a magnitude, they supply a constraint (`λd. d ≤ 2`) that restricts it, which
is the `⟨d,t⟩` typing applied directly. They stay out of v1 for the reason Scope already gives, which
is lexical rather than semantic — separating a range from a catalogue number (`926-68021`) needs the
en-dash/hyphen distinction.

### Two facts recorded, not yet acted on

- **The `Prep` enum and its importer have drifted.** `governed_preposition`
  (`crates/eigenius-wordnet/src/convert.rs:988`) extracts 11 prepositions and maps them at 1025-1037 with
  `_ => prep_any`; the enum declares 14, with `of` (dated 2026-07-26), `as` and `any` added by hand.
- **`5 °C` is exposed to the N-N kind compound rule.** Measure-phrase parsers are reported to mistake
  units for noun-noun compounds, and `closed-class.esl:941-942` names that rule as shipped. Seeding needs
  a regression test against it.

## The corpus says PPs are the minority case

Classifying every measure phrase by what governs it. Two extractions, because neither is clean:

| Construction | band-limited | full text | Examples |
|---|---|---|---|
| PP-governed | 35 (44%) | 53 (37%) | `with 10%`, `for 1 h`, `at 37 °C` |
| appositive / reagent list | 18 (22%) | 34 (24%) | `Tween-20, 0.1%`, `Scale bar, 200 μm` |
| other (prenominal, statistic) | 18 (22%) | 46 (32%) | `median 0.56-fold`, `Global 18%` |
| distributive `every N unit` | 5 (6%) | 5 (3%) | `every 3 days`, `every 2–3 days` |
| verbal argument | 4 (5%) | 4 (3%) | `containing 4 μg`, `purified 72 h` |
| **total** | **80** | **143** | |

**The band-limited run covers 41% of the paper.** `extract-section.py` takes two type-size bands —
Letter body 12.2–13.0 pt and Methods 10.6–11.15 pt — and excludes figure captions by design: its own
header records that content rules for captions "were tried and abandoned". That is 7,650 words of
18,775, and 80 of 143 measure phrases.

**The full-text run is complete and dirtier.** `pdftotext` over a two-column Nature page merges
columns, which puts artefacts such as `2 5 A` into the "other" bucket. So the band extraction is
typographically clean but partial, the full extraction complete but noisy, and neither count is
authoritative. Both agree on one thing: PP-governed measure phrases are a minority.

**The "appositive / reagent list" bucket was an artefact of the pattern and is withdrawn.** It
matched `<word>, <number><unit>` across list-item boundaries, so `0.1% Tween20, 0.1% BSA` was read as
apposition when the comma is a list separator. The instances read:

> …DMEM medium (Thermo Fisher Scientific, 11995073) with **10% FBS** (Sigma-Aldrich, F8317),
> **1% penicillin–streptomycin** (…), **10 μg ml⁻¹** of gentamicin and **250 ng ml⁻¹** fungizone…

That is NP coordination under one preposition, each conjunct carrying a **prenominal** measure
phrase — two constructions this document already has. The real shapes over the full text:

| Construction | n | Status |
|---|---|---|
| prenominal (`10 μM etoposide`, `0.1% crystal violet`) | 40 | already scoped as `N/N` |
| inverse-exponent unit (`μg ml⁻¹`, `ng ml⁻¹`) | 13 | **new — tokenizer** |
| label–value caption idiom (`Scale bar, 50 μm`) | ~9 | **new — the only genuine appositive** |
| `pH 7.5` | 2 | out of v1: D93 defers logarithmic units |

The unscoped remainder is much smaller than the classification table suggested, and prenominal
modification — not apposition — is what methods prose leans on.

**`μg ml⁻¹` is a tokenizer case this document missed.** Its compound-unit discussion covers `mg/dL`,
where a slash joins the parts. The inverse-exponent form is space-separated with a superscript minus
one, so the separator substitution and edge-trimming that produce today's tokens strip the `⁻¹` and
leave `ml` — turning a concentration into a volume rather than failing. Thirteen occurrences.

Prepositions governing a measure phrase, full text: `with` 16, `for` 11, `in` 9, `at` 7, `of` 7,
`after` 2, `into` 1. Seven prepositions — but only **six are declared `Prep` variants**. `after` is
not in the enum's 14 (`to on in with from for at upon about against into of as any`), so it needs a
variant before it can carry a governed measure phrase at all.

**These counts are reproducible.** `experiments/parsing/measure-quantities.py <corpus.txt>` emits
this table and the preposition counts; `--verbose` dumps every instance per bucket. Its docstring
records the four ways this measurement has produced a wrong-but-plausible number, including the two
that reached earlier drafts of this document.

### `every N unit` — category decided, semantics must not follow the standard one

`every 3 days`, `every 2–3 days`, `every 48 h` — five occurrences, a repeated procedure with a
stated period. The existing `every` is a universal determiner over a common noun
(`closed-class.esl:1818-1833`), `cat_forall(sg, λT. …)` with `∀A. (A → Prop) → Prop`; `3 days` is an
`MP`, not an `N`, and these are VP adverbials rather than argument-position determiners.

**The category is the subcategorised adverbial** — `((S\NP)\(S\NP))/MP`, a third `every` entry
beside the two determiner ones. That is the same choice this document already made for prepositions,
so it introduces no mechanism: one lexical entry, no unary rule.

**The standard semantics does not transfer, for three reasons.** The textbook form is

```
λV.λx. ∀t (adjacent_interval(t) ∧ magnitude(t) = 3_days
           → ∃e (V(e) ∧ theme(e,x) ∧ τ(e) ⊂ t))
```

1. **It needs event variables, and this grammar decided against them.** `closed-class.esl:938-939`
   states it for the PP adjuncts: *"no event variable; the relation is opaque, mapped to the domain
   predicate's context slot by the institution"*. There is no event, trace or thematic-role
   vocabulary anywhere in the lexicon or logic ontologies. Adopting `∃e`, `theme` and `τ` reverses a
   standing decision rather than extending one.
2. **`adjacent_interval(t)` is one-place, but adjacency is a relation.** As written it cannot say
   the timeline is partitioned into consecutive intervals, which is what it is meant to say.
3. **`∃e` is too weak to pin periodicity.** "Every 3-day window contains at least one change" is
   satisfied by changing the medium daily. It needs exactly-one. This is the same defect shape as
   the differential-comparative formulas — a bound where the reading is exact — and this document
   already took exact semantics over monotonic bounds for that reason.

**The house pattern applies instead:** an opaque conjunct, as the PP adjuncts use.
`λM.λV.λs. And(V(s), ontology:every_period(s, M))` — the periodicity is a relation the institution
interprets, the measure phrase is its second argument, and no event machinery is introduced.

**Not built in v1.** Three of the five instances carry ranges (`every 2–3 days`, `every 3–4 days`),
which are already deferred, so two are reachable. The category and the semantic shape are recorded;
whether two occurrences earn an entry is a coverage-target call.

### Dispersion splits in two, and only one half is a vocabulary gap

*"We measure concentration of blood cells at x ppm with two sigma of the average concentration in
baseline cohort."* An earlier draft of this document recorded this as a single gap, on the claim that
the statistics ontology "has no uncertainty vocabulary at all". That was a bad grep — for *sigma*,
*coverage* and *confidence*, which are not the words used. `statistics.esl:277-289` declares
functionals over a `SampleSet`:

```
axiom stats:mean_of      : core:string -> core:float
axiom stats:variance_of  : core:string -> core:float
axiom stats:median_of    : core:string -> core:float
axiom stats:mean_diff_of : core:string -> core:float
```

with `slope_of`, `intercept_of`, `spearman_rho`, `ppv` and `sensitivity` beside them, and dispersion
also appears as a component inside `StandardizedCohensD(mean_diff, pooled_sd)`.

**The `with` / `within` ambiguity is the split.** The two readings are different propositions with
different targets, which is why they cannot share one open question:

| Reading | Proposition | Owner | Corpus |
|---|---|---|---|
| agreement — *within* 2σ of the baseline average | `le(abs(sub(x, mean_of(S))), mul(2, sqrt(variance_of(S))))` | D52 | 21 with `mean ± s.e.m.` |
| precision — measured *with* 2σ | a value carrying its own uncertainty | D93, or deferred | 0 |

**The agreement reading is a plumbing gap, not a vocabulary one.** `mean_of`, `variance_of` and `le`
all exist. `abs`, `sub`, `mul` and `sqrt` do not: the `stats:` namespace declares only the three
comparison axioms (`lt`, `le`, `gt`), and `sqrt` exists solely as `formulas:ops:sqrt`, in another
institution. `mean ± s.e.m.` — 8 occurrences in the Nature text, 13 in the author manuscript — is
`sqrt(variance_of(S)/n)` and stops at the same place. **This is the common case and it needs
arithmetic, not new concepts.**

**The precision reading is deferred.** A measurement result carrying its expanded uncertainty (GUM,
coverage factor k = 2) has no representation anywhere: `EffectSize::Absolute(core:float,
core:string)` is a point value and D93's magnitude `q × Π cᵢ^{eᵢ}` is a point too. Adding it changes
the magnitude from a point to a value-plus-interval, and uncertainties propagate in quadrature rather
than like values — an algebraic addition, not a field. Zero corpus occurrences.

## The sorting rule: register goes to the CNL, clauses go to the grammar

`Scale bar, 50 μm`, `pH 7.5` and `(1,200 V, 20 ms, 2 pulses)` are not sentences. They are a label
and its value, in the elliptical register figure legends and instrument settings are written in.
Expanding one to the sentence it abbreviates — `The scale bar is 50 μm.` — adds nothing and drops
nothing, so it is a **register** change, and the controlled-English guide is where register is
handled. It is now a DON'T row there, not a construction here.

The guide already carries the safeguard that keeps this from becoming a way to avoid building
grammar: R2, *a faithful un-parsed claim beats a parsed distorted one*, which is why its range row
forbids collapsing `20–30%` to a midpoint. Rewriting that changes content is barred; rewriting that
changes register is not.

**The general rule, which this document had been missing:**

| Source shape | Goes to |
|---|---|
| non-sentential — a label, a parameter list, a caption annotation | a CNL rewrite rule |
| a well-formed clause the grammar cannot yet parse | grammar work |

Sorting each discovery into one of these is what keeps a new construction from becoming a new open
question. Applied to what the corpus turned up: `Scale bar, 50 μm` and `pH 7.5` are rewrites;
`every 48 h` sits inside a clause (*"doxycycline was added and refreshed every 48 h"*) and is
grammar work; `μg ml⁻¹` sits inside a clause and is already covered by the unit sub-grammar's
rational powers.

## Carried in from D93

D93 was built after this document was written (PR #262). What it settled, and one piece of work,
come into D95:

- **`931g` — resolved.** This document treated `931g` as gram and g-force competing in the chart; D93
  then decided that **v1 refuses to split a `g`-suffixed numeral**, since standard gravity is not in
  the vocabulary. The split rule, the preprocessor list and "Ambiguous unit symbols" are revised to
  the refusal: a bare `g` suffix is not split, and a prefixed gram is.
- **One converter.** The unit sub-parser normalises prose — `µ` (U+00B5), superscript exponents,
  `per` — into D93's strict stated form and calls `units::convert::Vocabulary::convert`, which the
  ESL form `units:quantity(v, "…")` also uses (D93 implementation plan, D6.2). Symbol resolution,
  prefixes, the °C point-reading offset and exactness are decided there, not here.
- **`%` and `ppm` are number notation**, a scale the parser applies, not units-layer vocabulary: D93
  records `%` as "not a unit — notation for a number".
- **What a quantity term is.** `(units:mk_quantity(coefficient, pi) : units:Quantity(unit))`, in
  base units, with the unit a group element over the seven SI base dimensions — `rad`, `sr` and `°`
  are all `1`. Quantity kinds (plane angle, solid angle) are metadata that conversion returns beside
  the unit; nothing here consumes them yet (D93, "Kinds are metadata, not algebra").
- **No stated-unit record.** What the author wrote stays in `enc:prose`, reached through
  `enc:from_unit`; D93 dropped the per-occurrence record, so this work emits none.
- **The WordNet importer's counts** (work item). `push_entry` holds the closed-class guard for all 19
  emission sites but returns nothing, so every caller counts an entry it may not have written: the
  2026-09-25 reseed reported 471,743 entries and wrote 471,655 — 88 withheld, 13 of them mass. The
  UMLS importer checks at the call site and counts `grammatical_skipped`; the WordNet one should
  report the same way, with `push_entry` returning whether it wrote. Separately, degree-noun entries
  (`e_<adj>_d_<noun>`) are emitted once per adjective–noun link path — 5,534 duplicate declarations
  with identical bodies, deduplicated at load — and should be emitted once. Neither changes the
  loaded lexicon, so neither needs a reseed.

## Open questions

- **Figure and table references against the numeral/unit split.** The unbracketed `Fig. 2d` in the
  WRN methods would split into two days, as `Fig. 2h` would into two hours; ten unbracketed
  `Fig. N<letter>` references occur there, and `Extended Data Fig.` and `Table` take the same form.
  The likeliest rule is that the preprocessor does not split a token directly after a figure or
  table reference word — the split is the preprocessor's decision, as the design section makes it.
  Undecided.

- **Arithmetic over the statistics functionals.** Nothing in the `stats:` namespace combines them.
  See "Dispersion splits in two" above; the agreement reading and `mean ± s.e.m.` both stop here.

## Scope

**In.** The lexer/preprocessor decomposition: a lexer that deletes nothing, and a preprocessor
owning every token-stream decision — the ones that exist today (bracketed asides, separator
substitution, edge trimming, comma handling, non-prose classification) and the new ones (quantity
span merging, the numeral/unit split, the revised aside rule that distinguishes a gloss from a
function argument, the revised `is_nonprose` that admits `53BP1` as a symbol). Then: the unit
sub-parser, run once per candidate sense; quantity items in `seed_leaves` — **one per candidate unit
sense**, as competing edges, with normalisation deferred until the chart selects; unit-symbol lexical
entries with senses; the `MP` category and one subcategorised entry per preposition sense that takes
it; the type-indexing of `cat_pp_arg`, `cat_pp` and `cat_pp_than`; a distinct interval-arithmetic category for measured quantities, leaving `cat_measure` as the ordinal
one; an adjective-adjunct entry for `by`; exact-degree λ-templates for the comparative constructions,
replacing Haruta et al.'s monotonic ones; a quantity-bearing corpus and re-established baseline; the
CNL guide revision.

An earlier draft's Scope described the *rejected* alternative — span recognition with exemptions
from the destructive steps — which the body argues against.

**Out of v1.** Intervals and ranges (`15–18`, `20–30%`), which need the en-dash/hyphen distinction
to separate a range from a catalogue number (`926-68021`, a LI-COR part number) and an interval type
D93 also defers; statistic routing to D52 records, which is separate existing work; the tolerance
construction (`within 2 °C of the setpoint`) and the fourth, vector-denoting PP category it needs,
deferred on zero corpus attestations.

An earlier draft also placed "any change to the composition rules" out of scope, on the grounds that
D93 showed them unaffected. That is false and the reason is recorded above: `cat_pp_arg`, `cat_pp`
and `cat_pp_than` each fix their object to `Entity`, so no quantity composes with a preposition until
they are type-indexed.
