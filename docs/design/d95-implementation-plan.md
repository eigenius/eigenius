# D95 implementation plan — quantities in the tokenizer and parser

Ordered slices for D95, against `quantities-in-the-parser` at `b10ad53`. D93 (units, conversion)
and D94 (exact rationals) are built; D96 (JATS) is decided and not built, so every slice below
works on plain text.

## What exists

- **Tokenizer.** `dcg::segment::tokenize(&str) -> Vec<String>` (`segment.rs:147`) fuses lexing
  with four decisions: bracketed-aside removal (`strip_bracketed_asides`, `:192`), separator-to-space
  substitution, edge trimming, and comma handling. It has 9 call sites: `parse/paths.rs:79,256`,
  `parse/mod.rs:633,690,882,955,981`, `augment.rs:227`, `verbalize.rs:66`.
- **Seeding.** `Parser::seed_leaves(&[String], …)` (`parse/seed.rs:568`) builds an `n × n` chart
  from the token count and looks each span up by its joined surface. A token with no entry seeds
  nothing.
- **Non-prose.** `is_nonprose` (`segment.rs:125`) is read only by `all_prose_tokens_known`
  (`parse/mod.rs:955`), the widen gate's coverage probe.
- **Cardinals.** `two`..`ten` are plural-determiner entries (`closed-class.esl:2169`), count
  dropped; digit numerals have none.
- **Prepositions by role** (`closed-class.esl`):

  | role | category | prepositions |
  |---|---|---|
  | VP adjunct `_prep` | `((S\NP)\(S\NP))/NP` | among between beyond for from in on to with within without |
  | noun modifier `_nmod` | `cat_pp/NP` | about against among at between beyond by for from in into of on to upon with within without |
  | argument marker `_arg` | `cat_pp_arg(p)/NP` | about against as at for from in into of on to upon with |

- **Units.** `units::convert::Vocabulary::convert(value, stated)` takes D93's strict stated form and
  returns `Converted { magnitude, unit, kinds }`; `quantity_term()` builds
  `(units:mk_quantity(c, pi) : units:Quantity(u))`. The units layer has unit arithmetic
  (`units:mul`, `units:pow`) and no quantity arithmetic. The °C offset is applied for a bare `°C`
  only.

## Corrections to D95 found while planning

1. **Only two PP categories fix their object's type, not three.** `⟦cat_pp_arg(p)⟧` and
   `⟦cat_pp_than⟧` are `Entity` because each marker is transparent: the category denotes the object.
   `⟦cat_pp⟧ = Entity -> Prop` denotes the predicate over the modified noun, and the object type
   appears only in the preposition's own category: `of : cat_pp / cat_mp(u)` denotes
   `Quantity(u) -> Entity -> Prop` with no change to `cat_pp`. The VP adjunct is not a PP category at
   all. So `at 37 °C` as an adjunct, and `a dose of 5 mg` as a modifier, need new entries and no
   category widening.
2. **`after` needs a VP-adjunct entry, not a `Prep` variant.** `Prep` indexes `cat_pp_arg` only, and
   `after 2 h` is an adjunct. **`at` has no VP-adjunct entry either**, so `incubated at 37 °C` has no
   `at` to use today whatever its object.
3. **Conversion happens at seeding, per candidate, not after selection.** D95's "normalisation after
   selection" assumed a stated-unit record carried through the chart. D93 dropped the record, and an
   item's sem must already have type `Quantity(u)` for the felicity gate, so each candidate reading is
   converted when it is seeded. Selection among candidates still belongs to the chart and ranker.
   **°C is the exception**, and decision 5 is about it.

## Decisions that block coding

1. **The `MP` category.** Recommended: `cat_mp : core:unit -> Cat` with
   `⟦cat_mp(u)⟧ = units:Quantity(u)`, and a binder `cat_unit_forall : (core:unit -> Cat) -> Cat` with
   `⟦cat_unit_forall(λu. R)⟧ = Πu:core:unit. ⟦R⟧` for consumers that take any unit — the
   `cat_forall` pattern, where the binder is instantiated from the consumed item rather than erased.
   `slot_is_concrete_nonentity` (`category.rs:468`) gains `cat_mp` with a literal unit, so a consumer
   that selects a dimension forces the unpacked path. The alternative, an unindexed `cat_mp` denoting
   `Σu. Quantity(u)`, needs no binder, and it puts dimension checking outside the type system, which
   D93 exists to prevent.
2. **Prose unit surfaces.** Recommended: a `lexicon:UnitSurface` class — a prose symbol and the
   `units:NamedUnit` it names — in a lexicon-side file, with prefixability inherited from the unit.
   The strict `units:symbol`s are surfaces implicitly; entries add senses: `g` → standard gravity,
   `l` → litre, `RCF` → standard gravity, `h`/`hr`/`hrs`/`hour`/`hours` → hour. A factor resolves
   exact-then-longest-prefix to every sense, so `mg` stays one reading (standard gravity takes no
   prefix) and `g` gets two. The units layer keeps one symbol per unit.
3. **A numeral-initial token no rule interprets.** Recommended: it becomes a word token and is
   counted as a missing lexeme when the lexicon has no entry for it, which is what `53BP1` becomes
   under D95's revised rule. The failure stays visible in the gate, where today it is invisible. The
   alternative D95 records — setting the token aside and parsing the rest — yields a parse that omits
   a constituent.
4. **Figure references in plain text.** Recommended: no numeral/unit split on a token directly after
   `Fig.`, `Figs`, `Figure`, `Table` or `Extended Data Fig.`; a JATS `<xref>` span supersedes the rule
   once D96 is built. The alternative is to leave `Fig. 2d` reading as two days until then.
5. **°C in a neutral item.** D95 decides that the consumer supplies point-or-difference, and that a
   difference in °C is a magnitude in K. An item converted at seeding has already chosen: `37 °C` is
   310.15 K, and `sampled at 5 °C intervals` wants 5 K. Recommended: the item carries the linear
   magnitude (`37 K`) and `cat_mp` carries the unit's origin (273.15 K for °C, zero for every other
   unit); a point consumer adds the origin. This needs `units:add` over quantities of one unit. The
   alternative, seeding °C quantities as two competing items (point and difference), makes the
   ranker choose what the consumer's category already determines.

## Slice 1 — lexer and preprocessor, behaviour-preserving

- `dcg::lex::lex(&str) -> Vec<Lexeme>`: every character in some lexeme, each with its byte span and a
  class (word, numeral, symbol, punctuation, space). Segmentation is today's: `-` joins, `–` `—` `/`
  and brackets separate.
- `dcg::preprocess::preprocess(&[Lexeme]) -> Vec<Token>`, `Token { surface, span, kind }`, with
  `TokenKind::{Word, Comma, NonProse}` in this slice. It makes today's decisions — asides, separators,
  edge trimming, comma handling, dangling-comma removal, run collapsing — and classifies non-prose with
  today's rule.
- `tokenize` is removed; its 9 call sites and `seed_leaves` take `&[Token]`. `all_prose_tokens_known`
  reads `TokenKind::NonProse`.
- **Test:** the old `tokenize`, kept in the test module as an oracle, equals the preprocessed
  surfaces over every sentence of the CNL page, the tracked parse fixtures, and — when present — the
  gitignored WRN letter body and methods. No chain change; no reseed.

## Slice 2 — preprocessor rules that change behaviour

- Revised non-prose rule: a numeral is `TokenKind::Numeral(Rational)`, with its sign kept (`−1`,
  `-1`); a digit-initial token that is not a numeral is a word (`53BP1`); decision 3 for the rest.
- Revised aside rule: `(` directly after a word character is an argument (`log2(copy number)`) and
  stays; after a space it is a gloss and is dropped, as today.
- Comparison operators (`<`, `≤`, `>`, `≥`, `=`) survive as symbol tokens classed non-prose, so
  `< −1` reaches no parse rather than the number one. Comparison constructions stay out of v1.
- The figure-reference guard (decision 4).
- **Measure** with the current snapshot (no chain change): the parse-rate run over the CNL page and
  the methods, with every changed sentence listed.

## Slice 3 — unit surfaces, the prose unit reader, the quantity recogniser

- The `lexicon:UnitSurface` vocabulary (decision 2).
- `dcg::quantity`: reads a prose unit — normalises `µ` (U+00B5) to `μ`, superscript exponents
  (`ml⁻¹`), `per`, spaces and `·` — resolves each factor to all its senses, forms the product of
  candidates, and converts each through `Vocabulary::convert`. `%` is a dimensionless factor 1/100
  here, not a unit (D93).
- The preprocessor merges numeral + unit lexemes (`37` `°C`, `μg` `ml⁻¹`) and splits an attached unit
  (`5mg`, `931g`) into `TokenKind::Quantity { value, readings }`. `5-fold` and `53BP1` are not
  quantities.
- **Tests** on D95's inventory: `931g` has two readings (grams, and 182599823/20000 m·s⁻²); `37 °C`,
  `2 h`, `5 mg/kg`, `10 μg ml⁻¹`, `10%`; `53BP1`, `HEK293T`, `5-fold` and `Fig. 2d` are not quantities.
- Chain change: the surface vocabulary moves the bootstrap manifest. The reseed waits for slice 5.

## Slice 4 — `MP` in the grammar, and seeding

- `cat_mp`, `cat_unit_forall` and their `denote_cat` arms (decision 1); the origin (decision 5), and
  `units:add` if decision 5 goes that way.
- `seed_leaves`: a quantity token seeds one `cat_mp(u)` item per reading, sem the converted term; a
  positive-integer numeral seeds the cardinal determiner items the word forms have.
- **Tests:** `931g` seeds two items with units `kg` and `s⁻²·m`; the N-N kind compound rule does not
  read `5 °C` as a compound (D95, "Two facts recorded").

## Slice 5 — consumers, the corpus, one reseed

- `cat_mp` siblings of existing entries: VP adjuncts for `at`, `for`, `in`, `with` and `after` (`at`
  and `after` are new adjunct prepositions), noun modifiers for `of`, `with` and `at`. Each is an
  opaque conjunct, the house pattern for PP adjuncts, over a relation taking a quantity.
- The prenominal measure phrase (`10 μM etoposide`, `0.1% crystal violet`, `5 °C intervals`), 40
  occurrences, D95's largest bucket.
- A tracked corpus of quantity sentences in the CNL register, derived from the WRN methods and
  annotated with the expected consumer, since the methods text itself is gitignored and not
  CC-licensed. The report separates quantity gaps from syntax gaps (D95, "Consequences for the
  measurement").
- **One reseed** for slices 3-5, then the re-established baseline, and the CNL guide revision.

## Slice 6 — arguments and standards, on evidence

Type-indexing `cat_pp_arg` and `cat_pp_than` (correction 1). The WordNet and UMLS importers emit
`cat_pp_arg`, so the change moves the whole lexicon. It is built when a corpus sentence needs a
measure phrase as a governed argument or a comparison standard; none of D95's examples does.

## Slice 7 — degree semantics

The interval-arithmetic category, distinct from `cat_measure`; differential comparatives
(`5 °C warmer`) with exact-degree templates; the adjective-adjunct `by`; scalar-change verbs
(`rose 5 °C`) through `m^Δ`; the verb's scale orientation. These consume decision 5's origin.

## Out, as D95 decides

Ranges and intervals; `every N unit`; the tolerance construction and the vector-denoting PP;
statistic routing to D52; the precision reading of dispersion.
