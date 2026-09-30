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

**All six are closed** (2026-09-26): 1, 2, 3, 5 and 6 decided, 4 deferred to D96's build.

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
6. **An attached quantity keeps its word reading** (decided 2026-09-26, after slice 3 measured the
   attached form). A token whose digits carry its unit — `931g`, `5A`, `2d` — is a quantity *and* the
   word it spells: seeding looks its surface up in the lexicon, as for any token, beside its quantity
   items, and the grammar and the felicity gate choose. The grammar refuses a quantity where no
   measure phrase composes, so a known identifier survives (`293T cells`, `2D` for two-dimensional,
   `McCoy's 5A` as a multiword form). Figure panels are D96's (decision 4).
   - **Evidence.** Of the 107 digit-initial tokens with letters in the WRN texts, one is an attached
     quantity (`931g`); the rest are figure panels, ordinals and names. Slice 3 read four as
     quantities: `931g`, and wrongly `McCoy's 5A`, `2d`, `8d`.
   - **Accepted costs.** A quantity token always seeds, so an unknown attached identifier (`5A`) is
     not reported as a missing word, augmentation does not try to ground it, and a sentence it
     blocks is counted a grammar gap. Where the quantity is its only reading and a measure phrase
     composes, the wrong reading can win.
   - **Rejected:** refusing an attached single uppercase letter (`5A`, `293T`, `12V`) by rule. It
     keeps a failure visible, and it drops the word-or-quantity choice for the chart to make.

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

## Slice 1 — lexer and preprocessor, behaviour-preserving — built

**New modules**

- `dcg/lex.rs`: `pub fn lex(text: &str) -> Vec<Lexeme>`, `Lexeme { span: Range<usize>, class:
  LexClass }` over byte offsets into `text`. `LexClass` is `Word` (a maximal run of
  `char::is_alphanumeric`), `Space` (a maximal run of `char::is_whitespace`) or `Punct` (one character
  that is neither). These are the classes the old edge trimming used, so `0.56` is three lexemes, `°C`
  two, and the preprocessor rejoins them. The spans tile the input.
- `dcg/preprocess.rs`: `pub fn preprocess(text: &str, lexemes: &[Lexeme]) -> Vec<Token>`, and
  `pub fn tokenize(text: &str) -> Vec<Token>` composing the two — the entry point keeps its name and
  now returns tokens.
  - `Token` has private fields read through `surface()`, `span()` and `kind()`; `TokenKind` is
    `Word | Comma | NonProse`.
  - It makes the old tokenizer's five decisions in its order: asides (including `a(b)c` → `ac` and
    the unclosed-opener drop, fixed in slice 2), paired em-dash appositives, separators, edge trimming,
    commas. A run of non-separator lexemes becomes a token spanning its first `Word` lexeme to its last.
  - `NonProse` is the old `is_nonprose` rule.
  - `join_surfaces(&[Token]) -> String` replaces the `tokens[i..=j].join(" ")` joins.
- `segment.rs` keeps only `segment_sentences`. `dcg/mod.rs` re-exports `lex`, `Lexeme`, `LexClass`,
  `preprocess`, `tokenize`, `join_surfaces`, `Token` and `TokenKind`; `is_nonprose` is gone.

**Consumers take `&[Token]`** and read `surface()` where they read text:

| consumer | anchor |
|---|---|
| `parse_packed_at_cap`, `parse_at_cap` | `dcg/parse/paths.rs:79`, `:256` |
| `parse_scoped_open_traced`, `routes_packed`, `parse_needs_unpacked` | `dcg/parse/mod.rs:633`, `:690`, `:652` |
| `recovery_ranks`, `contextual_sense_ranks` | `dcg/parse/mod.rs:882`, `:981` |
| `seed_leaves`, `distribute_head`, `split_coord_conjuncts` | `dcg/parse/seed.rs:568`, `:758`, `:1231` |
| `build_forest`, `drive_unpacked` | `dcg/chart/packed.rs:259`, `dcg/chart/unpacked.rs:42` |
| `binary_sites` and its triggers, `RightContext::after` | `dcg/rules/registry.rs:119-381`, `dcg/rules/mod.rs:69-79` |
| `ForestAttribution::attribute`, `forest_trace`, `attribution::record` | `dcg/chart/attribute.rs:115`, `dcg/chart/trace.rs:217`, `dcg/attribution.rs:55` |
| `augment_document_only` (tokenizes the whole document) | `dcg/augment.rs:227` |
| `unit_sense_names` (now skips the comma token, which it used to look up as `""`) | `dcg/verbalize.rs:66` |

- **`Parser::unknown_words(text, lemmatizer)`** is the missing-lexeme signal, defined once: the word
  tokens with no entry. `all_prose_tokens_known` is `unknown_words(…).is_empty()`. The twelve filters
  in `db_backed_encoding.rs` (finding 10 counted ten) and `encoding_prototype.rs`'s `encode_unit` call
  it instead of repeating `tokenize` + `is_nonprose` + `has_token`.
- `split_coord_conjuncts` returns the conjuncts' joined surfaces, which its one caller joined anyway.
- Joined surfaces and hole names (`hole_base(i, j)`, `dcg/holes.rs:34`) are unchanged, since
  positions and surfaces are.

**Tests**

- `preprocess.rs` holds the old `tokenize` verbatim as an oracle. The preprocessed surfaces and kinds
  equal it on every string of length ≤ 4 over a 22-character alphabet of every class the old code
  treated differently (≈245,000 strings), on 20,000 random strings up to length 40, and on prose
  covering asides, appositives, units and statistics. An `#[ignore]`d test runs it over `methods.txt`
  and `letter-body.txt`, whole and sentence by sentence; it passes on this checkout.
- `lex.rs`: the spans tile the input.
- `segment.rs`'s tokenizer tests moved to `preprocess.rs`; `rnr_tests`, `attribution.rs` and
  `chart/trace.rs` build their tokens with `tokenize`.
- Stale comments corrected: `segment.rs`'s "lowercased", `kernel/tests/lexicon_validates.rs`'s
  "the tokenizer lowercases".

**Chain:** none.

## Slice 2 — preprocessor rules that change behaviour — built

All in `dcg/preprocess.rs`; the lexer is unchanged.

- **Brackets.** Each closer pairs with the latest open opener, as before.
  - A pair whose opener directly follows a `Word` lexeme is an **argument** (`log2(copy number)`,
    `poly(ADP-ribose)`): its content is kept, and its brackets become `NonProse` tokens, so the
    sentence reaches no parse rather than a parse without the argument.
  - Any other pair is a **gloss** and is dropped, leaving a separator (`x.(y)z` → `x` `z`).
  - An unmatched opener or closer is a `NonProse` token, and the text around it stays. The unclosed
    opener used to drop the rest of the sentence.
- **Numerals.** `TokenKind::Numeral(Rational)`: digits, grouped by commas in threes or not, an
  optional decimal part, and a sign (`-`, `−` U+2212) directly before. A comma between a digit group
  and exactly three digits groups them (`1,200`); any other comma separates, as before. Found in the
  preprocessor rather than the lexer, since telling a grouping comma from a list comma is a decision.
- **Operators.** `<`, `>`, `≤`, `≥`, `=`, `≠`, `≈`, `~`, `±`, `×` and `−` at the edge of a token, or
  standing alone, become `NonProse` tokens instead of being trimmed away; inside a token they stay in it
  (`P<0.05`).
- **Kinds (decision 3).** A token that starts with a digit and is not a numeral (`53BP1`, `5-fold`,
  `1a`, `10⁻¹³`, `45-60`) is a `Word`. A token with no ASCII letter is `NonProse`.
- **The widen gate.** `every_token_seeds` (was `all_prose_tokens_known`) requires every non-comma
  token to have an entry, so a numeral or symbol that seeds nothing stops `widen` on the first attempt.
  `Parser::unseedable_tokens(text, lemmatizer)` lists those tokens; `unknown_words` still lists only
  words.
- **The harness.**
  - `db_backed_encoding.rs` and `encoding_prototype.rs` gain the outcome `NON-PROSE`, checked after
    `MISSING-LEXEME`, and the summary line a `non-prose` count.
  - `scripts/eval-parse-rate.sh` reads it (0 on older logs) and puts it in the coverage gate:
    `grammar-gap 0, missing-lexeme 0, non-prose 0`.
  - `baseline.json` is unchanged: the gate is absolute in the script, and no run on the CNL page
    backs a new number. `experiments/parsing/README.md`'s outcome table has the row.
- **Augmentation** (`dcg/augment.rs`) looks only at word tokens for OOV gaps; it used to report every
  number as a word to ground.

**Tests.**
- `preprocess.rs` has a test per rule.
- The legacy tokenizer is an oracle over an alphabet with no digit, bracket or operator (every string
  of length ≤ 4 and 20,000 random ones), where slice 2 changes nothing.
- `kernel/tests/closed_class_determiners.rs` `numerals_and_symbols_are_unseedable_not_missing`:
  `unknown_words` is `[53BP1]`, `unseedable_tokens` is `[37, <, 5]`, and a sentence with a numeral
  fails on its first attempt, where the old gate widened through every rung.

**Measured.** The CNL page is not in this checkout, so the parse-rate run is not done. The `#[ignore]`d
`list_the_wrn_sentences_slice_2_changes` lists token-stream changes against the legacy tokenizer: 31
of 296 methods sentences and 3 of 105 letter sentences change surface. Every sentence with a digit
also changes kind, and until slice 4 seeds numerals it is `NON-PROSE`. The changes:
- **Arguments kept.** `log(counts)`, `log(intensity)` ×2, `poly(ADP-ribose)`. Also `negative
  control(s)`, where the plural marker now stops the parse, and the OCR'd `anti(Cell Signaling …)`.
- **Signs and operators.** `protein levels <−1` was `levels 1`; now `<` and the numeral `−1`. `×` in
  `1 × 10^6` and `63× magnification` is a token. The U+2212 in the OCR'd `annexin V− FITC` is an
  operator token; as a marker (`CD8−`) trimming it was the distortion.
- **Digit grouping.** `1,000`, `2,000`, `12,000` are numerals, not `1 , 000`.
- **Unmatched brackets.** Sentences `segment_sentences` splits inside a parenthesis — at `(Chr.`, at
  `Extended Data Figs.` and inside URLs — keep their text and carry a bracket token, where they used
  to lose everything after the opener.

**Chain:** none.

## Slice 3 — the unit vocabulary, unit spellings, the recogniser — built

**Data**
- `ontologies/units/units.esl`: `units:molar` (`M`, `m^-3·mol`, `1000r`, prefixable) and `units:week`
  (`wk`, `s`, `604800r`, not prefixable), after `standard_gravity`.
- `lexicon-ontology.esl` gains `namespace units`, `class lexicon:UnitSurface`, `lexicon:unit_form` and
  `lexicon:unit` (`class_types units:NamedUnit`), beside `ReservedConstruct`.
- `closed-class.esl` gains `namespace units` and 14 spellings: `l`; `g` and `RCF` for standard
  gravity; `hour`, `hours`, `day`, `days`, `minute`, `minutes`, `week`, `weeks`; `ºC`, `˚C`, `℃`.
- The four partial-chain harnesses (finding 12) pass unchanged: an unresolved `units:` reference is
  not a compile error there.

**Kernel**
- `units/convert.rs`: `Vocabulary::units()` and `prefixes()`.
- `dcg/quantity.rs`:
  - `ProseUnits::load(layer)` joins the strict symbols to the `UnitSurface` spellings,
    case-sensitive; `ProseUnits::none()` reads no units.
  - `ProseUnits::read(text, at, attached, value)` returns the longest unit expression with a
    reading, and every reading as a `UnitReading { units, stated, value }`, restated in the strict
    form and converted by `Vocabulary::convert`.
  - What reads, with the reasons from the corpus:
    - A factor is a run of letters (`°`, `˚`, `℃` included) with an optional exponent (`²`, `⁻¹`,
      `^2`, `^(1/2)`), not followed by a letter or digit. It resolves exact-first, then longest
      prefix on a prefixable spelling. `µ` reads as `μ`.
    - Factors join by `·`, by `/` or ` per ` (once), or by whitespace only when the next factor has
      an exponent (`μg ml⁻¹`). Otherwise `2 h at 37 °C` reads `h at` as hour·attotonne.
    - A closed-class word is never a factor (`at`, `as`, `am`).
    - `′` and `″` are not unit characters: after a numeral they write DNA ends (`5′`, `3′`).
    - `%` alone is 1/100 (D93), stated `1`.
- `dcg/preprocess.rs`, decision 7:
  - A `Numeral` and the unit read after it, or a digit-initial `Word` whose digits carry the unit
    (`931g`), become one `TokenKind::Quantity(Quantity { value, readings })`.
  - The unit is read from the text through the tokens' spans, because edge trimming has dropped the
    `°` and the `%`.
  - The expression must end where a token ends, so `5′-UTR` stays a word.
  - `Token` loses `Eq`, since `Converted` has none.
- **Tokenizing is the parser's.** `Parser::over` builds `ProseUnits`, and `Parser::tokenize(text)` is
  the tokenization every consumer reads: seeding, the widen gate, `unknown_words`,
  `unseedable_tokens`, augmentation, verbalization and both harnesses. The free function is
  `tokenize(text, &ProseUnits)`; tests that need no units pass `ProseUnits::none()`.
- **A quantity token seeds nothing until slice 4**, so `unseedable_tokens` reports it and the harness
  counts its unit `NON-PROSE`. *The plan said `has_token` treats `Quantity` as known here; that waits
  for the seeding.*

**Tests**
- `kernel/tests/quantity_tokens.rs`:
  - the centrifugation sentence (`931g` with two readings, `2 h`, `30 °C`);
  - an inventory of 13 (`10 μg ml⁻¹`, `5 mg/kg`, `5 mg per kg`, `5 mM`, `10 µM`, `9 days`,
    `2 weeks`, `10%`, `0.2 ml`, `37 ºC`, `−80 °C`, `931 RCF`, `2 °C/min`);
  - eight non-quantities (`53BP1`, `HEK293T`, `5-fold`, `3′ end`, `5′-UTR`, `12 cells`,
    `chromosomes 3 and 5`, `96-well`);
  - `Fig. 2d` as two days (decision 4);
  - the parser's tokenization, with the quantity unseedable.
- `units_layer.rs` and `unit_conversion.rs` count and convert the molar and the week.

**Measured.** The `#[ignore]`d `list_the_quantities_in_the_wrn_texts` reads 69 quantities, 67 in the
methods and 2 in the letter. By stated unit: `%` 18, `h` 11, `d` 11, `mL` 11, `min` 6, `°C` 3, `mm` 2,
and one each of `g`/`g_n` (`931g`), `mM`, `mg`, `ng`, `s`, `wk` and `A`.
- **Wrong.**
  - `McCoy's 5A`, a culture medium, reads as 5 A.
  - `2d` and `8d` are figure panels (decision 4).
  - `6 s` comes from the OCR's letter-spaced `R P S 6 s h R NA`.
  - So the attached form is right once of four in this corpus: `931g`.
- **Missed.**
  - Quantities inside parenthetical glosses — `(60 mM KCl)`, `(150 mM NaCl, …)`,
    `(1,200 V, 20 ms, 2 pulses)` — are dropped with the gloss, as before.
  - This OCR text has lost `μ` and superscripts (`10 ml of gentamicin` was `10 μg ml⁻¹`), so D95's
    13 `μg ml⁻¹` are not in it.

**Chain:** `units`, `lexicon` and `closed-class` moved; `EXPECTED` is updated. The reseed waits for
slice 5.

## Slice 4 — `MP` in the grammar, and seeding — built

**The difference type**
- `units.esl`: `data units:Difference (u : core:unit) : Set { mk_difference : … }`, beside `Quantity`.
- `units/convert.rs`:
  - `pub enum Reading { Value, Difference }`;
  - `convert_as(value, stated, reading)`, applying the °C offset only to a `Value`; `convert` is
    `convert_as(…, Value)`;
  - `Converted` gains `reading`, and `term()` builds `mk_quantity : Quantity(u)` or
    `mk_difference : Difference(u)`, replacing `quantity_term()`;
  - `DIFFERENCE`, `DIFFERENCE_FORM`.
- `esl/compile.rs`: `desugar_quantity` resolves `units:difference(v, "…")` beside `units:quantity`.
- `dcg/quantity.rs`: each `UnitReading` carries both conversions, `value` and `difference`.

**Categories** (`lexicon-ontology.esl`)
- `data lexicon:Reading { value, difference }`;
- `cat_mp : core:unit -> lexicon:Reading -> lexicon:Cat`;
- `cat_unit_forall : (core:unit -> lexicon:Cat) -> lexicon:Cat`.
- `Mood`'s comment and description, "the only feature that alters ⟦·⟧", now name `Reading` as the
  other.

**Kernel**
- `denote_cat`: `cat_mp(u, value)` → `Quantity(u)`, `cat_mp(u, difference)` → `Difference(u)`,
  `cat_unit_forall(λu. R)` → `Π u : unit. ⟦R⟧`.
- `unify_into` gains a `cat_mp` arm: a unit variable binds occurs-consistently (`unify_unit`), a
  literal unit matches only itself, the reading matches exactly.
- `measure_phrase_cat(layer, unit, reading)` builds the category seeding uses.
- `cat_shape` and `cat_key` render `LitUnit` (`u"K"`). *`slot_is_concrete_nonentity` is unchanged:
  with the unit in the ordinary key, functors whose slots name different units already fall into
  different nodes, so the `sel:` key adds nothing for units.*
- `CombKind::UnitApply`, second in `comb_rules`: `cat_unit_forall(λu. A/B)` with `cat_mp(U, r)` on its
  right binds `u := U`, unifies `B`, and builds `L U R`.

**Seeding** (`seed_leaves` → `measure_items`)
- A `Quantity` token seeds, per unit reading, `cat_mp(U, value)` with the value term and
  `cat_mp(U, difference)` with the difference term. `931g` seeds four.
- A `Numeral` seeds the same pair at the dimensionless unit. `1` also seeds the cardinal determiner
  items of the closed-class `one`, and an integer of 2 or more those of `two` (`in_lexicon` none, sense
  `one`/`two`), resolved once in `Parser::over`, so `3 genes` reads as `three genes` does and `1 gene`
  as `one gene`. `0` seeds no determiner: `∃` would misread `0 genes`.
- `closed-class.esl` gains `one_subj` and `one_obj`, the singular cardinal, shaped like `a_subj` and
  `a_obj` — `one gene affects HeLa` had no determiner reading before. Like `two`..`ten`, `one` is not
  on the importers' closed-class list, so WordNet's noun and adjective senses of it stay.
- `seeds_itself` makes `Numeral` and `Quantity` tokens seeding for the widen gate and
  `unseedable_tokens`. A sentence with a number is no longer `NON-PROSE`; it parses or is a grammar
  gap, by its consumers.
- Decision 6 needs no code: `lookup_span` looks up the quantity token's surface, as every span's.

**Tests**
- `kernel/tests/quantities_in_the_parser.rs`, over a fixture with a unit-polymorphic VP-adjunct `at`
  (value), a verb `rose` taking `cat_mp(u"K", difference)`, and `5A` as a known name:
  - `HeLa incubated at 37 °C` has one reading, at 6203/20;
  - `at 931g` has two, 931/1000 and 182599823/20000;
  - `HeLa rose 5 °C` is a difference of 5, without the offset, and `HeLa rose 5 mg` has no parse;
  - `5A incubated` reads the name (decision 6);
  - packed equals unpacked on five sentences.
- `category.rs`: a unit variable binds, a literal mismatch or reading mismatch is refused, and
  `denote_cat` gives `Quantity` and `Difference`.
- `forest.rs`: two measure phrases of different units have different node signatures.
- `closed_class_determiners.rs`: `2 genes`, `3 genes` and `1 gene`, as subject and object, read as their
  word forms do; `one gene` parses and `one genes` does not; `0 genes` has no reading. The gate test
  now uses a symbol, since a numeral seeds.
- `unit_conversion.rs`: `units:difference(5, "°C")` commits as a `Difference(K)` and a value in that
  slot is refused; a difference never takes the offset; the difference term type-checks.
- The N-N kind compound cannot see a quantity: it combines two `cat_n`, and a quantity seeds only
  `cat_mp`. The value test asserts no `compound_kind`.

`pretty_term` prints literals as `<term>`, and normalisation erases the term's annotation, so the
tests read magnitudes from the sem's debug form; the unit is carried by the category.

**Chain:** `units`, `lexicon` and `closed-class` moved; `EXPECTED` is updated. The reseed waits for
slice 5.

## Slice 5 — consumers, the corpus, one reseed — built and reseeded; baselines owed

**Relations** (`ontologies/ontology/ontology.esl`)
- `prep_{at,for,in,with,after,of}_value : lexicon:Entity -> forall (u : core:unit) => units:Quantity(u) -> Prop`,
  one per preposition that takes a value. The unit is an explicit argument because implicit Π is
  deferred (#261).
- `has_quantity`, same type, for the prenominal and predicative measure phrase. Which quantity of the
  entity it is (a concentration, a temperature) the parse does not say.
- `prep_after : Entity -> Entity -> Prop`, for `after` over an NP.
- The six Rust sites that match the `prep_` prefix: `is_pp_refined` and `attribute.rs`'s
  `axiom_class` classify by name only, so a `_value` relation counts as a PP there, which it is.
  `verbalize.rs`'s four sites go through `prep_parts`, which reads `prep_X(subj, obj)` and
  `prep_X_value(subj, unit, quantity)` and renders the quantity as `6203/20 K` (`quantity_text`).
  `has_quantity` renders as `x is q` predicated, before its noun in a restrictor, and as
  `has-quantity q` in the expanded register.

**Entries** (`closed-class.esl`)
- 30 VP adjuncts, `at`, `for`, `in`, `with` and `after` in the six finiteness variants, at
  `cat_unit_forall(λu. ((S\NP)\(S\NP)) / cat_mp(u, value))`, sem `λu.λq.λV.λs. And(V(s), R(s, u, q))`.
- 3 noun modifiers, `of`, `with` and `at`, at `cat_unit_forall(λu. cat_pp / cat_mp(u, value))`, sem
  `λu.λq.λx. R(x, u, q)`.
- 8 SemTerms serve them.
- `after` joins `PREPOSITIONS_AND_CONJUNCTIONS`, so the importers stop seeding its content homonyms.
  Nothing gave `after` an NP reading, so it also gets the six VP adjuncts and the noun modifier over
  an NP that `within` has, over `prep_after`.
- *`at` still has no VP adjunct over an NP (`stored at the core facility`); unchanged here.*

**The prenominal and predicative measure phrase** (`measure_items`)
- Each value reading of a quantity token also seeds `S[dcl,adj]\NP` with sem
  `λx. has_quantity(x, U, q)`. The leaf `mod_lifts` make it prenominal (`10 μM etoposide`), and the
  copula takes it (`the temperature was 37 °C`).
- A numeral seeds no such item: `5 cells` stays a cardinal.

**The tokenizer, corrected on the corpus**
- *A range read wrong.* Slice 4's numeral seeding turned `4–12% gels` into the cardinal `4` over
  `12% gels`. An en-dash between digits now joins a range, one `NonProse` token (`is_range`), so the
  sentence has no parse until ranges are in.
- *`1,000g` split at the comma.* `groups_digits` required the next lexeme to be three digits;
  it now requires it to start with exactly three, and `numeral_prefix` reads digit groups, so an
  attached unit after a grouped numeral reads.

**Sentence splitting** (`segment_sentences`), measured on the WRN methods and letter
- Before: 401 segments, 12 with unbalanced brackets (six splits inside parentheses: three URLs,
  two `Figs.`, one `Chr.`).
- A `.`, `!` or `?` inside an open parenthesis or bracket does not end a sentence: −10 segments.
- A `.` directly followed by a letter or digit is word-internal (`DepMap.org`, `pLKO.1`,
  `Chr.2-2`, the DOI), and an initialism (`r.p.m.`, `s.e.m.`: every dot-separated part one letter)
  is an abbreviation unless an uppercase start follows, generalizing the single-letter rule: −31.
- `figs` joins `ABBREV`: −4.
- After: 356 segments, 0 unbalanced. Every merge in the diff is a false boundary.
- The `#[ignore]`d `list_the_wrn_segments_with_unbalanced_brackets` reruns the count.

**Tests**
- `quantities_in_the_parser.rs` uses the bootstrap entries (the fixture's own `at` is gone):
  - one relation per preposition (`for 2 h`, `after 72 h`, `in 50 μl`, `with 10%`, stacked
    `at 37 °C for 2 h`);
  - `a dose of 5 mg/kg`;
  - `10 μM etoposide` prenominal and `the temperature was 37 °C` predicative;
  - `after the dose`;
  - no closed-class entry takes a measure phrase at a reading variable: 33 take `value`;
  - packed equals unpacked on nine sentences.
- `verbalize.rs`: a measured value predicated, after a preposition, before its noun, and expanded.
- `segment.rs`: a parenthetical, a host name, an initialism, `Figs.`.
- `preprocess.rs`: ranges are non-prose; `1,000g` is one token. `quantity_tokens.rs`: `1,000g` has the
  two `g` readings.

**The corpus** (`experiments/parsing/quantities/`)
- `corpus.tsv`: 28 CNL-register sentences derived from the WRN methods and D95's examples. A covered
  row names the relations every reading contains and the values every reading renders; a gap row
  names the missing construction and its owner.
- `content-words.esl`: the nouns and verbs around them, in the importer's shapes and naming.
- `kernel/tests/quantity_corpus.rs` checks it without a database: every word is known, 22 covered
  rows pass (one reading each, two for the two `g` rows), and 6 gap rows still have no parse.
- Gaps: a measure phrase modifying a PP (`72 h after transduction`), a pseudo-partitive
  (`10 μg ml⁻¹ of colcemid`), `every N unit`, a range. And two that are not D95's: a fronted PP adjunct
  and a coordinated NP as an adjunct preposition's object, neither of which parses without quantities.

**Chain:** `ontology` and `closed-class` moved; `EXPECTED` is updated, with one history entry for
D95's four layers.

**Reseeded** (`2026-09-27`, at `6a3eabf`): `wordnet-umls-2026-09-27` and
`wordnet-umls-aligned-2026-09-27`. WordNet: 466,117 entries, 43,474 of them mass entries from the
countability lexicon, 92 withheld on closed-class surfaces.

**Measured over the full lexicon**, cap-only:
- The quantity corpus: 3 encoded, 17 ambiguous, 7 grammar gaps, 1 non-prose. Three of the grammar
  gaps are covered rows whose verbs the lexicon lacks in the passive: WordNet has `incubate` only as
  intransitive, and no verb `electroporate`. One gap row parses, because the imported `treat` governs
  `with`. The corpus README has the breakdown.
- The style guide's quantity rules, checked sentence by sentence and made current (finding 7); only
  differences stay 🔜, for slice 7.

**Measured and re-baselined** (`5fb85cf`, 2026-09-29): the sense ranks and selections re-recorded
against `wordnet-umls-aligned-2026-09-29-quantities`, the live draw replayed with every figure equal,
and `baseline.json` rewritten from the replay: 62/62 expected hits, 0 grammar gaps, 626 readings
(674 before), 175 skeletons.

## Slices 6–9 — what the paper attests (rewritten 2026-09-29)

The first slices 6 and 7 built what D95's examples anticipated: a measure phrase as a governed
argument or a comparison standard, and unit differences (`rose 5 °C`, `5 °C warmer`). Neither version
of the paper has either. It has other constructions, and the owner's rule applies: a construction
the paper uses once will recur in the next paper, so it is built. Counts from
`experiments/parsing/measure-quantities.py --shapes` over the Nature text, the author manuscript in
brackets, figure references removed. Slices 6–9 cite `2abed42`.

| Shape | Nature [AM] | Examples | Slice |
|---|---|---|---|
| a bound in words | 7 [3] | `less than one count per million`, `at least 1,000 cells`, `more than one MMR gene`, `more than half of the samples` | 6; hosts in 7, 8 |
| a bound in symbols, running text | 11 [7] | `P < 2 × 10⁻¹⁶`, `>90% infection efficiency`, `≥ 8 foci` | 6 |
| a bound in symbols, in parentheses | 9 [9] | `(P < 2 × 10⁻¹⁶, …)`, `(log2(mRNA expression in transcripts per million) < 1)`, `(<85%)` | 6 |
| an approximation | 7 [3] | `around 100 mm3`, `~100 mm3`, `approximately 8-week-old`, `~5 minutes` | 6 |
| a range with a unit or `%` | 7 [6] | `20–30%`, `45-60%`, `80–90% confluence`, `every 2–3 days`, `4–12% gels` | 6 |
| a measure phrase + `of` + noun | 5 [12] | `300 µl of CellTiter-Glo` (pseudo-partitive), `15% of colon cancers` (proportion) | 8, 7 |
| a measure phrase before a PP | 20 [8] | `9 days after shRNA induction`, `6 h before collection` | 8 |
| `every` + number | 5 [5] | `every 3 days`, `every 2–3 days` | 8 |
| `per` + noun | 42 [20] | `2 × 10³ cells per well`, `count per million`, `cells per sample` | 8 |
| `N-fold` | 1 [1] | `a median 0.56-fold fewer deletion mutations … compared to typical-lineage MSI models` | 9 |

The patterns approximate. `--shapes --verbose` prints every instance; read the bucket before citing
its count. Most `per` hits are seeding densities and figure-axis labels; most parenthesized symbols
are P values, but not all — `(log2(…) < 1)` defines a threshold the argument depends on.

**Not built, on evidence: the first slices 6 and 7.** Type-indexing `cat_pp_arg` and `cat_pp_than`
for a governed or standard measure phrase (finding 1); the interval-arithmetic category, differential
comparatives, the adjective-adjunct `by`, and scalar-change verbs over differences. None occurs in
either version of the paper. D95's design for them stands for when one does.

## Slice 6 — bounds, approximations and ranges

A bound is a constraint on a measure phrase's value, not a value (D95, "Ranges need no new
semantics"): `less than 37 °C` denotes `λq. lt(q, 37 °C)`. Each construction here builds one, and a
consumer then quantifies over its value slot.

**Decisions**

1. **A constraint category, indexed like `cat_mp`.** `cat_mpc : core:unit -> lexicon:Reading -> Cat`,
   with `⟦cat_mpc(u, value)⟧ = units:Quantity(u) -> Prop` and
   `⟦cat_mpc(u, difference)⟧ = units:Difference(u) -> Prop`. The unit index keeps a unit error a type
   error, and packing keys it exactly (finding 5), as for `cat_mp`.
   - *Rejected:* an interval type. Every consumer relation would need an interval-valued twin, and an
     approximation is not an interval.
   - *Rejected:* a third `Reading`. A bound constrains a value or a difference, so it is orthogonal to
     the reading.
2. **Three relations, declared in the units layer.** `units:lt`, `units:le` and `units:approx`, each
   `forall (u : core:unit) => units:Quantity(u) -> units:Quantity(u) -> Prop`. `more than b` is
   `lt(b, q)` and `at least b` is `le(b, q)`: direction is argument order, so a bound written
   `more than` and one written `>` are one proposition. `approx` is opaque, since the paper states no
   tolerance. None is reduced on literals: the parser needs the relations to state a bound, and
   deciding one is checking work, beside the typed arithmetic. Differences get relations when a bound
   on a difference is attested.
   - *Rejected:* `stats:lt` and `stats:le` (`statistics.esl:327-330`), which take `core:float`; a
     quantity is an exact rational with a unit.
3. **One rule lets every consumer take a constraint.** `[Πu. X / cat_mp(u, r)] [cat_mpc(U, r)] →
   X[u := U]` — `UnitApply` (`combinators.rs:297-310`) with a constraint in place of the value. The
   sem quantifies the value: for a consumer `f`, `λa₁…aₙ. ∃q. C(q) ∧ f(U)(q)(a₁)…(aₙ)`, n the arity of
   `⟦X⟧`. The grammar already scopes a quantified object over a preposition's slot this way
   (`gq_prep_vpadjunct`, `combinators.rs:1353-1375`); this generalizes it to the consumer's arity. The
   ∃ is the determiners' (`exists_sem`, `closed-class.esl:31-37`), which verbalization reads as `a`.
   - One rule reaches the 33 consumers built and each one slices 7 and 8 add.
   - *Rejected:* a constraint-taking sibling of every consumer — 33 entries now, and each later
     consumer twice. D95's "consumers subcategorise" decides between a measure phrase and a noun phrase
     as a preposition's object; a constraint is a quantifier over the slot, which the grammar handles
     by rule for noun phrases.
4. **A bound marker carries two entries.** The constraint,
   `cat_unit_forall(λu. cat_mpc(u, value) / cat_mp(u, value))`, and the predicate,
   `cat_unit_forall(λu. (S[dcl,adj]\NP) / cat_mp(u, value))`, sem
   `λu.λb.λx. ∃q. C(q) ∧ has_quantity(x, u, q)` — the phrase-level counterpart of the predicative item
   a quantity token seeds. The copula takes it (`the temperature was less than 37 °C`), and
   `mod_lifts`, which also fires on composed cells (`combinators.rs:855-872`), makes it prenominal
   (`>90% infection efficiency`).
5. **The markers.** Words: `less than`, `more than`, `at least`, `at most`, `up to`, and the
   approximations `approximately`, `about`, `around`, `roughly`. Symbols: `<`, `≤`, `>`, `≥`, `~`, `≈`,
   which the preprocessor already keeps as tokens (its decision 4, `preprocess.rs:390-397`) and which
   seeding looks up by surface like any token (`seed.rs:587-594`). `fewer than` bounds a count and
   waits for slice 7. `over`, `under`, `above` and `below` are not added: they are spatial
   prepositions, with no bound use attested.
6. **A symbol between a noun phrase and a value is a comparison clause**: `P < 2 × 10⁻¹⁶`,
   `log2(copy number) < −1`. The symbol takes a third entry, `(S[dcl,fin]\NP) / cat_mp(u, value)`,
   with the predicate's sem.
7. **Scientific notation is one numeral.** `2 × 10⁻¹⁶`, `2 × 10³` and `1.5 x 10³` lex today as a
   numeral, an operator and a word (`10⁻¹⁶`). The preprocessor joins mantissa, `×` or `x`, `10` and a
   superscript or `^` exponent into one `Numeral`. Seeding densities (`2 × 10³ cells per well`) need it
   as much as P values do.
8. **A range is a digit pair with a unit or `%` after it.** `2–3 days` and `80–90%` (en-dash) and
   `45-60%` (hyphen) become one range token whose readings carry both endpoints in the unit. It seeds
   `cat_mpc(u, value)` with `λq. le(lo, q) ∧ le(q, hi)`, and the predicate item, as a quantity token
   seeds its value and predicate. A digit pair with no unit is not a range (`926-68021`, a catalogue
   number), and `4–7 foci` is a count range (slice 7). This is the lexical rule D95 said ranges waited
   on.

**Order.** 6a: decisions 1–5 — the category, the relations, the rule and the word markers — tested in
every consumer position: the VP adjuncts across their finiteness variants, the noun modifiers,
predicative and prenominal. 6b: the symbols, the comparison clause, scientific notation. 6c: ranges.

**6a — built** (2026-09-29).
- `units:lt`, `le` and `approx` (`units.esl`); `cat_mpc` (`lexicon-ontology.esl`), its
  denotation and unification (`dcg/category.rs`); the `unit_constraint` combinator
  (`CombKind::UnitConstrain`, `constrained_sem`, `prop_arity` in `dcg/rules/combinators.rs`); nine
  word markers with two entries each, 18 entries over ten sems (`closed-class.esl`).
- A bound reads back as `∀P:Prop. (∀q:Quantity(u). And(C(q), body) → P) → P`. `verbalize` renders
  the value as its constraint wherever the quantity renders: `hela at less than 6203/20 K`, `the
  Temperature is less than 6203/20 K`, `some Dose of at least 1/200000`.
- `quantities_in_the_parser.rs`: every marker after a VP adjunct, with the constraint's direction
  and `37 °C` read as 310.15 K; a noun modifier, the copula and a prenominal modifier; a bound on a
  bound (`less than about 2 h`); no bound on a difference (`rose less than 5 °C` has no parse); the
  verbalization; packed equals unpacked on five bound sentences. `every_measure_consumer_takes_a_constraint`
  applies each closed-class entry that takes a measure phrase in kelvin to a constraint in kelvin:
  all 51 (33 prepositions, 18 bound entries) yield the category they yield for the value, so the
  finiteness variants are covered by category, not by sentence.
- Decision 3 changed while building: a marker is a consumer too, so `less than about 2 h` composes
  (a value below one of about 2 h). The first cut refused a constraint to a marker; that refused
  sound English.
- The bootstrap manifest moved on `units`, `lexicon` and `closed-class`; `EXPECTED` is updated. No
  persisted store resumes until the reseed after slice 8.

**6b — built** (2026-09-29).
- The symbols `<`, `≤`, `>`, `≥`, `~` and `≈` are closed-class entries: the constraint and the
  predicate the words carry (`>90% infection efficiency`), and the comparison clause
  `(S[dcl,fin]\NP) / cat_mp(u, value)` (`the temperature < 37 °C`). `=` has only the clause, over
  `has_quantity` (`P = 0.02`): decision 6's construction, which the paper writes 59 times as `P =`
  and 37 as `n =`. 19 entries.
- Scientific notation (`preprocess.rs`, its decision 7): a mantissa, `×` or `x`, and a power of ten
  are one numeral — `2 × 10⁻¹⁶`, `4.2 × 10^-13`, `1.5 x 10³`, and `2.2× 10-16`, where extracted text
  lost the superscript and the plain minus is read as the exponent only after `×`. A power of ten alone
  needs a superscript or caret (`10³`, `10^6`); `10⁻¹³` alone had been pinned as a word.
- A symbol with no entry (`±`) still seeds nothing; `symbols_are_unseedable_not_missing` uses it now.

**6c — built** (2026-09-29).
- `TokenKind::Range(QuantityRange)` (`preprocess.rs`, its decision 9): a digit pair joined by an
  en-dash or a hyphen, with a unit or `%` after it, both endpoints read in that unit and their
  readings paired in order. `926-68021`, `96-well`, `5-fold` and `4–7 foci` are not ranges.
- Seeding gives a range two items (`seed.rs`, `range_items`): `cat_mpc(u, value)` with
  `λq. And(le(u, lo, q), le(u, q, hi))`, and the predicate over `has_quantity`, built by the
  combinator's `constrained_sem`. `verbalize` renders it `from 7200 s to 10800 s`.

**6d — built** (2026-09-29), with slice 7's decision that a plain value is exact.
- The CNL guide's postfix forms: `or more` and `or higher` (at least), `or less` and `or lower` (at
  most), each with the constraint and predicate entries, over the prefix markers' sems.
- `unit_application_backward` (`CombKind::UnitApplyBwd`): the measure phrase on the left, a
  `cat_unit_forall(λu. A\B)` on its right — `UnitApply` mirrored. `at 37 °C or higher` is
  `∃q. le(310.15 K, q) ∧ prep_at_value(…, q)`.
- The CNL guide (`docs/method/controlled-english-style-guide.md`): a plain value is exact, and the
  table of bound forms (rule 7); ranges written with the unit once (rule 8); scientific notation
  (rule 9); the range DON'T row replaced.

**Tests and corpus, slice 6.** `quantities_in_the_parser.rs`, 21 tests: 6a's, and symbols as bounds,
comparison clauses, scientific notation, ranges and postfix bounds; `every_measure_consumer_takes_a_constraint` finds
70 consumers. `quantity_tokens.rs`: scientific notation and ranges read against the units layer, and
what is not a range. `quantity_corpus.rs`: 27 covered rows, five of them slice 6's, from sentences of
the paper — `4–12% gels`, a gap until now; `80–90% confluence`; `approximately 100 mm³`;
`less than 2 × 10⁻¹⁶`; `>90%` — and `every 2–3 days` a gap for slice 8.

**Acceptance.** Tests in `quantities_in_the_parser.rs` for each position and marker, with a °C case
wherever the value reading matters (decision 5 above). The attested sentences join
`experiments/parsing/quantities/corpus.tsv`; a row whose host is slice 7's or 8's is a gap row naming
that slice, and turns covered when it lands.

**What changes.** `units.esl` (the relations), `lexicon-ontology.esl` (`cat_mpc`), `closed-class.esl`
(the markers), `dcg/category.rs` (`denote_cat`, `unify_into`, the packing keys), `dcg/rules/` (the
rule), `dcg/preprocess.rs` (scientific notation, ranges), `dcg/verbalize.rs` (a bound renders as its
words, `at less than 37 °C`). The three ontologies move the bootstrap manifest; one reseed after slice
8 measures slices 6–8 together.

## Slice 7 — counts

A cardinal drops its number today: `two genes`, `5 cells` and `1,000 cells` all read `∃x`
(`closed-class.esl:2170-2178`). No bound on a count can be stated over a number that is not there.

**Decisions** (2026-09-29)

1. **A count is a relation between a set and a number.** `ontology:has_count : forall (T : Set) =>
   (T -> Prop) -> units:Quantity(u"1") -> Prop`: the number of `T` satisfying `P` is `q`.
   `has_quantity(x, u, q)` gives an entity its measured value; `has_count` gives a set its size. A
   count is dimensionless, so it shares a measure phrase's carrier, `Quantity(u"1")`, and slice 6's
   order relations state its bounds.
   - *Rejected:* a count function and an equation, `Id(count(T, P), n)`. ESL cannot write `Id` yet
     (D98 slice 1), and a function says nothing the relation does not for stating a count.
   - *Rejected:* `lexicon:card : Set -> Entity -> float`, which counts per entity (how many `T` an
     entity has) and is a float.
2. **A plain cardinal states the exact count** (decided above): `two genes affect HeLa` is
   `has_count(Gene, λx. affects(hela, x), 2)` — the size of the scope set, `|T ∩ V|`. Subject and
   object determiners alike, the word forms with their number and a digit with its value; `0 genes`
   now reads, where the existential had to refuse it.
3. **A bounded count uses the same markers.** Each marker gains two count-determiner entries,
   subject and object, taking the bare number, `cat_num` (decision 6): `at least 1,000 cells` is
   `∃q. le(1000, q) ∧ has_count(Cell, V, q)`. `fewer than` and `or fewer` join, for counts only.
   - Number agreement is not checked on a bounded count: `more than one MMR gene` takes a singular
     noun.
   - *Rejected:* one rule turning any constraint over `u"1"` into a determiner. It would apply to
     `10%` and `1.5` as readily as to a count, and telling them apart would read the sem, which the
     packed chart's combination decision may not.
4. **Word numerals are numbers too.** `one` … `ten` gain an entry at `cat_num`, as a digit seeds
   one, so `more than one`, `at least two` and `two or more` work.
5. **A count range** is an en-dash digit pair with no unit (`4–7 foci`): a dimensionless range token
   seeding count-range determiners, `∃q. le(4, q) ∧ le(q, 7) ∧ has_count(…, q)`, when both ends are
   whole numbers. A hyphen pair with no unit stays a word (`926-68021`).
6. **A numeral is a bare number, `cat_num`, not a measure phrase.** D95 decided that "bare numerals
   and quantities share a carrier, not a category"; slice 4 had seeded a numeral as `cat_mp(u"1",
   value)` and `cat_mp(u"1", difference)` all the same. Slice 7 showed the cost: a bound marker's
   measure predicate took the bare number, `mod_lifts` made it prenominal, and `at least 1,000 cells`
   gained a second reading, cells with a quantity of at least 1000. `⟦cat_num⟧ = Quantity(u"1")`, the
   carrier; the cardinal determiners and the count bounds take it, and no measure consumer does, so
   `incubated at 37` without a unit has no parse. A percentage is still a measure phrase (`>90%
   infection efficiency`). A bare number as a dimensionless measured value (`P = 0.02`) is a
   statistic, which the CNL keeps out of a claim.

**7a, 7b — built** (2026-09-29).
- `ontology:has_count` (`ontology.esl`); `cat_num` (`lexicon-ontology.esl`, `dcg/category.rs`).
- `one` … `ten`: their determiner entries state their number (20 sems), and each is a number at
  `cat_num`. A digit seeds `cat_num` and, as a whole number, the determiner categories of `one` or
  `two` with its own count (`seed.rs`, `count_determiner`); `0` included.
- The scope is written `λx. V(x)`: the verb phrase's own sem is typed over `Entity`, and the kernel
  checks a λ at `T → Prop` but does not subtype `Entity → Prop` to it — the first cut passed `V` and
  every subject count failed the gate, while the object form, already a λ, passed.
- 19 markers carry count-determiner entries, 38 over 10 sems: the nine words, `fewer than`, the six
  symbols, `or more`, `or less`, `or fewer`.
- A word numeral's count is written as the canonical term, `(units:mk_quantity(2r, 0) :
  units:Quantity(u"1"))`, not `units:quantity(2, "1")`: a whole number needs no conversion, and the
  conversion form needs the units vocabulary at compile time, which `lexicon_validates.rs` and its
  siblings do not load (finding 12).
- An en-dash pair with no unit is a count range (`QuantityRange::unitless`), seeding count-range
  determiners only.
- `verbalize`: `has_count(T, λx. body, q)` reads `q T, body` — `2 Cell`, `at least 1000 Cell`,
  `from 4 to 7 Cell`.
- Tests: `quantities_in_the_parser.rs`, 25 — exact counts (word, digit, subject, object, `0`, `one`,
  `2 × 10³`), bounded counts in each form, count ranges, and a bare number that is not a measure;
  `closed_class_determiners.rs`'s cardinal test, where `0 genes` now reads; `quantity_tokens.rs`,
  `4–7 foci` a count range; the quantity corpus, three count rows from the paper, and the bare-number
  statistic row dropped.

**Order.** 7a: decisions 1, 2. 7b: decisions 3–6. 7c: proportions.

**7c — proportions, decided and built** (2026-09-29). The paper's shapes: `45–60% of such cancers do
not respond`, `15% of colon … cancers`, `in more than half of the samples`, `> half of loss events`,
`Half of the cell pellet was saved`, `none of the four other RecQ DNA helicases were preferentially
essential`, `each of the >17,000 genes`.
7. **A proportion is a relation of a group, a property and a value.** `ontology:has_proportion :
   Entity -> (Entity -> Prop) -> Quantity(u"1") -> Prop`: of the group `x`, the members satisfying
   `P` are the proportion `q`. The group is the entity the partitive's noun phrase denotes — a
   definite plural (`the(Sample)`) or a kind (`kind_of(ColonCancer)`) — since `of the samples` names
   a group, where a cardinal counts a noun's type.
   - *Rejected:* a ratio of counts, `has_count(T ∩ P) / has_count(T)`, which needs arithmetic D95
     has not declared, and states two counts the sentence does not.
8. **The value is a measure phrase at the dimensionless unit**: a percentage, or `half`, an entry at
   `cat_mp(u"1", value)` — a proportion is a measurement, as `50%` is, not a count.
9. **The partitive `of` takes the value on its left and the quantified noun phrase on its right**,
   which it scopes over the group: `of : (GQ / GQ) \ MP`, sem `λq.λQ.λV. Q(λx. has_proportion(x,
   λy. V(y), q))`, as a subject and an object quantifier. A bound or range on the value (`more than
   half`, `45–60%`) is a constraint on its left, which `unit_constraint_backward` — `UnitConstrain`
   mirrored — applies. As a preposition's object (`in more than half of the samples`) it is the
   subject-shaped quantifier `gq_prep_vpadjunct` takes.
10. **`none of`, `all of`, `each of`, `most of` and `some of` state the proportion themselves**: 0, 1,
   1, more than 1/2, more than 0.

Built: `has_proportion` (`ontology.esl`); `half`, the two partitive `of`s and ten quantifier
partitives (`closed-class.esl`); `unit_constraint_backward` (`combinators.rs`); `verbalize` reads
`more than 1/2 of the Cell, …`. Tests: `a_proportion_of_a_group` (eleven shapes, one reading each),
verbalization, packed equals unpacked; the quantity corpus adds `Half of the cell pellet was saved.`
Still out: `such` (`45–60% of such cancers`, no entry), the head noun elided across a coordination
(`15% of colon, 22% of gastric … cancers`, 7d), and `the four other RecQ DNA helicases` (a determiner
before a numeral, 7d). 7d: a determiner before a
numeral, right-node sharing, and number words as numerals with units (`Nine days after doxycycline
treatment`, which the preprocessor does not read as a quantity) — these change how `two` itself
parses, and are measured with the reseed.
- **The count**: a numeric cardinality of a predicate, beside `lexicon:card` (`closed-class.esl:1611`),
  which is per entity and a float.
- **Bounded cardinals**: `at least 1,000 cells`, `more than one MMR gene`, `≥ 8 foci`, `fewer than`;
  count ranges, `4–7 foci`.
- **Proportions**: `more than half of the samples`, `15% of colon cancers`, `20–30% of endometrial
  cancers`.
- **Decided (2026-09-29): a plain count is exact.** `5 MSI cell lines` states five. Every plain
  count in the WRN paper is exact — the study's inventory (`Project Achilles screened 517 cell
  lines`), its replicates (`three technical replicates each from two biological replicates`), a
  defined set (`these four lineages`, `the four other RecQ DNA helicases`), a definition (`the
  co-occurrence of two genetic events`) — and where the paper means a bound it writes one (`at least
  1,000 cells`, `>17,000 genes`). The exact count is of the set the sentence defines (`evaluated WRN
  knockout in 5 MSI cell lines`), not of everything the predicate holds of. A lower bound is written
  out, and the CNL guide says how (Measured quantities, rule 7): `at least N`, `N or more`, `N or
  higher`. Rejected: the at-least reading, which the paper never uses for a plain count; and the
  existential, which drops the number.
- **Also needed, from the same survey**: a determiner before a numeral (`these four lineages`, `the
  four other RecQ DNA helicases`), which no rule builds today; right-node sharing in `5 MSS and 5 MSI
  cell lines`; a breakdown that sums to its whole (`14 MSI cell lines … (6 leukemia, 2 prostate, …)`).

## Slice 8 — positions a measure phrase takes

- **Before a PP** (20 [8]): `9 days after shRNA induction`, `6 h before collection`. Fronted, as in
  `2 days after lentiviral transduction, cells were seeded`, it also needs the fronted PP adjunct,
  a gap without quantities (`After the dose, …`).
- **A verb's object**: `reached ~100 mm3`, `reached 80–90% confluence`, `used 0.2 µg/mL of
  doxycycline`, `genes that had less than one count per million`.
- **Pseudo-partitive** (5 [12]): `300 µl of CellTiter-Glo`, `0.2 µg/mL of doxycycline`.
- **`every N unit`** (5 [5]): D95's category and opaque `every_period` sem ("`every N unit` —
  category decided").
- **Rates**: `per` before a count noun distributes (`2 × 10³ cells per well`: for each well), and
  `count per million` is number notation like `ppm`, which D93 records as 10⁻⁶ at the dimensionless
  unit and nothing reads yet.

## Slice 9 — ratios

`a median 0.56-fold fewer deletion mutations in microsatellite regions compared to typical-lineage
MSI models (P = 1.7 × 10⁻⁹)`: `N-fold` as a factor on a comparative over counts, `compared to` as the
standard's marker, `a median` as the statistic the ratio is. The ratio has
`stats:EffectSize::Relative`'s shape (D95, "The tolerance derivation"); the parse states it, and the
statistics institution can later ground it.

## Out, as D95 decides

The tolerance construction and the vector-denoting PP — its one attested variant, `within 1.5× the IQR
from the box`, describes how a plot was drawn; statistic routing to D52 records, which the design for
the study's argument takes up; the precision reading of dispersion.
