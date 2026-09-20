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

## `931g` is the inverse of multiword seeding, and harder in one respect

Multiword seeding **joins**: `cell line` is N surface tokens with a lexicon entry for the joined
span, and `seed_leaves` emits the joined item *alongside* the split ones, "carried as competing
chart edges, not resolved here."

A numeral-adjacent unit **splits**: `931g` is one surface token that must yield two constituents.
Same lattice, opposite direction — but **not the same mechanism**, and an earlier draft claimed it
was. The chart is `vec![vec![Vec::new(); n]; n]` with `n = tokens.len()`
(`kernel/src/dcg/parse/seed.rs:568-581`). A joined span is a union of cells that already exist; a
split needs *more positions than there are tokens*, and there is no cell to seed into. So the split
must happen where `n` is still being decided — in the preprocessor, which is where this document's
own design diagram puts it — and not in seeding.

**The asymmetry that makes splitting harder: the boundary is not given.** An MWE chooses among
boundaries the tokenizer already produced. A split must *propose* one, and the proposal space is
open — `931g` divides 3|1, `10x` 2|1, and `5-fold` and `53BP1` should not divide at all.

**The unit vocabulary is what makes it decidable.** A quantity split is
`⟨numeral⟩⟨known unit symbol⟩`, so the boundary is "the longest suffix that is a unit" — a lookup,
not a search:

| token | prefix | suffix | split? |
|---|---|---|---|
| `931g` | `931` numeral | `g` is a unit | **yes** |
| `53BP1` | `53` numeral | `BP1` is not a unit | no |
| `HEK293T` | `HEK293` not a numeral | (`T` is tesla) | no |
| `5-fold` | `5` numeral | `-fold` is not a unit | no |
| `2-2` | `2` numeral | `-2` is not a unit | no |

Both halves of the test are load-bearing. Without the numeral-prefix test, `HEK293T` would split on
tesla; without the unit-suffix test, `53BP1` would split on nothing.

So the unit vocabulary is not only what D93 needs for typing — it is what makes this tokenization
decidable at all. Before units exist, `931g` is unanalysable.

## An existing defect the same change repairs

`is_nonprose` routes out any token starting with a digit. Measured:

| token | routed out? | correct? |
|---|---|---|
| `53BP1` | **yes** | **no** — a gene, 12 occurrences in the Letter body |
| `HEK293T` | no | yes — a cell line |
| `931g` | yes | yes today, no once quantities parse |
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

## One chart item per quantity

The recognised span becomes a single leaf item carrying a magnitude and, where present, a unit. A
bare `0.56` is the same item with no unit — which keeps cardinality and quantities on one path
rather than building two mechanisms that must later agree.

**One guard neither document mentioned.** `cat_has_selectional_slot` routes a grammar with any
index-dependent argument slot to the unpacked CKY path, because node-level packing by `cat_shape`
erases the index and would be unsound. Its test is `matches!(ty, Exp::EigonClass(iri) if iri !=
ENTITY_TOP_IRI)` (`kernel/src/dcg/category.rs:466-475`) — and a quantity index is an *application*,
not an `EigonClass`, so the guard returns `false` and the packed path is taken *with the index
erased*. Extending the guard is in scope, or quantity slots must be type variables.

Its category needs no new constructor. `lexicon:Cat` is already type-indexed —
`cat_np : Set -> Num -> Cat` with `⟦cat_np(T,_)⟧ = T` — so a quantity is an NP at a quantity type,
`cat_np(units:Quantity(u), n)`, denoting the `Quantity u` itself (D93). **The composition rules are
therefore unchanged**: a quantity NP is a preposition's object or a nominal modifier by the rules
that already compose entity NPs.

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
- **merge quantity spans** — `37` `°` `C` into one item, `931g` split on the numeral/unit boundary,
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

So a **separate unit parser** runs on the unit portion of a recognised span and returns a `Unit`
term. It is small, closed and total: a finite symbol set, products, quotients and integer powers.
The DCG never sees it; it sees the resulting quantity item.

## Ambiguous unit symbols are polysemy, not a special case

`931g` is g-force; `10 g` is grams. `M` is molar or mega. `h` is hour, and in another register the
Planck constant.

A unit symbol is a **lexeme carrying several senses**, exactly as a noun carries several synsets,
and each sense denotes a different unit. It therefore routes through machinery that already exists:
the sense cap and contextual `SenseRanker`, the felicity gate, and reading selection. A wrong unit
sense is refused or down-ranked by the same path that refuses a wrong noun sense.

**This document contradicted itself on where the sense is chosen, and the contradiction has to be
resolved before implementation.** One section has the sub-parser "return a `Unit` term" at
recognition time; another says resolving early "would put unit disambiguation outside the ranker".
Both cannot hold: if a `Unit` is returned at recognition, gram-versus-g-force is already decided
before the `SenseRanker` runs.

The resolution that keeps the ranker in play is that **a quantity span seeds one item per candidate
unit sense**, as competing chart edges — the same shape as a polysemous noun. Which means the item
is not "one item carrying a unit" but "one item per sense", and D93's "the parser emits both the
stated and the normalised form" must happen *after* selection, not at recognition.

That is a different item shape from the one this document specifies above, and reconciling the two
is open work rather than a wording fix.

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

## Open questions

- **Which `Num` does a quantity carry?** The feature is syntactic and erased by ⟦·⟧, routing
  agreement only. `mass` is the closest existing fit — `24 h` takes no article, as `MSI` does not —
  but reusing it may import agreement behaviour a quantity should not have. `name` (D70) is also
  bare. A distinguished variant may be cleaner.
- **Do prepositions constrain their object's type?** `at 37 °C` and `at the promoter` are one
  preposition over different index types. If `at` is already polymorphic in `T` nothing changes;
  if constrained, it needs widening. Answerable by reading the lexicon.
- **Where does the °C point/interval decision get made?** D93 assigns the point reading to a bare
  °C and records the difference reading as a gap. The disambiguator is the preposition — "to"
  versus "by" — so this is a parse-time decision, and it is unclear whether it belongs in the
  quantity item or in the PP rule that consumes it.
- **Do bare numerals and quantities really share one item?** Asserted above on the grounds that two
  mechanisms would have to agree. The counter-argument is that cardinality (`two genes`) composes as
  a determiner and a quantity does not, so they may want different categories despite a shared
  carrier.

## Scope

**In.** The lexer/preprocessor decomposition: a lexer that deletes nothing, and a preprocessor
owning every token-stream decision — the ones that exist today (bracketed asides, separator
substitution, edge trimming, comma handling, non-prose classification) and the new ones (quantity
span merging, the numeral/unit split, the revised aside rule that distinguishes a gloss from a
function argument, the revised `is_nonprose` that admits `53BP1` as a symbol). Then: the unit
sub-parser; quantity items in `seed_leaves`; unit-symbol lexical entries with senses; the `Num`
decision; a quantity-bearing corpus and re-established baseline; the CNL guide revision.

An earlier draft's Scope described the *rejected* alternative — span recognition with exemptions
from the destructive steps — which the body argues against.

**Out of v1.** Intervals and ranges (`15–18`, `20–30%`), which need the en-dash/hyphen distinction
to separate a range from a catalogue number (`926-68021`, a LI-COR part number) and an interval type
D93 also defers; statistic routing to D52 records, which is separate existing work; any change to
the composition rules, which D93 shows are unaffected.
