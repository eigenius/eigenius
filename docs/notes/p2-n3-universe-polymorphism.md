# P2 · N3 — Universe polymorphism for EigenTT sorts

Note N3 of the [P2 plan](p2-type-theory-soundness-plan.md) §2. Settles the design questions
[#188](https://github.com/eigenius/eigenius/issues/188) defers. Written `2026-08-22` from `67e781f`.

> **SUPERSEDED `2026-08-22`: BUILD IT.** The maintainer directed that #188 proceed on §5a's
> reasoning — Cooper's TTR is coming and uses universe polymorphism, so the ladder will be climbed
> rather than stepped on twice. §6's hold no longer applies; §§2-4 and §7 stand as the design, and
> §5's counterweight stands as the honest cost. Build log at the bottom.
>
> The original recommendation follows, kept because its measurements are the baseline the work is
> judged against.

**Recommendation (superseded): do not build it yet — the trigger has not fired — but the three design
questions are settled here so that when it does, the answer is not re-derived.** The representation
question in particular is already decided by a precedent nobody has noticed.

## 1. The trigger, measured

#188 states its own condition: *"Pick this up when a second level bump is proposed — that is the
signal the ladder is real rather than hypothetical."*

Measured across `ontologies/`, `experiments/` and `demo/`:

| surface sort | uses |
|---|---|
| `Prop` | 712 |
| `Set` | 230 |
| `Type 1` | **2** |
| `Type 2` | **0** |

The two `Type 1` uses are the whole of the ladder above `Set`:

- `ontologies/reasoning/reasoning.esl:186` — `spec_poly`'s domain binder, raised from `T : Set` by
  #136/PR #187, which is what prompted #188.
- `ontologies/lexicon/lexicon-ontology.esl:250` — `data lexicon:Cat : Type 1`.

`Type 2` occurs once in the repository, and it is a **comment** at `reasoning.esl:183` describing the
hypothetical ladder ("*quantifying over `Type 1` domains would need `Type 2`, and so on up the
ladder*"). The two other textual matches are in `experiments/lexicon-align/alignment.jsonl` and refer
to diabetes.

**No second bump has been proposed.** One rule and one declaration sit at `Type 1`; nothing needs
`Type 2`.

## 2. Representation — settled by precedent

The chain already carries a level algebra, and it is exactly nanoda's.
`ontologies/lean/lean-expressions.eigon.json` declares `urn:eigenius:lean:LeanLevel`:

```
Zero  []
Succ  [base : LeanLevel]
Max   [left : LeanLevel, right : LeanLevel]
IMax  [left : LeanLevel, right : LeanLevel]
Param [name : LeanName]
```

Five constructors, matching `references/nanoda_lib/src/level.rs` (`IMax` `:16`, `Param` `:17`) one
for one. It was built for D30's Lean mirror and it is bootstrap-resident.

**So `eigentt:Level` should mirror that shape** — not because symmetry is pleasant, but because the
translation `#159`/D74 needs is then structural instead of special-cased. Today externalization
would have to map `Exp::Sort(usize)` onto `LeanLevel` by construction; with an isomorphic algebra it
is a fold.

**It cannot be reused directly.** Bootstrap layer order is
`core → eigentt-type-fragment → program → reflection → obo → institution → runtime → formulas →
lean-expressions → …`. `eigentt` is the second layer and `lean-expressions` the ninth; a lower layer
cannot reference a higher one. A parallel `eigentt:Level` is required. Its *shape* is nonetheless
settled, and any divergence from `LeanLevel` should be justified rather than accidental.

On the Rust side: `Exp::Sort(usize)` → `Exp::Sort(Level)` with

```rust
enum Level { Zero, Succ(Box<Level>), Max(Box<Level>, Box<Level>), IMax(Box<Level>, Box<Level>), Param(Name) }
```

Port `simplify` (`level.rs:55`), `subst_levels` (`:101`), `leq_core` (`:176`) and `leq` (`:229`) from
nanoda — 288 lines total, and the `IMax` cases in `leq` are the part not worth re-deriving.

## 3. Does the surface get levels? — No, not initially

Universe polymorphism has two halves: level *variables* in binders, and level *arguments* at use
sites (nanoda's `Const(name, levels)`, `expr.rs:445`, with declarations carrying `uparams`,
`env.rs:39`).

**ESL should not grow level syntax in the first landing.** Reasons:

- The surface today writes `Prop`, `Set`, `Type k` (`esl/compile.rs:1437`), and 942 of the 944 uses
  are the two monomorphic keywords. Adding `Sort u` syntax serves two sites.
- Level *inference* at declaration sites covers the actual need: `spec_poly` wants "this rule works
  at any level", which is a `uparams` list the elaborator can generalise, not something an author
  should hand-annotate.
- Surface syntax is the hardest part to reverse. The representation and the algebra can land, be
  exercised through inference, and grow explicit syntax later if authors need to constrain a level.

Concretely: `Level::Param` exists in the representation from day one, declarations gain a `uparams`
list, and the elaborator generalises free levels at declaration boundaries. No new ESL token.

## 4. Migration — smaller than it looks, and the reason is worth checking before relying on it

**No chain source in this repository carries an encoded sort.** `grep '"ctor":"Sort"'` across
`ontologies/`, `experiments/` and `demo/` returns **zero**. Sorts appear in ESL *source* (`: Type 1`,
`forall (T : Set)`), which is compiled, and in the D47 codec's arms — but no committed chain term in
the repo is a `Sort` node.

The codec's arms are `eigentt_type_mirror.rs:98` (`Exp::Sort(n)` → `{"ctor":"Sort","args":[n]}`) and
`:409` (decode, reading an integer). The chain-side ctor is declared at
`ontologies/eigentt/eigentt-type-fragment.json:10` with a single `level` argument.

So migration has two parts:

1. **The chain inductive changes**: `Sort`'s `level` argument goes from an integer to an
   `eigentt:Level` reference. That is a bootstrap-ontology edit — the manifest moves, and a reseed is
   owed. **Fold #213 into that reseed** (the plan's §5): it is the one bootstrap-touching item left
   in P2, and #213 costs a reseed of its own to adopt.
2. **Already-persisted terms must still decode.** The repo has none, but the served RocksDB stores
   are not in the repo. **Verify against a snapshot before committing to a decode strategy** — the
   cheap answer is for the decoder to accept a bare integer as `Succ^n(Zero)` alongside the new
   form, which costs one arm and removes the question.

## 5. What this buys, and the honest counterweight

The gain is that a rule quantifying over a domain at any level is writable once, instead of one
declaration per level with a bootstrap edit and a reseed each. #136/#187 paid that cost once to go
from `Set` to `Type 1`.

The counterweight is that **the cost has been paid once, for one rule, and nothing has asked for it
since.** Universe polymorphism touches `Exp`, `Val`, conversion, inference, readback, the D47 codec,
the chain inductive, and declaration/use-site plumbing — 509 non-test references to `Exp::Sort` /
`Val::Sort` across 12+ modules is the blast radius, and Rule 21 puts the result on the commit gate.
That is the largest single change in P2, in service of a ladder with two rungs occupied.

## 5a. The trigger that is actually likely: Cooper's TTR

**Recorded from the maintainer, `2026-08-22`:** Eigenius will likely incorporate techniques from
Cooper's *From Perception to Communication* ([`cooper2023perception`], D18 §9 / D61 §10 / D62 §3),
and **TTR uses universe polymorphism.** That is a named trigger, and it is a better predictor than
the ladder count in §1.

TTR is already load-bearing in the design docs rather than aspirational. D18's "ontology-as-types"
resolution cites it directly — *"Cooper's TTR is records-first, with a record type's labelled
fields-of-types matching an Eigenius class's required/recommended properties exactly"* — and the
`eigentt:TypeExpr` chain mirror already makes types first-class chain objects, which is the half of
TTR's types-as-objects stance Eigenius has. What it lacks is the other half: **quantifying over
those types at arbitrary level.**

Why that lands squarely on this issue. TTR stratifies — types of types, and predicates that range
over them. A predicate over record types is a statement at the level above whatever the record types
inhabit, and a *general* statement about them cannot be written at a fixed rung. Under
`Exp::Sort(usize)` each rung is a separate declaration, and every rung above `Set` is a bootstrap
edit plus a reseed — the cost #136/#187 paid once to move `spec_poly` from `Set` to `Type 1`. That
is precisely the cost universe polymorphism removes, and TTR is the first thing on the roadmap that
would pay it repeatedly rather than once.

**What this does not justify.** Building ahead of the TTR work would be speculative: the shape of
what Eigenius takes from Cooper is not settled, and taking *records-first* without taking
*stratification* is a coherent outcome that needs no polymorphism at all. §6's hold stands. What
changes is that the watch condition is now specific and expected rather than hypothetical, and the
signature to watch for is concrete:

> **the first TTR construct that needs a predicate ranging over types, or the second declaration
> that would sit above `Set`.** Either is the bump; the second is the one §1 measures for.

Whoever starts the TTR work should read §2 before choosing a sort representation for it — the level
algebra to mirror is already on the chain, and picking a different one there would be expensive to
unpick.

## 6. Recommendation

**Hold #188. Do not implement on the current evidence.** Record on the issue that the trigger is
measured and has not fired, so the next reader does not re-measure it.

Land it when *any* of:

- **the TTR work starts needing it** (§5a) — the likeliest path, and the one to watch;
- a second level bump is actually proposed — a rule or declaration that needs `Type 2`; **or**
- **#159/D74 makes it cheaper than the alternative.** Externalize-and-check has to translate EigenTT
  sorts into Lean's `Level`. With `Exp::Sort(usize)` that translation is a special case that works
  only for the monomorphic fragment; with an isomorphic `eigentt:Level` it is a fold. If the Lean
  path lands first, universe polymorphism may arrive as its prerequisite rather than on its own
  trigger — and that is a better reason to build it than the ladder is.

None of the three is in #188 today. The first is the likeliest and the one that would arrive with a
design of its own attached, so the useful discipline is to read §2 before that design fixes a sort
representation independently.

## 7. Exit gate, when it is picked up

- `eigentt:Level` declared, mirroring `lean:LeanLevel`'s five constructors; divergences justified.
- `simplify` / `subst_levels` / `leq` ported from nanoda `level.rs`, with its `IMax` cases, and the
  citations pinned at the current submodule revision.
- `Exp::Sort(Level)`; `conv.rs`'s integer cumulativity replaced by `leq`; `check_infer`'s
  `Sort(n) : Sort(n+1)` by `Sort(l) : Sort(Succ(l))`.
- Declarations carry `uparams`; the elaborator generalises free levels; **no new ESL syntax.**
- The decoder accepts the legacy integer form; verified against a served snapshot, not only the repo.
- One reseed, shared with **#213**.
- `cargo test --workspace`, `clippy -D warnings`, `fmt`, plus the WRN demo and both parse baselines
  on the reseeded snapshot.


---

## 8. Build log

**Slice 1 — the level algebra (`2026-08-22`).** `kernel/src/nbe/level.rs`: `Level` with nanoda's five
constructors, plus `of_nat` / `as_nat` as the numeral bridge every monomorphic site will use.
`simplify`, `combining`, `subst`, `leq` / `leq_core` / `leq_imax_by_cases` ported from
`references/nanoda_lib/src/level.rs` @ `6ae1f0c`, citations pinned. Pure module, no dependency on
`Exp`/`Val`, so it lands before anything else moves.

The load-bearing test is `agrees_with_integer_comparison_on_numerals`: on closed numeral levels
`Level::leq` must agree with `m <= n` exactly, since that is what the 942 monomorphic sort uses in the
tree rely on. 14 tests; kernel lib green at 1779.

Deliberate divergence from nanoda: no hash-consing (levels here are tiny and owned rather than
arena-interned), and `leq_core`'s unreachable arm returns `false` where nanoda `panic!`s — an
unexpected shape should not take down the commit gate.

**Slice 4 — the chain inductive and the codec (`2026-08-23`).** `eigentt:Level` declared in the
eigentt bootstrap layer with the five constructors; `TypeExpr`'s `Sort` argument retyped from
`core:integer` to it. `encode_level_json` / `decode_level_json` in the D47 codec, the decoder
accepting the pre-#188 bare integer permanently. Manifest moved on exactly one layer,
`eigentt-type-fragment`; **#213 rides that reseed**.

Two corrections this slice forced:

- **§4's "no chain source carries an encoded sort" measured the wrong thing.** It grepped committed
  JSON; the bootstrap ontologies carry `type_expr(...)` in ESL, which is encoded at build time.
  Retyping the ctor failed Rule 16 across `lexicon`, `closed-class` and others immediately. Those
  terms re-encode from source so nothing had to be rewritten — but the surface was not zero.
- **§3's "no ESL syntax" has a sharper edge than stated.** A polymorphic level cannot be printed as
  source at all. The printer now fails loudly rather than emitting `Sort(Succ(Zero))` ctors that
  reparse into nothing, and `a_polymorphic_level_refuses_to_print_rather_than_emitting_garbage`
  pins it. So the asymmetry is: reading the old numeral encoding is supported, writing it is not;
  and a polymorphic term is **chain-writable and source-unwritable**. Nothing on a chain today is
  affected, since every level is a numeral — but slice 5 introduces the first terms that cannot
  round-trip through ESL, and that is a constraint on it rather than a surprise to discover.

**Remaining slices**, in order:

2. `Exp::Sort(Level)` / `Val::Sort(Level)` — the mechanical change across ~509 non-test sites.
   `Level::of_nat` at construction, `as_nat()` guards where a site pattern-matched a numeral.
3. `conv.rs` cumulativity → `Level::leq`; `check_infer`'s `Sort(n) : Sort(n+1)` → `Sort(l.succ())`;
   `infer_dependent_sort`'s two-case Pi rule → `IMax`.
5. `uparams` on declarations; elaborator generalises free levels. No ESL syntax (§3).
6. Reseed with **#213** folded in; full gate plus the WRN demo and both parse baselines.
