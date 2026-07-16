# DCG module reorganization plan

Status: proposal. Scope: `kernel/src/dcg/{parser,packed,lookup,category,reserved}.rs`.
Method: each phase is independently shippable and gated by the existing differential
oracle (`packed_forest_equals_unpacked_on_core_grammar`, `packing_router_decision_is_correct`
in `kernel/tests/closed_class_determiners.rs`). No phase lands with a red oracle.

## Thesis

The two chart drivers (packed and unpacked) duplicate **trigger geometry** — where and
when each grammar rule fires — while correctly sharing the **rule semantics** (the
`category.rs` builders). That duplication is the module's central structural liability.
The fix is: (1) one rule registry both drivers interpret, and (2) marker-category nodes
that turn token-position scans into ordinary category tests. The file/struct layout
problems are secondary and become mechanical once the registry exists.

## Grounding: what is verified vs proposed

**Verified (read from source, 2026-07-13):**

- `parser::apply` splits a sem-blind decision (`combinable`, receives only
  `CategoryPayload`) from materialisation (`build`, only reader of child sems). This is
  the compile-time invariant that makes packing by `(cat_shape, prov)` sound.
  [`parser.rs:212,296,488`]
- The token-keyed constructs (coordination, apposition, `but not`, reciprocal, relative,
  appositive, pied-piping) are implemented **twice**: inline in `parse_at_cap`
  [`lookup.rs:2364–2595+`] and again as `BinRule` + `build_forest` triggers +
  `apply_bin_rule` [`lookup.rs:1654,1824`]. `apply_bin_rule`'s doc states each arm
  "mirrors the corresponding unpacked CKY rule exactly" — an obligation, not an enforced
  property.
- Two concrete divergences exist between the paths:
  - Unpacked coordination pushes **both** `coordinate_prop` and `coordinate_np` results
    when both return `Some` [`lookup.rs:2377–2394`]; packed uses `.or_else` and takes the
    first [`lookup.rs:1665–1667`]. Equivalent only if the two builders are disjoint on
    every category pair (true today, unchecked).
  - `apply_bin_rule`'s `ButNot` arm branches on `is_coordination(r.sem())`
    [`lookup.rs:1671`] — a sem read inside a decision that runs on node *representatives*
    in `binary_edges`, contradicting the "decision is category-based" contract.
- The widen-on-failure policy is duplicated: `widen_packed` [`lookup.rs:1429`] and
  `widen_unpacked` [`lookup.rs:2037`], and the two-pass reranker fallback is repeated
  verbatim in `parse_packed`/`parse_unpacked`.
- `parser::cky_parse` [`parser.rs:914`] is used only by tests
  (`lexicon_validates.rs`) and the `mod.rs` re-export; the production drivers are in
  `lookup.rs`. `LexicalIndex` (3,854-line `lookup.rs`) carries both the lexicon service
  and all parse policy (`packing`, `cell_beam`, `sense_cap`, `pos_prune`,
  `combinatory_core`, `sense_ranker`).
- Reference (`references/lightblue`): one driver (`ChartParser.hs`) consumes one rule
  registry (`CCG.hs`, `binaryRules` = a composition chain of rule functions). Reserved
  constructs are **ordinary chart nodes** with marker categories (`CONJ`, `LPAREN`/`RPAREN`);
  `checkCoordinationRule`/`checkParenthesisRule` are ternary rules keying on those node
  categories, written once. Parse settings (`ParseSetting`) are a separate record from the
  lexicon. Type checking lives outside `Parser/` (in `DTS/`), composed at the pipeline top.

**Proposed (to be validated by implementation + the oracle):**

- Marker-category nodes make every token-keyed trigger a category test that is sem-blind,
  decidable on representatives, and expressible as a packed `Edge` — including pied-piping,
  which currently has no packed edge and forces the unpacked path.
- The single-registry reshape preserves the forest on every oracle sentence (this is
  exactly what the oracle checks; the plan does not assert equivalence, it gates on it).

## Target architecture

```
dcg/
  lexicon.rs        entry_to_item + the lookup half of LexicalIndex (sources, probe,
                    overlay, scoping) — the form→entries service, no parse policy
  seed.rs           seed_leaves, lookup_span, morphology items, sense cap / rerank apply
  rules.rs          binary combinators (combinable/build) + the Construction and
                    UnaryShift REGISTRIES — the single source of truth for rule triggers
  chart/
    packed.rs       Forest repr + build_forest + kbest + cube (algorithms beside the data)
    unpacked.rs     item-level interpreter + per-cell beam
  felicity.rs       classify_felicitous, hole handling, OpenParse
  parse.rs          Parser facade: config record, router, ONE widen policy over an
                    `attempt(cap, beam) -> (closed, open)` closure, reranker two-pass
  category.rs       unchanged (the Cat calculus + construction builders)
  reserved.rs       unchanged (ReservedTable as data)
```

Rule registry (the core new type; extends the existing `SemRecipe` pattern):

```rust
struct Construction {
    trigger: Trigger,   // MarkerCategory(cat) after phase 3; ReservedAt/Seq/... before it
    sem_blind: bool,    // may the decision run on representatives?
    build: fn(&Item, &Item, &Arc<Layer>) -> Option<Item>,  // existing category.rs builder
}
```

The unpacked driver iterates the registry and cross-products cells through `build`.
The packed driver iterates the same registry, decides on representatives when
`sem_blind`, records an `Edge`, and calls the same `build` per pair at extraction. Span
arithmetic and the prop/np question are written once. The oracle then tests that the two
*interpreters* agree — which is the packing-soundness question stated directly.

## Phases

### Phase 0 — strengthen the safety net (no production change) — **DONE 2026-07-13**

Landed in `kernel/tests/closed_class_determiners.rs` (test-only; no production code touched):

- `assert_paths_agree(off, on, cases)` — the shared driver-parity harness (closed forest as a sorted
  sem multiset + open count). The existing oracle now calls it; Phases 2/3/5 gate on it. Each case
  carries an `exercises_rule` flag that **fails if the case parses to nothing on both paths**, so a
  refactor cannot degrade coverage into vacuous agreement.
- `packed_forest_equals_unpacked_on_coordination_and_butnot_stress` — 12 new cases over the two
  divergence sites (n-ary/comma lists, group-vs-GQ coordination, object-GQ generalization,
  coordination inside a relative, the three `but not` forms).
- `coordinate_prop_and_coordinate_np_are_disjoint` — the builder-level witness for open decision #1.
- `coordination_gaps_are_not_driver_divergences` — pins the sentences that parse on NEITHER path.

**Findings.**

1. **Open decision #1 is RESOLVED: the two coordination builders are disjoint.** Witnessed over a
   848-category pool drawn from the real bootstrap + demo lexicon and closed under application,
   type-raising, and both coordination builders (so `cat_np`, `cat_group`, `cat_coord` and raised GQs
   are all present — asserted, not assumed): across 719,104 ordered pairs × 3 connectives,
   `coordinate_prop` and `coordinate_np` never both fire. They are disjoint on the LEFT category —
   `coordinate_np` needs `cat_np`/`cat_group`, `coordinate_prop` needs `cat_coord` or a prop-ending
   left, and `⟦cat_np(T)⟧ = T` / `⟦cat_group(C)⟧ = List C` are never prop-ending.
   ⇒ The packed path's `.or_else` does **not** drop a reading. This is a **pure dedup in Phase 2**,
   not a bug fix. The unpacked path's both-fire shape is redundant, not more complete.

2. **The `ButNot` sem-read hazard is LATENT and unwitnessable at the sentence level.** `but not` over
   a coordinated operand (`HeLa affects BRCA1 but not BRCA1 and MSH2`, `… BRCA1 and MSH2 but not
   HeLa`) does not parse on either path — and those were the only sentences that could put a
   coordination sem and a non-coordination sem in one packed node, i.e. the only ones that could make
   the representative-based decision diverge from the per-pair one. No divergence is therefore
   *demonstrated*. ⇒ Phase 2 must fix it **structurally** (declare it `sem_blind: false` and make the
   type enforce that such a rule never decides on a representative). Do not wait for a failing parse
   to justify it; the oracle cannot produce one.

3. **No packed/unpacked divergence exists on any sentence tried.** The strengthened oracle is green.
   The refactor starts from a genuinely equivalent pair of drivers.

4. **Grammar gaps observed** (recorded, out of scope): a coordinated quantified subject (`a gene and a
   cell line affect HeLa`) does not parse — a raised GQ is `S/(S\NP)`, not a `cat_np`, so
   `coordinate_np` cannot fire and `coordinate_prop` refuses to generalize subject-GQs (agreement
   would stop biting). Likewise coordinated relatives and `but not` over a coordination. These are
   grammar coverage gaps, not driver divergences; pinned by
   `coordination_gaps_are_not_driver_divergences` so that closing one is a deliberate, visible event.

Original scope, for reference:

Before touching either driver, make the oracle catch the known divergences.

- Add oracle sentences that force `coordinate_prop` **and** `coordinate_np` to both
  return `Some` on one split (if constructible), and a `but not` case with mixed
  coordination/non-coordination sems in the right cell.
- Add a driver-parity assertion helper that, for a corpus, compares the packed and
  unpacked closed forests as sorted normalized-sem multisets + open counts (the oracle
  already does this; factor it so later phases reuse it).
- Fail-closed check: confirm the oracle currently passes on the expanded corpus. If a new
  case reveals an *existing* divergence, record it as a finding and fix it in Phase 2
  (do not weaken the test to green).

Risk: none (test-only). Exit: expanded oracle green (or a recorded pre-existing-divergence
finding).

### Phase 1 — unify widen/reranker policy (no grammar change)

- Introduce `attempt(cap, beam) -> (Vec<Item>, Vec<OpenParse>)` as a closure; `parse_at_cap`
  and `parse_packed_at_cap` become the two implementations passed in.
- One `widen` function drives escalation; the packed variant is the one that ignores `beam`.
- One `parse_with_reranker_fallback` wrapping the two-pass static-rank retry.

Risk: low (mechanical dedup; no rule or trigger touched). Exit: full test suite green,
byte-identical forests.

### Phase 2 — single rule registry, two interpreters

- Define `Construction`/`UnaryShift` tables in `rules.rs`, one entry per token-keyed and
  unary construct, each pointing at its existing `category.rs` builder.
- Rewrite the unpacked inline blocks [`lookup.rs:2364–2595+`] and the packed
  `build_forest`/`apply_bin_rule` sites to *interpret the table*. Trigger geometry
  (span math, comma absorption, the `["each","other"]` suffix, `c-2` appositive offset)
  is written once per trigger kind.
- Resolve the two divergences here: coordination emits per the shared rule (decide whether
  both builders fire — align to the unpacked both-fire behavior unless the oracle shows a
  spurious reading); `ButNot`'s sem read becomes an explicit `sem_blind: false` rule that
  either materialises eagerly per pair or routes unpacked.

Risk: medium (touches both drivers). Guard: the Phase-0 oracle. Exit: oracle + suite green;
`apply_bin_rule`'s "mirrors exactly" comment deleted because there is one definition.

### Phase 3 — marker-category nodes — **CLOSED 2026-07-13: NOT ADOPTED**

Implemented for coordination (Phase 3a), measured, and **reverted**. Recorded here as a closed
question so it is not re-proposed without the numbers.

**What was built.** Coordination via marker nodes, in the **two-stage binary** shape (the CCG
`(X\X)/X` factoring): `and`/`or`/list-comma seeded as chart leaves with a new `cat_conn : Conn -> Cat`
category and an inert sem; `[and]·[Y]` binding the marker to its right conjunct as a new
`cat_coordinand : Cat -> Conn -> Cat`; `[X]·[and Y]` then folding it into the list. `⟦cat_conn⟧`
undefined (fail-closed), `⟦cat_coordinand(B,c)⟧ = ⟦B⟧`. It WORKED — all 140 tests green, both
differential-oracle tests green, coordination firing only through the markers.

**Why it was reverted: the realized benefit was ~zero and the cost was real.**

- **+209 / −26 lines of code**, and a **~10–15% parse slowdown** (suite ~55–63s → ~69–75s, repeated
  runs). The slowdown is NOT site count — pinning `CoordBind` to the marker's one-token cell and
  indexing `Coordinate` on the connective recovered only ~3s of ~15. It is **inherent to binary
  chaining**: `[and]·[Y]` materialises a `cat_coordinand` intermediate in cell `[c, j]` for every
  connective `c` and every `j`, adding chart items in O(n) cells per connective that the old
  single-step rule never built. Every parse pays it, coordinated or not; the WordNet-scale path is
  already OOM-sensitive.
- **The one benefit that would have justified it was already delivered by Phase 2.** "The coordination
  decision is category-based, so it is representative-safe under packing" was made true in Phase 2 by
  putting `sem_is_coordination` into the packing `Sig`. Phase 3 re-bought it.
- **"No token-position arithmetic" was only partly delivered.** The token check had to come back as a
  performance INDEX (a `cat_coordinand` can only occupy a cell beginning at its marker, so splits whose
  right operand starts elsewhere are skipped — provably result-preserving, but still a token index).
- The remaining payoff — retiring `parse_needs_unpacked` — is **one router guard for a construct the
  code itself calls "rare, non-piling"**, plus removing pied-piping's `entries_for` smuggle. Not worth
  a permanent slowdown on every parse plus a chain-data change to `lexicon:Cat`.

**Both candidate shapes fail, for complementary reasons — this is what closes the question.**

| | reaches pied-piping? | chart cost |
|---|---|---|
| **Two-stage binary** (built, reverted) | yes — chaining scales to any arity | **materialises intermediates; ~10–15% on every parse** |
| **Ternary + `Edge::Ternary`** (lightblue's `coordinationRule l c r`) | **no — it is ternary; pied-piping is QUATERNARY** | ~free (marker is one leaf, no intermediate) |

The expense *is* the generality: reaching four operands requires chaining, and chaining requires
intermediates. There is no shape that is both cheap and sufficient. The cheap shape (ternary) buys only
cosmetics on top of Phase 2; the sufficient shape (binary chaining) costs more than the carve-out it
removes.

**Findings worth keeping (they outlive the reverted code).**

1. **Pied-piping is QUATERNARY.** It consumes four chart constituents — noun `[i, p-1]`, subject
   `[p+2, k]`, VP `[k+1, j]` — plus a preposition it pulls **directly from `entries_for(tokens[p])`,
   bypassing the chart entirely** (`lookup.rs`, the pied-piping block). This is why the packed forest
   has no edge for it, and it kills any ternary-rule approach outright. Anyone revisiting
   `parse_needs_unpacked` must start here. The `entries_for` smuggle is itself a smell: a rule reaching
   past the chart into the lexicon.
2. **`conn_list` is a PHANTOM constructor** (pre-existing, still live). `coordinate_prop` /
   `coordinate_np` build `Exp::InductiveCtor(Conn, "conn_list", [])`, but `data lexicon:Conn`
   (`ontologies/lexicon/lexicon-ontology.esl`) declares only `conn_and`, `conn_or`, `conn_but_not`. It
   survives only because `⟦·⟧` erases the connective, so the term is never checked against its decl.
   A real latent bug, worth its own small fix — independent of any of this.
3. **Marker intermediates cost ~10–15%.** Measured, not estimated. Do not re-propose marker nodes
   without a plan for the intermediate-materialisation cost.


### Phase 4 — split LexicalIndex; lift parse policy into a config

- Extract the form→entries service (sources, `probe_form`, overlay, `scoped`,
  `entries_for`, `span_limit`) into a lookup type in `lexicon.rs`.
- Move parse policy (`packing`, `cell_beam`, `sense_cap`, `pos_prune`, `combinatory_core`,
  `sense_ranker`) into a `ParseConfig` (lightblue `ParseSetting` analogue) owned by the
  `Parser` facade. Builder methods (`with_*`) move to the facade.
- Public API (`mod.rs` re-exports) preserved via type aliases / re-exports during the move.

Risk: low–medium (call-site churn; the two tests using `with_packing` and `routes_packed`
update). Exit: suite green; `LexicalIndex` no longer names parse policy.

### Phase 4 — split LexicalIndex — **DONE 2026-07-13** (after first being skipped)

Skipped once, then done — and the reason for the reversal is the useful part.

**The skip was right on the framing I had.** I costed a split into two *public* types for ownership
clarity: 78 call sites, no line reduction, no perf win. Churn.

**What changed it** was noticing that `LexicalIndex`'s own name and doc comment
(*"a `form → entries` lookup … built once per layer"*) describe something it had long stopped being. It
had accreted: sense cap, cell beam, LLM reranker, pos-prune, packing flag, both CKY drivers, the widen
ladder, the felicity gate, and D64 resolution. The name was a **fossil of the original design**, exactly
like `parser::cky_parse` (which had been stranded for the same reason — see below). That reframes the
work from "tidy up ownership" to "the type is lying about what it is", which is worth 78 mechanical
call-site edits.

**The backbone.**

```
trait LexicalLookup    entries_for(form) -> Vec<LexEntry>;  span_limit(n)      ← the fence
struct LexicalIndex    layer, source, overlay                                  ← impl LexicalLookup. Nothing else.
struct ParseConfig     sense_cap, cell_beam, packing, pos_prune, combinatory_core, sense_ranker
struct Parser          lex: Arc<dyn LexicalLookup>, layer, reserved, config
                       ← seeding, ranking, both drivers, widen, felicity, resolution
```

`Parser::build(layer)` is the one-call path; `Parser::over(lex, layer)` shares one lexicon across
several parsers (so the A/B tests stop building the index twice).

**The trait is the point, and it is load-bearing.** `Parser` holds `Arc<dyn LexicalLookup>`, not a
concrete `LexicalIndex` — so parsing, ranking, and policy **cannot** re-accrete onto the lexicon,
because the compiler will not let the parser call anything else on it. Verified after the split: the
only lexicon calls anywhere in the parser are `self.lex.entries_for` and `self.lex.span_limit`, and no
stage module so much as names `LexicalIndex`. A concrete type invites accretion; a two-method trait
refuses it. That is the whole reason the drift happened, and the only durable defence against it
happening again.

The split also *found* things, which is how you know the seam is real: two tests that assert on index
coverage (`index_covers_the_committed_entries`, `lazy_index_is_lazy_and_matches_eager`) turned out to be
**lexicon** tests, not parser tests, and now hold a `LexicalIndex` directly; and
`with_document_augmentation` is a fact about *words*, so it stayed on the lexicon and its call sites now
augment the index before a parser wraps it.

Verified: full workspace builds; 140/140 parser, 51/51 lexicon, 1613/1613 kernel unit; `fmt` and
`clippy -D warnings` clean. No behavior change — the differential oracle is green.

### Phase 5 — file layout — **DONE 2026-07-13**

`lookup.rs` (3,647 lines, eight concerns) decomposed into a module directory. The new modules are
**children** of `lookup`, not siblings: Rust makes a private item visible to its module's descendants,
so each stage can still reach `LexicalIndex`'s private fields — the split needs **no `pub(crate)`
loosening and no public API change**. `dcg::lookup::X` still resolves for every exported type.

```
dcg/lookup/
  mod.rs            1455  LexicalIndex + Source + entries_for/probe/scoped + tokenize
                          + the router + the widen/reranker policy + sense-rank orchestration
  seed.rs            775  seed_leaves / lookup_span / morphology / leaf unary shifts  (BOTH paths)
  chart_packed.rs    468  build_forest / kbest / cube / binary_edges / materialize_unary
  chart_unpacked.rs  419  parse_at_cap (flat beamed CKY) + pied-piping + beam helpers
  resolve.rs         257  D64 open-parse resolution (holes → antecedents)
  rules.rs           256  binary_sites (the registry) + apply_bin_rule                (BOTH paths)
  felicity.rs        233  classify_felicitous / OpenParse / holes — the kernel as oracle
dcg/packed.rs        363  (unchanged) the forest DATA: Forest / PNode / Edge / Sig
```

Pure code motion: no behavior change, no API change, no perf change (suite 68s, in band). Guarded by
the differential oracle — 140/140 integration + 44/44 dcg unit tests, `fmt` and
`clippy -D warnings` clean.

**History.** Done with `git mv lookup.rs lookup/mod.rs` so the move is a real rename. Note the default
`git log --follow` (50% similarity) will NOT trace it, because `mod.rs` retains only 40% of the
original; rename detection finds it at `-M25%`. To get clean history, commit in **two** steps — a pure
rename first (100% similarity, always detected), then the extraction as an ordinary edit.

**Not done:** `parser::cky_parse` (test-only, used by `lexicon_validates.rs` and the `mod.rs`
re-export) is still a second production-shaped driver that no production path uses. Left as-is —
retiring it means porting that test file, which is a separate change.

## Ordering rationale

- Registry (Phase 2) **before** marker nodes (Phase 3): with the registry in place, marker
  nodes are a change to the trigger-matching layer only, not a scatter across two drivers.
- Marker nodes **before** the file split (Phase 5): the split is mechanical once the rule
  set is driver-independent; doing it first would relocate the duplication rather than
  remove it.
- Widen dedup (Phase 1) first: independent, safe, shrinks `lookup.rs` and de-risks reading
  the driver code for Phases 2–3.
- Phase 0 first, always: the oracle is the only witness that a driver change preserved the
  forest. Strengthen it before relying on it.

## Do not take from the reference

- lightblue's `Node` exposes sems and full daughter trees to every rule (fine at hand-lexicon
  scale, incompatible with packing). Keep the `CategoryPayload`/`SemanticPayload` split — it
  is strictly stronger than the reference here.
- lightblue's closed class is 947 lines of Haskell (`MyLexicon.hs`). Keep the closed class as
  ontology data (`ontologies/lexicon/closed-class.esl`).
- lightblue hardcodes `CONJ`/`LPAREN` constants. Keep the reserved *forms* as `ReservedTable` data.
- lightblue's marker-category *nodes* (`CONJ` as a chart constituent) — **tried and rejected**, see
  the closed Phase 3 above. What works at lightblue's scale (hand lexicon, no packing) costs ~10–15%
  here, and the win it would have bought was already taken by Phase 2.

## Open decisions

All three are now closed by measurement rather than deliberation:

1. ~~Coordination both-fire vs first-only.~~ **RESOLVED in Phase 0** — the builders are disjoint
   (witnessed, 719k pairs). `.or_else` is safe; Phase 2 unified them as a pure dedup.
2. ~~`sem_blind: false` rules under packing (eager vs route-unpacked).~~ **DISSOLVED in Phase 2** —
   neither was needed. Both sem-reading rules (`Coordinate`, `ButNot`) consult the SAME predicate, so
   carrying `sem_is_coordination` in the packing `Sig` makes the representative decision exact. No
   over-approximation, no wasted edges, no second code path.
3. ~~Marker-category `⟦·⟧`.~~ **MOOT** — Phase 3 is closed; there are no markers. (For the record, the
   answer that worked was: `⟦cat_conn⟧` **undefined** (`Err`), which is what makes `prop_ending` false
   and `classify_felicitous` drop the item — a marker fails closed rather than denoting something
   inert.)

## Status (2026-07-13)

Committed: Phase 0 (`52070aa`), Phase 1 (`eee9d9d`), Phase 2 (`dfe7a30`). Phase 3 built, measured,
reverted — closed as not adopted. Phases 4–5 (split `LexicalIndex` / lift parse policy into a config;
file layout + retire the test-only `cky_parse`) remain, and are consolidation rather than expansion —
the shape of work that has actually paid here.
