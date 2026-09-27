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

**Still owed, with the user:**
- re-record the sense ranks and selections (finding 11; `experiments/parsing/README.md:12-176`).
  Segmentation changed, so recorded draws keyed by sentence text can miss;
- the parse-rate run on the CNL page and the re-established `baseline.json`. The CNL page
  (`first-page-cnl-v3.txt`) is not on this machine.

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
