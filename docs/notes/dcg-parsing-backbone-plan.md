# DCG parsing backbone — decoupling plan

Status: proposal. Follows `dcg-module-reorganization-plan.md` (Phases 0–5, done/closed).
Scope: `kernel/src/dcg/{parser,category,packed}.rs`, `kernel/src/dcg/lookup/*`.

## Thesis

The rule set and the chart drivers are split across files by **what each happens to depend on**, not by
**what each is**. Three lexicon reach-backs are the whole cause: two fetch grammar constants, one is a
bug. Remove them and the rules and the chart become lexicon-free, at which point they can be organized
by concern — and the reorganization becomes *legal* rather than something forced with visibility tricks.

Fix the coupling first, then move the files. Moving first would relocate the coupling, not remove it.

## Grounding

**Verified** (read from source, 2026-07-13/14; the parser suite, lexicon suite, and 1613 kernel unit
tests are green at this commit):

- **Both chart drivers consume an identical rule surface** — `apply`, `apply_bin_rule`, `binary_sites`,
  `seed_leaves`, `bare_nominal_shifts`, `raise_nps`, `front_participial`, `complete_coord`,
  `classify_felicitous`. They differ in exactly two ways: the **representation** (packed
  nodes-with-hyperedges vs flat items-in-cells), and two extras carried only by the unpacked path
  (`apply_core`, the combinatory spike; `pied_pipe`, the quaternary carve-out). They are one concern
  with two implementations, kept separate on purpose so the differential oracle has something to
  compare.
- **The rules live in three files, split by dependency, not by concern:**
  | file | lines | holds | why it is where it is |
  |---|---|---|---|
  | `parser.rs` | 993 | categorial combinators (`apply` / `apply_core` / `apply_group`) | needs only `Layer` ⇒ free functions |
  | `category.rs` | 1978 | Cat **algebra** (~210 lines) **and** construction builders (~472) | both need only `Layer` |
  | `lookup/rules.rs` | 256 | the token-keyed registry (`binary_sites`, `apply_bin_rule`) | needs the **lexicon** ⇒ `impl Parser` |
- **`parser.rs` contains no parser.** Since `cky_parse` was retired it holds only the categorial
  combinators. It is a *rules* file wearing a driver's name — the third naming fossil in this module,
  after `LexicalIndex` (a lookup that had become a parser) and `cky_parse` (an entry point that had
  become a test harness).
- **`category.rs` conflates two layers**: the Cat algebra (`denote_cat`, `unify_cat`, `cat_subsumes`,
  `feat_meets` — knows nothing of parse items) and the grammar's construction builders (`relativize`,
  `coordinate_*`, `reciprocate`, `appose_group`, `type_raise`, `pied_pipe`, `front_participial`).
- **The packed forest's data and its algorithms are in different directories**: `dcg/packed.rs` holds
  `Forest`/`PNode`/`Edge`/`Sig`; `dcg/lookup/chart_packed.rs` holds `build_forest`/`kbest`/`cube` — the
  only code that touches them.
- **Exactly three lexicon reach-backs** bind the rules and chart to `LexicalLookup`:
  1. `lookup/seed.rs:213` — `kind_raised_nps` fetches `entries_for("a")` / `("these")` for the
     determiner's raised category.
  2. `lookup/rules.rs:62` — `appositive_obj` fetches `entries_for("a")` for the `a_obj` raised category.
  3. `lookup/chart_unpacked.rs:129` — pied-piping fetches `entries_for(tokens[p])` for the preposition.

  (Everything else touching the lexicon — seeding, morphology, `has_token` — is legitimately lexical.)
- **The `Parser` fence holds.** After the Phase-4 split the parser reaches the lexicon *only* through
  `self.lex.entries_for` and `self.lex.span_limit`; no stage module names `LexicalIndex`.

**Proposed** (to be validated by the differential oracle + new tests): that removing the three
reach-backs is behaviour-preserving for (1) and (2), and behaviour-*correcting* for (3).

## The three reach-backs are not the same kind of problem

### (1) + (2): grammar constants, mis-sited — a layering fix

`a`, `these`, and `a_obj` are **grammar constants that happen to be stored in the lexicon**. Resolving
them once at construction converts the rules' dependency from `LexicalLookup` (a *service*) into a
`Grammar` (a *value*). No behaviour change — the same categories, fetched once.

It is also not free today. `kind_raised_nps` runs for **every `cat_n` item in every cell** (via
`bare_nominal_shifts`), and each call does `entries_for("a")` **and** `entries_for("these")`. On the
lazy path `entries_for` takes a **mutex lock and clones the entry vector** — so the current code takes
two locks per noun item per cell, to fetch two constants. Pre-resolving removes that from the hot path.

### (3): the pied-piping smuggle — a correctness fix

The rule reaches past the chart into the lexicon for a preposition that is *already a seeded token in
cell `[p][p]`*. Three concrete defects follow, all in `chart_unpacked.rs:128–165`:

- **It bypasses the lexicon scope filter.** Every seeded entry passes through `scoped(entries, scope)`
  (D65 §4: a tagged entry whose lexicon is outside the scope is dropped). This calls `entries_for`
  **raw**. Under `parse_scoped(…, Some(&[lex_a]))` a pied-piping preposition can therefore come from a
  lexicon that is not in scope. The scope contract is violated, silently.
- **It drops the preposition's `Cost`.** The result is built with
  `noun.cost() + subj.cost() + vp.cost()` — the prep's own cost is never summed in, unlike every other
  rule. So a pied-piping parse systematically under-counts, and the preposition's `lexicon_order` (the
  **primary** rank key) is zeroed for that word.
- **It bypasses the sense cap, the contextual reranker, the cross-POS prune, and the cell beam** — all
  of which act at seed time, which a smuggled entry never reaches.

Reading the preposition from the chart fixes all three at once, because a chart item has already been
through scoping, capping, ranking, pruning, and beaming.

**It does NOT make pied-piping packable.** It stays quaternary (noun + subject + VP + preposition); the
packed forest has no n-ary edge, and decomposing it into binaries is the marker chaining measured as
too expensive in the closed Phase 3. `parse_needs_unpacked` survives. What dies is the bug and the
coupling — not the carve-out. (Stated explicitly because the temptation to claim otherwise has already
cost one reverted phase.)

## Target layering

```
item.rs        Item / Cost / Combinator / the CategoryPayload–SemanticPayload split
category.rs    the Cat ALGEBRA only: ⟦·⟧, unify, subsume, feature-meet   (knows nothing of items)
rules/         THE GRAMMAR, in one place
  combinators.rs   apply / apply_core / apply_group                      ← from parser.rs
  constructions.rs relativize, coordinate_*, reciprocate, appose_group,
                   type_raise, complete_coord, front_participial, pied_pipe  ← from category.rs
  registry.rs      binary_sites + apply_bin_rule + the unary shifts       ← from lookup/{rules,seed}.rs
chart/         THE DRIVERS — one concern, two implementations
  packed.rs        Forest + build_forest + kbest + cube                   ← merges dcg/packed.rs
                                                                            + lookup/chart_packed.rs
  unpacked.rs      the flat beamed CKY                                    ← from lookup/chart_unpacked.rs
lexicon.rs     LexicalIndex + LexicalLookup                               (no parsing — already true)
seed.rs        lexicon × morphology → leaf cells   (the ONE place the lexicon meets the chart)
felicity.rs    the kernel as oracle
parse.rs       Parser: ParseConfig, the router, the widen ladder, entry points
resolve.rs     D64 open-parse resolution
```

`rules/` and `chart/` depend on `category` + `item` + a small `Grammar` (layer, reserved table,
resolved templates) — **not** on the lexicon. That is what lets them leave `lookup/` and sit at the top
level, where a parser belongs.

## Status (2026-07-14)

**Phase A — DONE.** `Grammar { layer, reserved, dets }` (`kernel/src/dcg/grammar.rs`), resolved once at
`Parser::over`. `kind_raised_nps` and `appositive_obj` take the templates as values. `lookup/rules.rs`
no longer touches the lexicon.

**Phase B — DONE, and both bugs were real.** Tests written first, and both *failed* against the old rule:
- `pied_piping_respects_the_lexicon_scope` — an out-of-scope preposition was admitted, returning a
  `prep_beside` reading from a lexicon the caller had excluded.
- `pied_piping_counts_the_prepositions_cost` — parse cost was 0 where the preposition's `sense_rank = 5`
  should have made it ≥ 5.

  **Finding: on the DEFAULT path the scope bug is masked by a coincidence.** The router's pied-piping
  detector (`parse_needs_unpacked`) finds the fronted preposition via `lookup_span`, which *is*
  scope-aware — so an out-of-scope preposition makes the router miss the construct entirely and divert
  the sentence to the packed path, which has no pied-piping rule. The scope survived by accident, not
  because the rule respected it. The test pins `with_packing(false)` (a supported configuration — the
  differential oracle's baseline), where the rule actually runs.

  Fixed by reading the preposition from chart cell `[p][p]`, where it already sits as a seeded token, and
  summing its cost like every other operand.

**Phase A+B result: the rules and both chart drivers are LEXICON-FREE.** `grep` for `self.lex` /
`entries_for` across `rules.rs`, `chart_packed.rs`, `chart_unpacked.rs` returns nothing. Seeding was
hoisted out of both drivers (`build_forest` now takes leaf cells; the flat CKY loop became
`Grammar::drive_unpacked`), so each driver file is now a clean two-way split:

| file | `impl Parser` (needs the lexicon) | `impl Grammar` (pure driving) |
|---|---|---|
| `chart_packed.rs` | `parse_packed`, `parse_packed_at_cap` | `build_forest`, `kbest`, `cube`, `binary_edges`, `materialize_unary` |
| `chart_unpacked.rs` | `parse_unpacked`, `parse_at_cap` | `drive_unpacked` |
| `rules.rs` | — | `binary_sites`, `apply_bin_rule`, `appositive_obj`, the unary shifts |

Verified: 142 parser (2 new), 51 lexicon, 1613 kernel unit; `fmt` + `clippy -D warnings` clean.

**Phase C — ATTEMPTED, REVERTED. The physical move is blocked by mis-homed helpers.**

`git mv` of `rules.rs` and the two chart drivers out of `lookup/` produced **195 compile errors**, and
they were not import noise — they exposed a bad cut:

- `raise_nps` (bounded **type-raising** — a *rule*) lives in `seed.rs`.
- `with_noun_num` (a *rule*-side category refinement) lives in `seed.rs`.
- `hole_base` / `freshen_anaphor` live in `felicity.rs`, but the **drivers** need them
  (`materialize_unary` re-freshens span-pure holes when it applies a unary shift).
- `beam_cell` / `cell_histogram` live in `chart_unpacked.rs`, but `seed_leaves` needs them (the leaf
  beam).

Every one of these could be forced through with a `pub(crate)`, and the move would compile — which is
precisely the trap. The helpers would stay in the wrong modules, and the new directory structure would
be a *claim* about the layering rather than a *fact* about it. Reverted.

**Phase C therefore needs a helper re-homing pass FIRST** (a Phase C0), moving each of the above to the
module whose concern it actually is. Only then is the directory move mechanical. The separation that
matters — `Grammar` vs `Parser`, and both charts being pure grammar operations — is already done and
green; what remains is where the files sit.

## Phases

### Phase A — `Grammar`: resolve the category templates once

- Introduce `Grammar { layer, reserved, templates }`, where `templates` carries the three raised
  categories currently re-fetched per call (`a` subject-raised, `these` subject-raised, `a_obj`
  object-raised). Resolved once, at `Parser::over`.
- Repoint `kind_raised_nps` and `appositive_obj` at the templates. `entries_for` calls (1) and (2) go
  away.
- `apply_bin_rule` / `binary_sites` then need only `Grammar`, not the lexicon — they can stop being
  `impl Parser`.

Risk: low (no behaviour change — the same category values, resolved once).
Guard: the differential oracle; the full suite.
Exit: oracle + suite green; measure the parse-time delta (expect a small speedup from the removed
per-item mutex locks — report it either way, including if it is nil).

### Phase B — the pied-piping smuggle

**Behaviour-changing, so the test comes first.**

- Write two failing tests that witness the bug as it stands:
  1. **scope bypass** — a scoped parse (`parse_scoped` with a lexicon list) that admits a pied-piping
     preposition from an out-of-scope lexicon;
  2. **cost under-count** — a pied-piping parse whose `Cost` omits the preposition's `lexicon_order` /
     `sense_rank`.
  If either cannot be constructed, say so and record why — do not fix a bug that cannot be witnessed.
- Then read the preposition from chart cell `[p][p]` (filtering by `is_vp_adjunct_prep` on the seeded
  items) and sum its cost like every other operand.
- Reach-back (3) goes away; the chart drivers become lexicon-free.

Risk: medium (changes which parses are admitted, and their ranking, for pied-piping sentences).
Guard: the two new tests, plus the oracle and the full suite.
Exit: the new tests pass; the suite is green; any forest that *changes* is explained (it should only
change for scoped parses and for pied-piping cost).

### Phase C — reorganize by concern

Only after A and B. Now a mechanical move (`git mv` where a file survives as a unit; the split ones
will show as delete+add unless committed as rename-then-edit — see the Phase-5 note in the
reorganization plan).

- Split `category.rs` into the algebra and `rules/constructions.rs`.
- `parser.rs` → `rules/combinators.rs` (it holds no parser; the name is a fossil).
- `lookup/rules.rs` + the unary shifts → `rules/registry.rs`.
- Merge `dcg/packed.rs` with `lookup/chart_packed.rs` into `chart/packed.rs`; `lookup/chart_unpacked.rs`
  → `chart/unpacked.rs`.
- `lookup/` retains what a bridge should: `lexicon.rs`, `seed.rs`, `felicity.rs`, `parse.rs`,
  `resolve.rs` — or those move up and `lookup/` disappears entirely.

Risk: low (pure code motion, no API change).
Guard: the oracle; `fmt` + `clippy -D warnings`.
Exit: suite green; every module's name describes its contents.

## Non-goals

- **Making pied-piping packable.** See above; it stays quaternary and the router carve-out stays.
- **Retiring the unpacked driver.** It is the differential oracle and the `combinatory_core` path. It
  stays regardless.
- **Marker-category nodes.** Closed in the reorganization plan's Phase 3 (built, measured at a ~10–15%
  parse cost, reverted).

## The pattern worth naming

Three fossils were found in one module: `LexicalIndex` (a lookup that had become a parser),
`cky_parse` (an entry point that had become a test harness), and `parser.rs` (a driver file that holds
no driver). Each drifted the same way — a rule needed something the layer next to it happened to have,
so the code moved to the dependency rather than the dependency moving to the code.

The Phase-4 fix generalizes: `Parser` holds `Arc<dyn LexicalLookup>`, a two-method trait, so parsing
cannot re-accrete onto the lexicon — the compiler refuses. **A concrete type invites accretion; a
narrow trait refuses it.** The same fence is what `Grammar` should be for the rules: a *value* they are
handed, not a *service* they can reach into.
