# Grammar formalization plan

Move the DCG grammar rules from imperative Rust to a **data-driven representation the
parser consumes**, so a grammar fix becomes a *rule edit* rather than a
reverse-engineer-instrument-guess loop. The categories are already formalized
(`data lexicon:Cat : Type 1` in `ontologies/lexicon/lexicon-ontology.esl`, denoted into
EigenTT by `⟦·⟧`); the **rule logic** (`combinable`, `build`/`build_refine`, the unary/binary
shifts) is what remains imperative.

## Why now

- **Grammar fixes are slow, and the code-embedding is the main cause.** The bare-mass `And`
  work is the exemplar: three failed structural attempts, each needing ad-hoc instrumentation
  (`MWP_DEBUG`, `GATE_DEBUG`) just to discover *which rule fired and what it produced*.
- **The rules came from rule-based references** (core-en's `typechanging` XML, the
  Chatzikyriakidis–Luo DCGs). Formalizing is *recovering* the declarative form the rules had
  before they were flattened into Rust — not inventing a representation.

## Goal / non-goals

**Goal:** a single-source-of-truth rule set (data) that the parser is generated from or
interprets, preserving (a) the typed CN-as-types semantics and the kernel felicity oracle, and
(b) full-lexicon performance. Success = *editing a rule is a data change* and the diagnosis of
"which rule does X" is a query, not an instrumentation session.

**Non-goals (for this effort):** flattening our dependent-typed semantics to a core-en-style
relational LF (that would throw away the type theory that is the platform's point); a from-scratch
parser rewrite; formalizing the sem *transformations* (deferred to Phase 4).

## Load-bearing principles

1. **The parser must CONSUME the rules.** A spec beside the code is a sync tax, not a speedup —
   the exact drift trap that has burned every hand-maintained doc in this repo. Single source of
   truth: the parser is **compiled from** (or interprets) the rule data.
2. **Formalize the syntactic/category layer first; keep sem construction as a per-rule code hook.**
   The category side is already near-data; this gets most of the velocity win without the hard
   sem-reflection (Phase 4). The sem hook is a named function reference the rule carries.
3. **Separate the universal CCG combinators from the grammar-specific rules** (core-en's core
   lesson). Application / composition / type-raising are a small fixed engine; only `KindCompound`
   / `Attrib` / coordination / the shifts are grammar data.
4. **Witnessed, not declared.** Every rule carries a golden characterization test (a minimal
   fragment → its `(category, sem)`); a **differential oracle** asserts the data-driven dispatch
   *exactly reproduces* the current hand-written behavior; every phase is gated by the
   deterministic `grammar-gap 0` sweep. Formalization must not change the grammar.
5. **Formalize toward the reference schema** (core-en / C&L categorial-grammar conventions),
   recovering structure — not fossilizing the current entanglements.
6. **ESL is the authoring wrapper; compile is the delivery.** Author rules in ESL (as `lexicon:Cat`
   already is); generate Rust from them for native perf. Start with an in-Rust rule table and only
   move authoring to ESL / on-chain once the model is proven (Phase 3).

## Rule representation and runtime consumption

This is the crux, so it is spelled out concretely. The key structural fact: the seam we are
formalizing **already exists** in the code. Today `combinable(left_cat, right_cat) -> Option<SemRecipe>`
makes the **sem-blind category decision** (which rule fires + the result's category shape), and
`build(recipe, left_item, right_item) -> Item` **constructs the sem**. We datafy the LHS
(`combinable`'s decision) and keep the RHS (`build`) as named sem-builders — the `SemRecipe` variant
is the migration boundary.

### The category-pattern language (a small new dispatch matcher, `match_cat`)

Categories are already `lexicon:Cat` terms (`Exp::InductiveCtor`: `cat_n`, `cat_np`, `fwd`=`A/B`,
`bwd`=`A\B`, `cat_s`, `cat_group`, `cat_pp`, …). A rule's operand/result patterns are **Cat terms
with metavariables** — `cat_n(?C, ?num)`, `bwd(cat_s(?_, adj), ?_)`.

**Correction to an earlier draft of this plan.** The dispatch matcher is *not* `unify_cat`. Witnessed
(a throwaway probe, since removed): `unify_cat` binds `cat_n(?C,?num)` but **fails** on `bwd(?x,?y)`
and `cat_s(?m, adj)`, and it *meets* features (`num_any ~ sg` → match) rather than matching them.
That is because `unify_cat` is the grammar's **combination** operation (subsumption on type indices,
contravariant functor args, feature-meet), and it only binds variables in **type-index / feature**
positions — never a whole subcategory. The grammar in fact has **three distinct category operations**:

| operation | matcher | binds | semantics |
| --- | --- | --- | --- |
| **combination** (functor + arg) | `unify_cat` | type-index / feature slots | subsumption + feature-meet (directional) |
| **coordination** (conjunct generalization) | `common_cat` | — | subtype-lattice join (anti-unification) |
| **dispatch** (which rule fires) | **`match_cat`** (new) | ANY position, incl. whole subcategories | exact structural match |

So Phase 1 adds one small primitive — `match_cat` (`category.rs`, ~15 lines) over a `CatPat` term —
that matches a pattern structurally and binds metavars anywhere. Result instantiation still reuses
`subst_cat`. `unify_cat` / `common_cat` are untouched, in their proper combination/coordination roles.

### The descriptor (a tagged union whose *types* enforce sem-blindness)

The schema is designed so that a sem-reading guard on a **packed** rule is *unrepresentable*, not
merely discouraged — the packed-forest soundness invariant is enforced by the compiler, not by
reviewer vigilance. The split is: **packed rules** (`binary_category`, `unary`) decide sem-blind on
`(cat_shape, ENF-prov)`; **item-level rules** (`binary_token`) run off the packed path and are the
*only* place a sem may be read.

```rust
enum GuardArg { Cat(CatRef), Prov(OperandRef), Lit(Literal) }   // NB: no Sem variant
struct CatGuard { pred: PredId, args: Vec<GuardArg> }           // sem-blind BY CONSTRUCTION

struct SemGuard { pred: PredId, arg: OperandRef }               // sem-reading; item-level only

enum Rule {
    BinaryCategory {                     // the `combinable` rules — packed, sem-blind
        name, priority,
        left_pat: CatPat, right_pat: CatPat,
        guards: Vec<CatGuard>,           // <- cannot mention a sem
        result_pat: CatPat,              // dispatch shape; `§` marks the sem-hole
        sem_builder: BuilderId, combinator: Combinator, cost: Cost,
    },
    Unary {                              // the shifts — packed, sem-blind
        name, pat: CatPat, guards: Vec<CatGuard>, sem_builder: BuilderId,
    },
    BinaryToken {                        // the `BinRule` rules — item-level, off the packed path
        name, token: TokenPat, geometry: SpanGeom,
        guards: Vec<CatGuard>,           // cat/prov guards, and additionally:
        sem_predicate: Option<SemGuard>, // <- the ONLY place a sem may be read
        sem_builder: BuilderId,
    },
}
```

The enforcement lives in the types: `BinaryCategory`/`Unary` carry only `Vec<CatGuard>`, and
`CatGuard`'s `GuardArg` has **no `Sem` variant** — so a packed rule *cannot be written* to gate on a
sem; the ill-formed And-gate we tried at the wrong layer would fail to typecheck here. A rule that
genuinely must read the sem is, by that fact, a `BinaryToken` (item-level) rule — the classification
follows from the schema. Notes: `result_pat` is the erased **dispatch shape** (`§` = the sem-hole the
`sem_builder` fills); `priority` is the explicit dispatch order (mirroring `combinable`'s arm order);
`PredId` / `BuilderId` are names into the fixed predicate / builder libraries (§ above).

### Worked examples — our actual rules, as data

```text
rule kind_compound {                     # [cat_n][cat_n] -> compound_kind   (grammar-specific)
  kind:        binary_category
  left_pat:    cat_n(?C, ?_)
  right_pat:   cat_n(?D, ?num)
  guards:      [ not_compound_refined(right) ]
  result_pat:  cat_n(§, ?num)            # dispatch shape; Σ-restrictor built by sem_builder
  sem_builder: refine_kind_compound      # builds  Σx:?D. compound_kind(x, ⟦left⟧)
  combinator:  Compound   cost: +compound_step
}

rule forward_app {                       # A/B · B -> A   (UNIVERSAL — sem simple enough to datafy now)
  kind:        binary_category
  left_pat:    fwd(?A, ?B)
  right_pat:   ?B                         # unify ?B against the right cat
  guards:      [ left_not_typeraised, left_not_fwdcomp ]
  result_pat:  ?A                         # instantiated by subst from the binding
  sem_template: App(?L, ?R)              # no code hook: a pure term template
  combinator:  ForwardApp   cost: 0
}

rule attrib {                            # [S[adj]\NP][cat_n] -> attributive-Σ  (the bare-mass And lives here)
  kind:        binary_category
  left_pat:    bwd(cat_s(?m, adj), cat_np(?_, ?_))
  right_pat:   cat_n(?C, ?num)
  result_pat:  cat_n(§, ?num)
  sem_builder: refine_attrib             # Σx:?C. (flat-And if ?C already refined, else adj(x))
}

rule coordinate_and {                    # [X] and [Y] -> same-cat conjunction / cat_group
  kind:          binary_token
  token:         and | conn:conn_and
  sem_predicate: sem_is_coordination     # the sem-reading escape hatch
  sem_builder:   coordinate_sem
}

rule bare_np_shift {                     # cat_n(pl|mass) -> raised bare-argument NP
  kind:        unary
  pat:         cat_n(?C, ?num)
  guards:      [ num_in(pl, mass) ]
  sem_builder: bare_nominal_shift
}
```

Two structural points these make concrete: (1) the **universal** combinators (`forward_app`, …) are
*also* descriptors, but their sems are simple enough to be `sem_template` data from day one — so the
"universal engine vs. grammar rules" split is really "simple-template rules vs. code-hook rules."
(2) For a **refine** rule the result category is `cat_n(Σx. R, num)` — the CN-as-types fusion means
the restrictor `R` *is* the sem; `result_pat` therefore carries only the **erased shape**
(`cat_n(§, num)`) for dispatch, and the sem-builder produces the Σ body. This is why the category
side datafies cleanly while the sem side needs a builder.

### What the parser consumes at runtime

Per adjacent cell-pair `(L, R)` in the CKY node loop:

1. **Dispatch (pure data).** Walk the binary_category descriptors — indexed by the operands' outer
   constructors so only relevant ones are tried (matching today's branch structure → no perf loss).
   For a candidate: `match_cat(left_pat, L.cat, σ)` then `match_cat(right_pat, R.cat, σ)` into one
   shared `σ`; if both match and every `guard` holds, the rule fires; the `sem_builder` gives the
   result. **This table walk replaces the hand-written `combinable` match** — as now done for the
   nominal-modification family in `combine_nominal_mod`.
2. **Sem build (named builder → template).** Invoke the descriptor's `sem_builder` (Rust in
   Phase 1–2) with `(L.sem, R.sem, bindings)` → the full result `Item` (category + Σ-sem + cost +
   `combinator`). In Phase 4 a builder becomes a `sem_template` instantiated by substitution — the
   universal rules already are.

Token rules dispatch by boundary token + span geometry (gated by `sem_predicate`); unary rules apply
per cell by their `pat` + `guards`. **So the runtime artifact is a table of rule descriptors** —
Cat-pattern data plus named guard / sem-builder references — walked per cell-pair. Category
*dispatch* is fully data; sem *construction* is a named builder (code first, template later).

### `guards` and `sem_builder` are named references into small libraries

Guards (`not_compound_refined`, `is_adj_clause`, `num_in`) and sem-builders (`refine_kind_compound`)
are **named entries in a fixed library**, referenced by string/id in the descriptor. Phase 1–2 those
libraries are Rust fns; the descriptor *table* is the data. This keeps the escape hatch honest: a
genuinely program-like rule references a named predicate/builder, but the **dispatch and the rule
inventory stay data**. Phase 4 migrates the builders (and, where clean, the guards) to term
templates / EigenTT programs, shrinking the library toward empty.

### Metavariable & guard grammar (detailed)

**Sorts come from the on-chain `Cat` signature — for free.** `lexicon:Cat`'s constructors are typed
(`cat_n : Set → lexicon:Num → lexicon:Cat`, `fwd : Cat → Cat → Cat`, `cat_s : Mood → Fin → Cat`, …),
so a metavar's **sort is fixed by its position**: in `cat_n(?C, ?n)`, `?C : Set` (a noun/entity
class), `?n : Num`; in `fwd(?A, ?B)`, `?A,?B : Cat`. The matcher enforces sort structurally — a `Num`
metavar can only bind a `Num`. No separate sort declaration is needed; it is read off the inductive.

**A pattern is a `CatPat` term with metavar leaves; matching is `match_cat`** (the dispatch matcher,
per the table above — not `unify_cat`). Concretely:

- **Literals match exactly:** `cat_n`, `bwd`, and *specific* feature values (`adj`) must be present in
  the subject, by ctor name + arity (the `decl` is ignored, as in `is_ctor`).
- **Metavars `?x` bind** the matched subterm into a `CatSubst` — in **any** position: a whole
  subcategory (`?adjclause` in `bwd(?adjclause, ?_)`), a type index (`?C`), or a feature (`?num`).
  This is the capability `unify_cat` lacks. Matching is **exact** — no subsumption, no feature-meet,
  so a `num_any` in the subject matches only a `num_any` pattern position, never `sg`.
- **`?_` is an anonymous wildcard** (matches, never binds, never checked).
- **Non-linearity is an equality constraint.** A metavar used twice forces its sites to bind equal
  terms. Left-pattern then right-pattern thread one `CatSubst`, so a metavar shared across operands
  must agree — a mismatch fails the rule.

**Scope & freshness.** Metavars bound by `left_pat`/`right_pat` are in scope for `guards`,
`result_pat`, and the `sem_builder`. The **Σ-bound instance var** (`x` in `Σx:C. R`) and any other
new binders are **not** pattern metavars — the sem-builder generates them fresh; a Phase-4
`sem_template` gets a `fresh` primitive for the same job. Result instantiation is
`subst_cat(result_pat, subst)`.

**Guards are named predicates over bindings + operand provenance — and, for `binary_category`,
strictly sem-blind.** A guard is `pred(arg, …)` where each arg is a metavar ref (`?C`), a whole
operand (`left` / `right`, an `Item`), or a literal set. The library (Phase 1–2 = Rust fns, keyed by
arg sort) is small and already exists as today's inline checks:

| predicate | arg(s) | reads | today |
| --- | --- | --- | --- |
| `is_compound_refined` / `not_compound_refined` | `cat` | Σ-restrictor App-head is `compound`/`compound_kind` | `combinators.rs` |
| `is_sentence_premod` / `is_finite_clause` / `is_vp_adjunct_prep` | `cat` | structural | `category.rs` |
| `num_in` | `num`, `{pl,mass,…}` | a `Num` metavar's ctor | inline `matches!` |
| `not_typeraised` / `not_fwdcomp` | `item` | `Item.prov` (`Combinator` tag) | `combinable` flags |

The **sem-blind constraint is load-bearing**: a `binary_category` guard may read a bound *category*
metavar's structure and an operand's *provenance* (`Combinator`), but **never a sem** — this is what
keeps the dispatch a function of `(cat_shape, ENF-prov)` and therefore **sound against the packed
forest signature**. Rules that must read the sem are `binary_token` only and declare a
`sem_predicate` (e.g. `sem_is_coordination`) — they already live off the packed path (`apply_group`).
So the grammar of guards is: *sem-blind category/provenance predicates for the packed rules; a
sem-reading escape hatch confined to the item-level token rules.*

**Ordering is data.** Multiple descriptors can match one cell-pair when their patterns overlap
(forward *application* vs forward *composition* both want a `fwd` left). Today `combinable` resolves
this by **arm order** (first match wins, one recipe per pair). The descriptor table therefore carries
an explicit **priority order**, dispatch is first-match, and that order is part of the data the
differential oracle must reproduce. (Distinct outer constructors don't overlap, so ordering only
matters within a constructor family — which is also how the constructor-indexed dispatch stays fast.)

**Worked match trace** (`kind_compound`, an actual Phase 1 rule — this is what the code now does).
`L.cat = cat_n(Mmr, sg)`, `R.cat = cat_n(Gene, sg)`:

```text
1. match_cat(cat_n(?_,?_),   L.cat) -> ✓ (left is a common noun; nothing bound)
2. match_cat(cat_n(?C,?num),  R.cat) -> σ = { ?C ↦ Gene, ?num ↦ sg }   (same σ, threaded)
3. guards: not_compound_refined(right) -> R.cat's Σ-restrictor is not a compound App-head ✓
4. build: refine_kind_compound(σ, L, R) ->
     cat_n( Σx:Gene. compound_kind(x, ⟦L⟧), sg )   -- the sem-builder; code until Phase 4
```

Attributive adjective is the same shape with a richer left pattern that carries the whole trigger:
`left_pat = bwd(cat_s(?_, adj), ?_)` — the `adj` fin literal *is* the old `is_adj_clause` check, now
expressed in the pattern (which is why that guard is gone). No guard is needed for it.

The **universal combinators** (`forward_app`, etc.) stay hand-written this phase for a structural
reason worth recording: their dispatch is *not* pure structural match. After destructuring the
functor `fwd(?A, ?B)` they must **combine** the argument slot `?B` with the other operand via
`unify_cat` — subsumption, the directional subtype check (`Gene ≤ Entity`), plus feature-meet. So
datafying them (Phase 2) needs a rule kind whose pattern carries an explicit *combination constraint*
on an arg slot, distinct from the grammar-specific structural rules here. `match_cat` alone is the
right tool for the nominal-modification family (pure structure + one semantic guard); it is not, by
itself, the tool for the combinators.

## Generation & artifacts

- **Generator** reads the descriptor table → emits the dispatch (`combinable`) and the
  builder-dispatch (`build`) — or an interpreter walks the table directly. Interpret first for
  correctness; **codegen if the hot CKY loop can't afford runtime unification** (decide by the
  Phase 1 benchmark — the generated match can reproduce today's constructor-indexed branch exactly).
- **Home:** the descriptor table lives in Rust (Phases 1–2) → ESL resources compiled to Eigon-JSON,
  committed to a layer (Phase 3).
- **Derived:** a generated inventory doc + golden per-rule tests (the witnessed spec).

## Phases (each independently valuable, each gated)

### Phase 0 — Legibility groundwork (precondition; days)

- **0a. Derivation tracer** (`EIGENIUS_DERIVATION=1`): per reading, print the rule tree
  (combinator + operand categories + result), reusing the chart's `Combinator` provenance +
  forest edges. Replaces the ad-hoc debug probes with a standing tool.
- **0b. Rule inventory**, code-anchored + golden-test-witnessed: enumerate the current rules
  (trigger / result / where). This is the map of what Phase 1+ formalizes.
- **0c. Universal-vs-grammar split:** factor `combinable` so the generic CCG combinators are
  visibly separate from the grammar-specific refine/shift/coordination rules.
- **Exit gate:** the tracer names every rule in a live parse; the inventory lists them with
  passing golden tests; the split lands with `grammar-gap 0` and the reranked encoded floor
  unchanged.

### Phase 1 — Vertical slice: prove the compile loop on ONE family (the key validation; ~1 week)

- Target the **nominal-modification family** (`KindCompound` / `Attrib` / `NamedCompound` /
  `PpMod`) — the rules tangled in the bare-mass bug.
- Define the rule-descriptor schema (Rust data first). Express these four as data (trigger +
  result category schema + `sem_hook`).
- Build the generator/interpreter for just this family; wire the parser to dispatch from the data.
- **Exit gate (differential + behavioral):** the data-driven dispatch **exactly reproduces**
  current parses for this family (differential-oracle test); `grammar-gap 0` and encoded floor
  unchanged; and a rule edit (change a `result` category) changes parse behavior with a
  data-only diff. Benchmark: no perf regression (decide interpret vs codegen here).

### Phase 2 — Roll out family-by-family (the velocity payoff; the bulk)

- Convert the remaining grammar-specific rules: the rest of the binary category rules; the
  token-keyed binary rules (coordination, relatives, apposition — `BinRule`); the unary shifts
  (`bare_nominal_shifts`, `raise_nps`, kind-raise, `complete_coord`, `front_participial`).
- Each family: express as data → differential-check against current behavior → sweep-gate. Sem
  construction stays code hooks throughout.
- **Exit gate:** all grammar-specific rules are data; `combinable`/`build` dispatch is generated,
  not hand-written; full-page sweeps (deterministic + reranked) unchanged; perf within budget.
  **This is where "a fix is a data edit" is realized.**

**Progress.**

- **2a — "other grammar" binary rules — DONE** (byte-identical sweep). Close-naming apposition and
  the GQ-as-preposition-object raise (3 kinds) are now table rows in `other_grammar_rules`; the
  descriptor generalized (`RefineRule`→`CatRule`, `RefineBuilder`→`SemBuild`, `SemRecipe::Refine`→
  `SemRecipe::Rule`); the guard library grew one entry (`ProperName`); the `SemRecipe::Name` /
  `GqPrepObj` variants and `PrepObj` enum are gone (subsumed by the generic builder path). 10 golden
  tests (both families) + 1618 lib tests + `grammar-gap 0` byte-identical.
- **2b — universal combinators + dependent determiner — DONE** (byte-identical sweep). These are the
  *combination-constraint* rule kind: after destructuring a functor they `unify_cat` an argument slot
  against the other operand (subsumption + feature-meet), which can FAIL — so the combination is part
  of the dispatch, not just the build. Represented as a `CombRule` table (`comb_rules`) with
  `CombKind::{Apply{functor,slash}, Compose{slash}, DepApply}` and per-rule Eisner `ProvGuard`s; the
  interpreter (`CombKind::combine`) does the destructure + `unify_cat` + `subst_cat` + `feat_meets`.
  The determiner folded in as `DepApply` (polymorphic instantiation, a bespoke interpreter arm). The
  separate `combine_determiner` is gone; **`combinable` is now fully table-driven** (`combine_universal`
  → `combine_nominal_mod` → `combine_other_grammar`, all interpreters). 5 combinator golden tests
  (incl. the DetRefine Fst-projection) + 1623 lib tests + `grammar-gap 0` byte-identical.
- **2c — token-keyed binary rules (`BinRule`: coordination, relatives, apposition) — DONE**
  (byte-identical sweep). This family was already a well-factored registry (enumerated `BinRule`,
  centralized `binary_sites` geometry, named-fn builders); 2c unified each rule into ONE `TokBinRule`
  descriptor (`trigger` geometry fn + `build` fn + `reads_sem`), and made `binary_sites` /
  `apply_bin_rule` interpreters over the `bin_rules()` table (the `BinRule` tag carries the
  coordination connective, so builder dispatch is keyed by a `BinKind` discriminant). The
  **`sem_predicate` escape hatch is now an explicit `reads_sem` declaration** — pinned by the
  `escape_hatch_matches_sig` test to exactly {Coordinate, ButNot}, the rules `Sig` carries the
  coordination bit for. 2 escape-hatch invariant tests + 1628 lib tests + `grammar-gap 0`
  byte-identical.
- **2d — unary shifts — DONE** (byte-identical sweep). The composed-cell shifts (coordination
  completion, bare-nominal, **type-raise/kind-raise**, fronted participial) are now one ordered
  `unary_shifts()` table (`kind` + per-item `apply` fn), consumed by all THREE former shift sites —
  the unpacked CKY (extends the cell), the packed forest builder (adds `Edge::Unary`), and
  `materialize_unary` (re-applies at extraction) — eliminating the triplicated orchestration. The
  load-bearing order (bare-nominal before type-raise) is table order; `AbsorbComma` stays inline (a
  sentence-initial cross-cell special case). Verified per-item-independent (so the packed per-item and
  unpacked whole-cell applications are equal). 1630 lib tests + `grammar-gap 0` byte-identical.

**Phase 2 is complete.** Every grammar-specific rule is now data: `combinable` (universal combinators,
nominal-modification, other-grammar), the token-keyed `BinRule` family, and the unary shifts are all
table-driven interpreters. The sem/trigger *logic* stays as named per-rule functions (principle 2);
the *rule set* — which rules exist, their triggers, guards, order, escape-hatch declarations — is
data. Per the replicate-then-fix plan, the bare-mass `And` fix is now the next step: an isolated edit
to the kind-raise rule (or a guard), verified by the ambiguity delta + targeted look-alike golden
tests, not the byte-identical gate.

### Phase 3 — ESL authoring + on-chain (platform-native)

- Move the rule data from Rust structures to **ESL resources** (authored like `lexicon:Cat`),
  compiled to Eigon-JSON, committed to a layer; the generator reads the on-chain rule set.
- **Exit gate:** the grammar rule-set is on-chain ESL, validated by the commit gate; "which rule
  emits `S[adj]\NP`?" is an EigenQL query; rule changes are chain commits with provenance;
  behavior + perf unchanged.

### Phase 4 — Sem reflection (the hard core; deferred / optional)

- Formalize the sem *transformations* as typed EigenTT programs over reflected terms (D47
  `eigentt:TypeExpr` `Exp↔Json` codec), eliminating the last code hooks — the full grammar
  (syntax + semantics) becomes typed data.
- Only if Phases 1–3 prove the model and the payoff justifies the effort. High risk / high effort.

## Cross-cutting invariants

- **Non-negotiable gate:** deterministic `--no-llm` sweep holds `grammar-gap 0` /
  `missing-lexeme 0` at every step; reranked encoded floor does not regress. Fail-closed.
- **Differential oracle:** a test that the generated dispatch reproduces the hand-written parses
  (this is what makes "formalization changed nothing" a *witnessed* claim, not a hope). It is the
  hand-written code's role for the whole rollout, retired only when a family is fully migrated and
  differential-clean.
- **Golden characterization tests** per rule = the witnessed inventory; a rule change fails its
  snapshot, forcing a deliberate update.
- **Performance** measured per phase; the compile path exists precisely to keep native speed.

## Risks & mitigations

- *Sem-hook boundary awkward* (categories datafied, sems in code) → Phase 1 tests exactly this on
  4 rules before committing.
- *Some rules are genuinely program-like* (coordination reads the sem) → the code-predicate escape
  hatch; datafy the declarative majority, not everything.
- *Perf regression from interpreted dispatch* → benchmark in Phase 1; codegen from the same data if
  needed.
- *Formalizing the mess* (baking in the bare-mass entanglement) → formalize toward the reference
  schema; and entanglement fixes can ride the migration (a rule edit once the rule is data).
- *Scope creep / never-ending* → Phases 1–2 deliver the velocity win and are the committed scope;
  Phases 3–4 are optional platform-native upgrades gated on proven value.

## Payoff timeline

- After **Phase 0**: legible grammar (tracer + inventory) — faster diagnosis immediately.
- After **Phase 1**: proven compile loop on one family — architecture validated.
- After **Phase 2**: all rules data-driven, parser generated — **fixes become data edits** (the
  goal).
- After **Phase 3**: on-chain, queryable, versioned grammar — platform-native.
- **Phase 4**: full typed grammar — the endgame.

## Open decisions (resolve before/within Phase 1)

1. **Interpret vs codegen** for the hot CKY dispatch — decide by the Phase 1 benchmark.
2. **Descriptor home** — Rust data table (Phases 1–2) then ESL (Phase 3), as planned, vs jump
   straight to ESL. Default: Rust-first to de-risk the dispatch before the authoring move.
3. **Escape-hatch surface** — how a rule declares a sem-reading predicate (coordination) in the
   descriptor without leaking the whole imperative shape back in.

## Immediate next action

Build the **Phase 1 nominal-modification slice** as the proof-of-loop: descriptor schema (Rust) +
the four rules as data + generator/interpreter + differential-oracle test + the deterministic
sweep. If the loop is clean, expand family-by-family; if the sem-hook boundary is awkward, we
learn it on four rules, not forty. The full spec for that slice is the appendix below.

---

## Appendix: Phase 1 slice — the nominal-modification family, fully specified

The slice datafies exactly the four `SemRecipe::Refine` arms of `combinable`
(`rules/combinators.rs` §207–263) plus their `build_refine` assembler (§463–535). Everything else
in `combinable`/`build` stays hand-written this phase: the determiner arm (`DetRefine`), forward/
backward application, forward composition, the Eisner-NF provenance guards, close-naming apposition,
`GqPrepObj`, and the coordination carve-out (`apply_group`, sem-reading → item-level). This is the
smallest cut that exercises the whole loop — pattern dispatch, a category guard that inspects a
Σ type-index, a sem-builder that reads both operands — while touching one self-contained family.

### The four rules as descriptors

All four are `BinaryCategory` rules. Metavars: `?C` (component type), `?num` (number feature),
`?m` (mood), `?_` (anonymous). Literals are ctor names from the `lexicon:Cat` signature. Every
`result_pat` is `cat_n(§, ?num)` — the head noun's number, refined restrictor in the sem-hole `§`.

```text
attrib            # attributive adjective  (D63 §8.5 Slice 3b)
  left_pat   bwd(cat_s(?m, adj), ?_)        # a predicative adj clause S[_,adj]\NP
  right_pat  cat_n(?C, ?num)
  guards     []                             # adjectives stack even on compound-refined nouns
  result_pat cat_n(§, ?num)
  sem        refine_attrib                  # flat-Σ conjunction (see below)
  combinator Compound     priority 1

named_compound    # named-entity compound  [cat_np][cat_n]  (D63 §8.13)
  left_pat   cat_np(?_, ?_)
  right_pat  cat_n(?C, ?num)
  guards     [ not_compound_refined(right) ]   # left-branching NF: head is not itself a compound
  result_pat cat_n(§, ?num)
  sem        refine_named_compound
  combinator Compound     priority 2

kind_compound     # N-N kind compound      [cat_n][cat_n]   (D63 §8.13)
  left_pat   cat_n(?_, ?_)
  right_pat  cat_n(?C, ?num)
  guards     [ not_compound_refined(right) ]   # SAME guard/block as named_compound
  result_pat cat_n(§, ?num)
  sem        refine_kind_compound
  combinator Compound     priority 3

pp_mod            # post-nominal PP modifier  [cat_n][cat_pp]
  left_pat   cat_n(?C, ?num)
  right_pat  cat_pp(?_)
  guards     []
  result_pat cat_n(§, ?num)
  sem        refine_pp_mod
  combinator Compound     priority 4
```

The four `(left_ctor, right_ctor)` keys are pairwise disjoint — `(bwd-adj, cat_n)`, `(cat_np, cat_n)`,
`(cat_n, cat_n)`, `(cat_n, cat_pp)` — so dispatch is deterministic regardless of order. `priority` is
kept anyway, to reproduce `combinable`'s arm order exactly (the differential oracle demands byte
identity, not just extensional agreement).

### Guards — the family confirms the sem-blind schema

The only guard the family needs is `not_compound_refined(right)`. It inspects the right operand's
**category**: `cat_n(Σx:Base. body, _)` with the restrictor's App-spine head equal to the
`ontology:compound` / `compound_kind` axiom (`is_compound_refined`, §710). That reads the Σ **type
index of the category** — never `right.sem()` — so it is a `CatGuard` with a single `Cat(right)`
argument, exactly the schema's packed-rule guard shape. The trickiest predicate in the family is
still sem-blind; this is the concrete evidence the enforced schema is not too tight. (`attrib`'s
trigger — "is the left an adjectival clause" — needs **no** guard at all: the `adj` Fin literal sits
in `left_pat` = `bwd(cat_s(?_, adj), ?_)`, so `match_cat` decides it structurally.)

Predicate-library entry consumed by this slice:

```text
not_compound_refined(c: Cat) -> bool     # negation of is_compound_refined (combinators.rs §710)
```

### Sem-builders — a 1:1 extraction of `build_refine`, not a rewrite

Each `sem` name is one arm of the existing `build_refine` (§472–534), lifted to a named builder
`fn(binds: {C, num}, left: &Item, right: &Item, layer) -> Item`. No logic changes — the extraction
is what makes the differential oracle trivially green on day one.

```text
refine_attrib          if ⟦C⟧ = Σ bx:Base. P(bx) and logic:And resolves:
                          cat_n( Σ bx:Base. And(P(bx), ⟦left⟧(bx)), num )   # FLAT Σ, same base
                        else:
                          cat_n( Σ x:C. ⟦left⟧(x), num )
refine_named_compound  cat_n( Σ x:C. compound(x, ⟦left⟧), num )
refine_kind_compound   cat_n( Σ x:C. compound_kind(x, ⟦left⟧), num )
refine_pp_mod          cat_n( Σ x:C. ⟦right⟧(x), num )
```

Each reads its operands' **sems** (`⟦left⟧` / `⟦right⟧`) — but that is the builder, not a guard: sem
access is permitted where the descriptor already committed to firing, forbidden only in the decision
of *whether* to fire. All four emit provenance `Compound` and `Cost::ZERO`, matching `build_refine`.
`refine_attrib` is the arm where the bare-mass `And` is actually constructed — datafying this family
puts that construction behind a named, traced, edit-in-one-place builder, which is the whole point of
starting the slice here.

### Integration & dispatch order

`combinable` keeps its universal arms in place; where its four `Refine` arms currently sit, it
consults the refine-rule **table** (try descriptors in `priority` order; first whose patterns unify
and whose guards hold wins). Because the universal arms (`DetRefine`, fwd/bwd app, fwd comp) return
early and match disjoint category shapes, moving only the refine arms behind the table preserves the
exact firing order. `build` routes a table hit to the descriptor's named sem-builder in place of the
`match kind { … }` it does today.

### Differential-oracle scope & exit criteria

- **Oracle corpus:** fragments exercising each rule and the guard's boundary — attributive adjective
  on a bare noun and on an already-adjective-refined noun (the flat-Σ path); named + N-N compounds;
  a 3-noun chain (the `not_compound_refined` left-branching cut); a post-nominal PP; and negative
  cases where no refine rule should fire. For each, assert the datafied dispatch yields a forest
  whose items are **byte-identical** (category + sem + provenance + cost) to the hand-written path.
- **Sweep gate (non-negotiable):** deterministic `--no-llm` sweep holds `grammar-gap 0` /
  `missing-lexeme 0`; the reranked encoded floor does not regress.
- **Behavioral proof:** editing one descriptor (e.g. `kind_compound.result_pat` or its guard)
  changes parse behavior through a **data-only** diff — the loop's payoff, demonstrated once here.
- **Perf:** benchmark the table dispatch against the hand-written arms; if interpreted lookup costs
  too much on the hot CKY path, codegen the same descriptors (Open decision 1, decided here).
