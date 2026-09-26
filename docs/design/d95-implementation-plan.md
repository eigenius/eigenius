# D95 implementation plan — quantities in the tokenizer and parser

Ordered slices for D95. Every `file:line` is at `quantities-in-the-parser` `69b6302`; paths under
`kernel/src/dcg/` are written from `dcg/`. D93 (units, conversion) and D94 (exact rationals) are built.
D96 (JATS) is decided and not built, so every slice works on plain text.

## Before starting

- **The measurement page is not in this checkout.** `scripts/measure-parse-rate.sh` defaults to
  `references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v3.txt` (`:84`, `:131-138`), and so
  do `experiments/parsing/baseline.json:11` and `db_backed_encoding.rs:114-127`. `/references` is
  gitignored (`.gitignore:18`), and the directory here holds only `methods.txt`, `letter-body.txt` and
  `extract-section.py`. The script fails at its page check (`:139`). Slices 2 and 5 measure; the page
  has to be provisioned before either.
- **Disk.** `cargo test --workspace` fills the volume. Each slice runs the crates it touches.

## Decisions

**All five are closed** (2026-09-26): 1, 2, 3 and 5 decided, 4 deferred to D96's build.

1. **The `MP` category is indexed by its unit.** `cat_mp : core:unit -> lexicon:Reading -> Cat`, with
   `⟦cat_mp(u, value)⟧ = units:Quantity(u)` and `⟦cat_mp(u, difference)⟧ = units:Difference(u)`, and
   a binder `cat_unit_forall : (core:unit -> Cat) -> Cat`,
   `⟦cat_unit_forall(λu. R)⟧ = Πu:core:unit. ⟦R⟧`, for consumers that take any unit. It follows
   `cat_forall`: the binder is instantiated from the consumed item and the sem is applied to the unit,
   where the feature binders are erased. A consumer that needs a dimension names it
   (`warmer : … / cat_mp(u"K", difference)`), so `5 mg warmer` has no parse.
   - **Packing** keys units exactly (finding 5, slice 4). *This replaces the earlier text, which had
     slice 4 decide a unit-selecting consumer against every unit in a node.*
   - **Rejected:** an unindexed `cat_mp` denoting `Σu. Quantity(u)`. No category could select a
     dimension, and moving to the indexed form at slice 7 would change the item shape and every `MP`
     entry written before it.
2. **Prose unit spellings are `lexicon:UnitSurface` resources.** Each pairs a spelling
   (`lexicon:unit_form`) with one `units:NamedUnit` (`lexicon:unit`). Every `units:symbol` is a
   spelling of its own unit implicitly; the resources add senses: `g` → standard gravity, `l` →
   litre, `RCF` → standard gravity, `days` → day. Prefixability comes from `units:prefixable`.
   - **`lexicon:unit_form`, not `lexicon:form`.** `lexicon:form`'s domain is `LexicalEntry`
     (`lexicon-ontology.esl:374-377`) and it feeds `lexicon:form_index` (`:386-390`). The precedent is
     `lexicon:construct_form` on `lexicon:ReservedConstruct` (`:551-562`), loaded by type.
   - **Read by the quantity recogniser only**, into a case-sensitive table (`M` and `m` differ; the
     lexical index is lowercase-keyed). A factor resolves exact-then-longest-prefix to every sense, so
     `mg` has one reading and `g` two. Each reading is restated in strict symbols (`g_n`, `mL`) and
     converted by `Vocabulary::convert`, so ESL's `units:quantity(…)` keeps one reading per string.
   - **Not ranked.** *An earlier version said a reading's sense key is its named unit.* A chart item
     carries no sense (finding 4), so `931g`'s readings reach the reading choice as distinct parses.
   - **Not a `lexicon:LexicalEntry`**: a unit spelling has no category and never stands in the chart
     alone; as entries `h`, `g`, `l` and `min` would seed items with no numeral before them.
   - **Not in the units layer**, which keeps one symbol per unit.
   - **The molar and the week enter the units layer** under D93's criterion: `M` = 1000 mol·m⁻³,
     prefixable (`mM` 5 times in the methods); `wk` = 604800 s, not prefixable (`weeks` once).
   - **No spellings for prefix names** (`milligrams`) in v1: none occur in the methods.
3. **A numeral-initial token no rule interprets is a word.** With no lexical entry it counts as a
   missing lexeme, as `53BP1` does under the revised non-prose rule. Setting the token aside was
   rejected: that parse omits a constituent.
4. **Figure references in plain text are deferred to D96's build.** Until JATS ingest exists, an
   unbracketed `Fig. 2d` reads as two days; bracketed references are removed as asides.
5. **A difference is its own type.**
   - **`units:Quantity(u)` is the measured value**, as built: `at 37 °C` is 310.15 K, `for 2 h` is
     7200 s.
   - **`units:Difference(u)` is new**, with the same constructor arguments. `5 °C warmer` is 5 K. The
     converter gains a difference reading that never applies the °C offset, and ESL gains
     `units:difference(5, "°C")`.
   - **The reading is a category feature**, `lexicon:Reading` = `value | difference`. Like `Mood` it
     changes the denotation, so it is not erased and has no wildcard.
   - **Every quantity token seeds both items**; they differ in value only for a bare °C. `931g` seeds
     four.
   - **Every consumer declares one reading.** `value`: `at`, `for`, `in`, `of`, `after`, `with`, the
     prenominal modifier. `difference`: `warmer`, the differential `by`, `rose`, `increase`,
     `intervals`, from slice 7. A test checks that no entry takes both, and each consumer gets a °C
     test, since only °C shows a wrong reading.
   - **Arithmetic, in slice 7, is typed:** value − value → difference, value + difference → value,
     difference ± difference → difference. Value + value depends on the kind of quantity and is slice
     7's question.
   - **Rejected:** the unit's origin in the category (needs `units:add` in the kernel and a second
     index); one item carrying a pair of `Quantity(u)`s (point and difference share a type); two items
     in one category (the ranker would choose what the consumer determines).

## What the code says that D95 does not

1. **Only two PP categories fix their object's type.** `denote_cat` (`dcg/category.rs:34-154`) gives
   `⟦cat_pp_arg(_)⟧ = Entity` (`:83-85`) and `⟦cat_pp_than⟧ = Entity` (`:74-76`) because each marker
   is transparent. `⟦cat_pp⟧ = Entity → Prop` (`:89-94`) denotes the predicate over the modified noun,
   so `of : cat_pp / cat_mp(u, value)` needs no change to `cat_pp`, and a VP adjunct is not a PP
   category. D95 says three.
2. **`at`, `of` and `after` have no VP-adjunct entry, and `after` has no entry at all.** The adjunct
   set is `in for with to from` plus `among between beyond within without`
   (`closed-class.esl:1126-1406`, sems `:943-1001`, six finiteness variants each). There is no
   `ontology:prep_after` (`ontology.esl:68-89`), and `after` is not in the importers' closed-class list
   (`dcg/closed_class.rs:36-40`). `after 2 h` needs an adjunct entry, not a `Prep` variant.
3. **Conversion happens at seeding.** An item's sem must have type `Quantity(u)` for the felicity gate
   (`dcg/parse/felicity.rs:76-174` checks `sem : ⟦cat⟧`), and D93 dropped the stated-unit record, so
   each reading is converted when it is seeded.
4. **Quantity items are not ranked.** `LexEntry.sense` lives on the entry; "once a leaf enters the
   chart its Item carries no sense" (`dcg/lexicon.rs:272-273`). Items seeded outside a `LexEntry`
   (derived adverbs, `dcg/parse/seed.rs:597-629`) skip `apply_sense_cap` (`:1045-1104`) and the ranker
   (`contextual_sense_ranks`, `dcg/parse/mod.rs:973-1096`). So `931g`'s gram and standard-gravity
   readings are distinct parses, and the reading choice records the pick: `enc:DecisionPoint` is
   emitted per encoded sentence (`crates/eigenius-encoding/src/emit.rs:339-410`). D93's table puts the
   `931g` sense there. D95's "the sense ranker sees unit senses alongside word senses" does not hold.
5. **Packing erases units.** `cat_shape` and `cat_key` render `Exp::LitUnit` as `_`
   (`dcg/chart/forest.rs:286`, `:356`), so `cat_mp(u"kg", value)` and `cat_mp(u"s^-2·m", value)` would
   share a node. The packed CKY decides a node pair once, on representatives
   (`dcg/chart/packed.rs:286-316`), and a representative's refusal records no edge for the others.
   Class indices have the same exposure; soundness rests on no shipped entry having a concrete
   non-`Entity` slot (`dcg/chart/forest.rs:383-392`). Rendering the unit in both keys puts different
   units in different nodes.
6. **The category unifier does not know `cat_mp`.** `unify_into` (`dcg/category.rs:292-354`) handles
   `cat_np`, `cat_n`, `cat_group`, `cat_s`, `cat_pp_arg` and slashes, and falls back to `slot == arg`
   (`:353`), so a unit variable in a consumer's slot would never bind.
7. **The CNL guide was revised in #262.** `docs/method/controlled-english-style-guide.md` splits test
   statistics (`:84`) from measured quantities (`:85`, 🔜), with a 🔜 section (`:98-127`) and a note
   that a quantity-bearing corpus is part of landing them (`:199-204`). D95's "The CNL guide needs
   revising" quotes the old rule. Slice 5 turns the 🔜 rows current.
8. **Tokens carry no source offsets.** The encoder computes a sentence's span as
   `doc.find(text)` (`crates/eigenius-encoding/src/formalize.rs:89-93`, `:216-220`): byte offsets of
   the first occurrence, 0 on a miss, which `encoding.esl:61-69` documents as character offsets. Not
   D95's to fix; it is D96's offset-base question.
9. **Today's aside stripping loses text.** An unclosed opener drops the rest of the sentence, and
   `a(b)c` becomes `ac` (`dcg/segment.rs:194-203`). The bracket arms of the separator substitution
   (`:154-156`) are unreachable.
10. **The harness tokenizes for itself.** `encode_unit` in
    `crates/eigenius-wordnet/tests/db_backed_encoding.rs` computes its OOV list with `tokenize` and
    `is_nonprose` (`:905-913`) and its 60-token budget from the token count (`:915`), and nine
    diagnose/probe tests repeat the filter (`:1483`, `:1587`, `:1694`, `:2793`, `:2959`, `:4588`,
    `:4724`, `:4817`, `:4924`).
11. **A token change is a replay miss.** `rank_key` (`dcg/sense_ranker.rs:129-152`) keys on each
    span's surface and candidate senses; `assert_replay_faithful` (`db_backed_encoding.rs:735-751`)
    fails on any miss. The recordings under `experiments/parsing/ranks/` and
    `demo/prose-to-formulas-v2/ranks.json` re-record once, after the reseed.
12. **Four test harnesses build a chain without the units layer**: `kernel/tests/lexicon_validates.rs:69-150`,
    `encoding_validates.rs:65-106`, `proposal_draw_round_trip.rs:60-95`, `witness_hash_agreement.rs`
    (~`:60-85`). A `units:` reference in `lexicon-ontology.esl`, `ontology.esl` or `closed-class.esl`
    does not resolve there.
13. **The skeleton eraser folds long numerals.** A run of four or more digits becomes `§`
    (`dcg/skeleton.rs:44-72`), so two readings that differ only in such a magnitude share a skeleton.

## Slice 1 — lexer and preprocessor, behaviour-preserving

**New modules**

- `dcg/lex.rs`: `pub fn lex(text: &str) -> Vec<Lexeme>`, `Lexeme { span: Range<usize>, class:
  LexClass }` over byte offsets into `text`, `LexClass = Word | Space | Punct`. Every byte is in one
  lexeme. A word is a maximal run of non-space, non-separator characters with its leading and
  trailing non-alphanumerics split off as `Punct` lexemes — so `BRCA1.` is `BRCA1` + `.`, `°C` is `°`
  + `C`, `0.56` and `WRN's` stay whole, and `-` stays inside a word. `—` `–` `‒` `―` `/`, brackets
  and `,` are single `Punct` lexemes.
- `dcg/preprocess.rs`: `pub fn preprocess(text: &str, lexemes: &[Lexeme]) -> Vec<Token>`,
  `Token { surface: String, span: Range<usize>, kind: TokenKind }`, `TokenKind = Word | Comma |
  NonProse` in this slice. It reproduces `tokenize` (`dcg/segment.rs:147-182`) decision by decision:
  bracketed asides and paired em-dash appositives (`:192-217`, including `a(b)c` → `ac` and the
  unclosed-opener drop, fixed in slice 2), separators, edge trimming, the comma token, dangling commas
  (`:174-179`), comma runs (`:180`). `NonProse` is today's `is_nonprose` (`:125-128`).
- `segment.rs` keeps `segment_sentences` (`:79-118`, unchanged by D95); `tokenize`, `is_nonprose`
  and `strip_bracketed_asides` leave it. The re-export (`dcg/mod.rs:130`) exports `lex`,
  `preprocess`, `Token`, `TokenKind`.

**Consumers take `&[Token]`** and read `.surface` where they read text today:

| consumer | anchor |
|---|---|
| `parse_packed_at_cap`, `parse_at_cap` | `dcg/parse/paths.rs:79`, `:256` |
| `parse_scoped_open_traced`, `routes_packed`, `parse_needs_unpacked` | `dcg/parse/mod.rs:633`, `:690`, `:652` |
| `recovery_ranks`, `all_prose_tokens_known`, `contextual_sense_ranks` | `dcg/parse/mod.rs:882`, `:954`, `:981` |
| `seed_leaves`, `distribute_head`, `split_coord_conjuncts` | `dcg/parse/seed.rs:568`, `:758`, `:1231` |
| `build_forest`, `drive_unpacked` | `dcg/chart/packed.rs:259`, `dcg/chart/unpacked.rs:42` |
| `binary_sites` and its triggers, `RightContext::after` | `dcg/rules/registry.rs:119-381`, `dcg/rules/mod.rs:69-79` |
| `ForestAttribution::attribute`, `forest_trace`, `attribution::record` | `dcg/chart/attribute.rs:115`, `dcg/chart/trace.rs:217`, `dcg/attribution.rs:55` |
| `augment_document_only` (tokenizes the whole document) | `dcg/augment.rs:227` |
| `unit_sense_names` | `dcg/verbalize.rs:66` |

`all_prose_tokens_known` filters on `TokenKind::Word` instead of `!is_nonprose`. Joined surfaces
(`seed.rs:588`, `parse/mod.rs:888`, `:994`) and hole names (`hole_base(i, j)`, `dcg/holes.rs:34`)
are unchanged, since positions and surfaces are.

**Tests**

- The old `tokenize` moves into `preprocess.rs`'s test module as an oracle.
  `preprocess(s, &lex(s))` surfaces equal the oracle's tokens over:
  - `segment.rs`'s eight tests (`:222-328`);
  - every sentence literal in `kernel/tests/closed_class_determiners.rs`;
  - `methods.txt` and `letter-body.txt` when present, `#[ignore]`d since they are gitignored.
- Lexer totality: the lexemes' spans tile the input.
- Migrated callers: `crates/eigenius-wordnet/tests/encoding_prototype.rs` (`encode_unit` `:117`,
  `encode_doc` `:151`, the non-ignored `prototype_classifies_a_text_document_into_the_four_outcomes`
  `:353`); `db_backed_encoding.rs` `encode_unit` (`:904-917`) and the nine filters (finding 10);
  `seed.rs` `rnr_tests` (`:1253-1298`); `attribution.rs:249`; `chart/trace.rs:314-486`.
- Stale comments corrected: `segment.rs:120` ("lowercased"), `kernel/tests/lexicon_validates.rs:981-982`
  ("the tokenizer lowercases").

**Chain:** none. **Done when** the oracle tests and the kernel, `eigenius-wordnet` and
`eigenius-encoding` test suites pass.

## Slice 2 — preprocessor rules that change behaviour

- **Numerals.** The lexer gains `LexClass::Numeral`: digits with an optional decimal part, a sign
  attached (`-1`, `−1` U+2212), and digit grouping (`1,200` — digits, comma, exactly three digits, no
  space). The preprocessor makes it `TokenKind::Numeral(Rational)`; today `1,200` is `1` `,` `200`.
- **Decision 3.** A digit-initial token that is not a numeral (`53BP1`) is a `Word`. `NonProse` is
  left with letterless non-numerals: `<`, `≤`, `>`, `≥`, `=`, `×`, `±`, an unmatched bracket.
- **Asides.** `(` directly after a word character is an argument (`log2(copy number)`): its content is
  kept as tokens and the brackets become `NonProse` tokens, so the sentence reaches no parse rather than
  a parse without the argument. `(` after a space is a gloss and is dropped, as today. Removing an aside
  leaves a separator (`a(b)c` → `a c`). An unclosed opener is a `NonProse` token and the rest of the
  sentence stays.
- **The widen gate.** A `NonProse` or (until slice 4) `Numeral` token seeds nothing, so its sentence
  cannot parse. `all_prose_tokens_known` (`dcg/parse/mod.rs:954-959`) returns `false` when one is
  present, which stops `widen` (`:823-825`) instead of exhausting the ladder (D95, "Numerals reach the
  parser and seed nothing").
- **The harness** classifies a unit with such a token as a new outcome, `NON-PROSE`, beside
  `MISSING-LEXEME` (`db_backed_encoding.rs:568-608`, `:904-983`). The summary line (`:4472-4480`), its
  parser in `scripts/eval-parse-rate.sh` (`:83-103`) and `baseline.json`'s `expected` block (`:37-50`)
  gain the count, informational. This is D95's requirement that a quantity gap and a syntax gap be
  distinguishable.
- **Measure** (needs the CNL page): the parse-rate run before and after, with every changed unit
  listed. An `#[ignore]`d kernel test prints the token-stream diff over `methods.txt` and
  `letter-body.txt`.

**Chain:** none.

## Slice 3 — the unit vocabulary, unit spellings, the recogniser

**Units layer** (`ontologies/units/units.esl`)
- `units:molar` (`M`, dimension `m^-3·mol`, factor `1000r`, prefixable) and `units:week` (`wk`, `s`,
  `604800r`, not prefixable), after `standard_gravity` (`:319-327`).
- `kernel/tests/units_layer.rs`: count 43 → 45 (`:87`), dimension and factor tables (`:112`, `:197`).
- `kernel/tests/unit_conversion.rs`: round-trip list (`:81`); `5 mM` → 5 mol·m⁻³; `2 wk` → 1209600 s.

**Spellings**
- `lexicon-ontology.esl`:
  - gains `namespace units`;
  - `class lexicon:UnitSurface { requires lexicon:unit_form, lexicon:unit; }`;
  - `property lexicon:unit_form : core:string`;
  - `property lexicon:unit : core:resource { class_types units:NamedUnit; domain lexicon:UnitSurface; }`.

  They sit beside `ReservedConstruct` (`:551-562`); `in_lexicon` (`:484-488`) is the pattern for a
  class-constrained resource property.
- `closed-class.esl` gains `namespace units` and the resources, beside the reserved constructs
  (`:2329-2361`). The methods need: `l`, `g`, `RCF`, `hour`/`hours`, `day`/`days`, `minute`/`minutes`,
  `week`/`weeks`, `ºC` (U+00BA) and `˚C` (U+02DA) for °C.
- The four partial-chain harnesses (finding 12) add the units layer before `lexicon-ontology.esl`.

**Kernel**
- `units/convert.rs` exposes prefix lookup (the `prefixes` map, `:78-81`) and the factor resolution
  `resolve_symbol` performs (`:354-377`), returning every `(prefix, unit)` pair instead of the first.
- `dcg/quantity.rs`:
  - **`ProseUnits::load(layer)`** reads `lexicon:UnitSurface` by type with `typed_resource_iris`
    (`layer/index.rs:92`), the way `ReservedTable::load` does (`dcg/reserved.rs:115-139`), and joins
    it to `Vocabulary::from_layer` (`units/convert.rs:186-232`).
  - **`ProseUnits::read(&str) -> Vec<UnitReading>`** normalises `µ` (U+00B5) to `μ`, superscript
    exponents (`ml⁻¹`), `per`, spaces and `·`, and resolves each factor to all its senses. For each
    combination it restates the strict form and converts it. `%` is a dimensionless factor 1/100, not a
    unit (D93).
- `Parser::over` (`dcg/parse/mod.rs:310-318`) builds `ProseUnits` beside `ReservedTable::load`. The
  preprocessor takes it as an argument, so `augment.rs` and `verbalize.rs` get it from the parser.
- **The recogniser** in `preprocess.rs`:
  - merges a numeral with the unit lexemes after it (`37` `°` `C`, `10` `μg` `ml⁻¹`), taking the
    longest run that reads as a unit expression;
  - splits an attached unit (`5mg`, `931g`), producing `TokenKind::Quantity { value, readings }`;
  - `5-fold`, `53BP1` and `HEK293T` are not quantities. A numeral with no unit stays `Numeral`.
- `has_token` (`dcg/parse/mod.rs:404-431`) and the harness treat `Quantity` as known.

**Tests** (`kernel/tests/quantity_tokens.rs`, over `testing::bootstrap_context()` as
`unit_conversion.rs:24-30` does):
- `931g` has two readings (grams, and 182599823/20000 m·s⁻²);
- `37 °C`, `2 h`, `5 mg/kg`, `10 μg ml⁻¹`, `5 mM`, `9 days` and `10%` each have one;
- `53BP1`, `HEK293T` and `5-fold` are not quantities;
- `Fig. 2d` is two days (decision 4, recorded as current behaviour).

**Chain:** `units`, `lexicon` and `closed-class` move; `EXPECTED`
(`kernel/tests/bootstrap_manifest_pinned.rs:185-206`) is updated in the same commit. The reseed waits
for slice 5.

## Slice 4 — `MP` in the grammar, and seeding

**The difference type**
- `units.esl`: `data units:Difference (u : core:unit) : Set { mk_difference : forall (u : core:unit)
  => core:rational -> core:integer -> units:Difference(u) }`, beside `Quantity` (`:36-40`).
- `units/convert.rs`:
  - `pub enum Reading { Value, Difference }`;
  - `convert_as(value, stated, reading)`, with the offset rule (`:272-278`) applied only for `Value`;
    `convert` stays `convert_as(…, Value)`;
  - `Converted::term(reading)` beside `quantity_term` (`:158-173`);
  - constants `DIFFERENCE`, `DIFFERENCE_FORM` beside `QUANTITY`, `QUANTITY_FORM` (`:48-51`).
- `esl/compile.rs`: `desugar_quantity` (`:1787-1862`) accepts `units:difference(v, "…")`.
- Tests: `unit_conversion.rs` gains `5 °C` → 5 K as a difference and 278.15 K as a value; ESL commits
  a `Difference(K)`.

**Categories** (`lexicon-ontology.esl`, `data lexicon:Cat` `:260-365`)
- `data lexicon:Reading { value, difference }`;
- `cat_mp : core:unit -> lexicon:Reading -> lexicon:Cat`;
- `cat_unit_forall : (core:unit -> lexicon:Cat) -> lexicon:Cat`.

**Kernel**
- `denote_cat` (`dcg/category.rs:34-154`):
  - `cat_mp(u, value)` → `Quantity(u)`;
  - `cat_mp(u, difference)` → `Difference(u)`;
  - `cat_unit_forall(λu. R)` → `Π u : EigonPrimitive(Unit). ⟦R⟧`, as the `cat_forall` arm (`:121-134`)
    builds `Π T : Set`.
- `unify_into` (`dcg/category.rs:292-354`) gains a `cat_mp` arm: a `Var` unit binds, a literal unit
  compares by equality, the reading compares by equality.
- `slot_is_concrete_nonentity` (`dcg/category.rs:468-475`) is true for `cat_mp` with a `LitUnit`, so a
  unit-selecting functor gets a `sel:` key.
- `cat_shape` and `cat_key` (`dcg/chart/forest.rs:229-358`) render `LitUnit` canonically (finding 5).
- A combinator `CombKind::UnitApply` (`dcg/rules/combinators.rs`):
  - modelled on `DepApply` (`:147-150`, `:258-288`, registered first in `comb_rules` `:297-335`);
  - left `cat_unit_forall(λu. body)`, where `body`'s argument slot unifies with the right item's
    `cat_mp(U, r)`, binding `u := U`;
  - sem `App(App(left.sem, LitUnit(U)), right.sem)`. The unit is applied explicitly because implicit Π
    is deferred (eigenius#261).

**Seeding** (`seed_leaves`, `dcg/parse/seed.rs:568-713`)
- A `Quantity` token's single-token span gets two items per reading, `Item::new(cat_mp(U, value),
  quantity term)` and `Item::new(cat_mp(U, difference), difference term)`.
- A `Numeral` token gets the same two items at the dimensionless unit `1`. A positive integer also gets
  the cardinal determiner items. Their templates are resolved from `lexicon:two_subj` and
  `lexicon:two_obj` (`closed-class.esl:2177-2192`) in `Parser::over`, the way `DetTemplates::resolve`
  (`dcg/grammar.rs:54-66`) resolves `a` and `these`; the count stays dropped, as for `two`..`ten`.
- The widen gate from slice 2 counts `Numeral` as seedable.

**Tests** (`kernel/tests/quantities_in_the_parser.rs`, fixture pattern of
`kernel/tests/comparative_than.rs:39-144`):
- `931g` seeds four items (two units × two readings) in four packed nodes; `forest.rs`'s node-key
  test (`:383-392`) gains the unit case;
- `unify_cat` binds a unit variable and refuses a literal mismatch;
- a fixture consumer at `cat_unit_forall(λu. …/cat_mp(u, value))` composes with `37 °C`;
- one at `cat_mp(u"K", difference)` composes with `5 °C` and refuses `5 mg`;
- packed equals unpacked on those sentences, as `packed_forest_equals_unpacked_on_core_grammar` does
  (`closed_class_determiners.rs:1804-1859`);
- `5 °C intervals` is not read by `kind_compound` (`dcg/rules/combinators.rs:742-760`), which takes two
  `cat_n` operands.

**Chain:** `units` and `lexicon` move.

## Slice 5 — consumers, the corpus, one reseed

**Relations** (`ontologies/ontology/ontology.esl`)
- One opaque relation per preposition that takes a value:
  `lexicon:Entity -> forall (u : core:unit) => units:Quantity(u) -> Prop`, beside `prep_at` and the
  others (`:68-89`).
- Six Rust sites match the `urn:eigenius:ontology:prep_` prefix and assume `Entity` arguments:
  `dcg/chart/attribute.rs:364`, `dcg/rules/constructions.rs:1220`, `dcg/verbalize.rs:552`, `:604`,
  `:762`, `:840`. The new relations take a prefix outside it, or each site is taught the quantity
  arity.

**Entries** (`closed-class.esl`)
- **VP adjuncts** at `cat_unit_forall(λu. ((S\NP)\(S\NP)) / cat_mp(u, value))` for `at`, `for`, `in`,
  `with` and `after`. Each has the six finiteness variants the NP adjuncts have (`in_prep`
  `:1126-1166`), sem `λu.λq.λV.λs. And(V(s), R(s, u, q))`.
- **Noun modifiers** at `cat_unit_forall(λu. cat_pp / cat_mp(u, value))` for `of`, `with` and `at`, as
  `in_nmod` (`:1468-1475`), sem `λu.λq.λx. R(x, u, q)`.
- `after` joins `PREPOSITIONS_AND_CONJUNCTIONS` (`dcg/closed_class.rs:36-40`), so the importers stop
  emitting content entries on it; that moves the imported lexicon, inside this slice's reseed.
- **The prenominal measure phrase** (`10 μM etoposide`, `0.1% crystal violet`), 40 occurrences:
  - a value item also seeds a predicative-adjective item, `S[dcl,adj]\NP`
    (`predicative_adjective_cat`, `dcg/category.rs:585`), with sem `λx. R_measured(x, U, q)`;
  - the leaf `mod_lifts` (`dcg/parse/seed.rs:679-705`) make it a prenominal modifier, and the copula
    takes it predicatively (`the temperature was 37 °C`).
- **Tests:** no entry takes both readings (a scan over closed-class entries whose category mentions
  `cat_mp`); a °C sentence per consumer.

**The corpus**
- `experiments/parsing/quantities/`: CNL-register sentences derived from the WRN methods, each annotated
  with its expected consumer. The methods text itself is gitignored and not CC-licensed.
- A kernel test parses them over the bootstrap lexicon and a fixture of their content words, so grammar
  coverage is checked without a database.

**One reseed** for slices 3–5 (`scripts/reseed-lexicon-db.sh --umls-all`, the prerequisites in the
reseed memory), then:
- `scripts/build-alignment-snapshot.sh`;
- re-record the sense ranks and selections (finding 11; `experiments/parsing/README.md:12-176`);
- the parse-rate run on the CNL page and the quantity corpus, and the re-established `baseline.json`;
- the style guide's 🔜 quantity rows made current (finding 7).

## Slice 6 — arguments and standards, on evidence

Type-indexing `cat_pp_arg` and `cat_pp_than` (finding 1), when a corpus sentence needs a measure
phrase as a governed argument or a comparison standard; none of D95's examples does.
- `cat_pp_arg` is emitted at six sites in the WordNet importer (`crates/eigenius-wordnet/src/convert.rs:250`,
  `:260`, `:740`, `:1221`, `:1241`, `:1313`) and none in UMLS, so the change moves the imported lexicon.
- `unify_into`'s `cat_pp_arg` arm (`dcg/category.rs:327-331`) and the two `denote_cat` arms change
  with it.

## Slice 7 — degree semantics

- The interval-arithmetic category, distinct from `cat_measure` (`dcg/category.rs:99-104`).
- Differential comparatives (`5 °C warmer`) with exact-degree templates, over the comparative
  machinery in `kernel/tests/comparative_than.rs`.
- The adjective-adjunct `by`, beside `by_agent` (`closed-class.esl:577`) and `by_nmod` (`:2449`).
- Scalar-change verbs (`rose 5 °C`) through `m^Δ`, and the verb's scale orientation.

These take difference items. The typed arithmetic of decision 5 lands here, as axioms reduced on
literals in the way `units:mul` is (`nbe/unit_ext.rs`).

## Out, as D95 decides

Ranges and intervals; `every N unit`; the tolerance construction and the vector-denoting PP;
statistic routing to D52; the precision reading of dispersion.
