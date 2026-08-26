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

## 3. Does the surface get levels? — YES, and it is Lean's syntax

> **REVISED `2026-08-23`.** This section said *"ESL should not grow level syntax in the first
> landing"*. That was wrong, and the error is worth naming: I argued from usage counts — 942 of the
> 944 sort uses are `Prop` or `Set`, so `Sort u` syntax "serves two sites". **That measures the
> monomorphic present, which is exactly what the feature exists to change.** It counted demand for a
> feature by counting uses of its absence; the same reasoning would have rejected indexed families
> because nothing used indices.
>
> The concrete consequence: `SortKind` is `Prop | Set | Type(usize)`, and nothing reachable from ESL
> can construct a `Level::Param` — the only producers are the D47 decoder and a test. So §7's
> *"declarations carry `uparams`; the elaborator generalises free levels"* has **nothing to
> generalise**: every authorable level is concrete, `uparams` would be empty on every declaration,
> and universe polymorphism would be implemented and unreachable from the language. §5a's TTR
> predicates ranging over types are *authored*, in source, and cannot be written without this.
>
> The tell was already in the tree: slice 4 added
> `a_polymorphic_level_refuses_to_print_rather_than_emitting_garbage`, a test asserting the printer
> *cannot* print a polymorphic level. I recorded that as a documented limitation. It was evidence the
> design was incomplete.

**Adopt Lean 4's surface syntax** ([reference](https://lean-lang.org/doc/reference/latest/The-Type-System/Universes/)),
because the numbering already agrees and the alternative is inventing a second spelling for a thing
the ecosystem has settled.

| Lean | Eigenius today | after |
|---|---|---|
| `Prop` = `Sort 0` | `Prop` → `Sort(0)` | unchanged |
| `Type` = `Type 0` = `Sort 1` | **`Set`** → `Sort(1)` | `Set` kept; `Sort 1` also writable |
| `Type u` = `Sort (u+1)` | `Type k` → `Sort(k+1)` | **already identical** |
| `Sort u` | *absent* | **added** |
| `u + n`, `max u v`, `imax u v` | *absent* | **added** |

`Type k`'s numbering matching Lean's is what makes this additive rather than a renumbering of the 11
existing `Type k` uses. Keyword collision checked: `Sort` occurs twice in the tree — once in a
comment in `logic.esl`, once as the English word in a lexicon `form` string — and `max` / `imax`
occur not at all.

**The one divergence to document rather than fix.** Eigenius's `Set` is Lean's `Type` / `Type 0`. In
Lean 4 `Set α` is a *library* type for sets, so a reader arriving from Lean will misread `Set` here.
Renaming it would touch 230 uses for no semantic gain; the correspondence belongs in the ESL guide's
type-theory primer instead.

**Binder form: `universe u;`, not `.{u}` — initially.** Lean offers both an explicit per-declaration
binder (`def id.{u} ...`) and a scoped `universe` command, and notes that *"Lean automatically
instantiates most level parameters"*. ESL is statement-oriented, and `.{` after a qualified name
(`data lexicon:Cat.{u}`) is the one form that needs real lexer work, since `.` is not an identifier
character. Take `universe u, v;` first and add explicit use-site instantiation only when inference
is insufficient — that is the same "add syntax when a consumer needs it" discipline, applied where
it is actually true rather than as a reason to add none.

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
2. ~~**Already-persisted terms must still decode.**~~ **Withdrawn `2026-08-23`.** There is no
   backwards-compatibility problem to solve: retyping the ctor moves the manifest, every persisted
   store then fails to resume with `ManifestDrift`, and **the reseed that answers it rewrites the
   chain from source**. No term in the old encoding can survive to be read by the new code, so a
   legacy decode arm is a compatibility layer for a state that cannot occur. One was written and
   removed.

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

> **⚠ SUPERSEDED `2026-08-25` by decision, not by evidence.** #188 was picked up as D76 Phase E2 while
> none of the three triggers below had fired. Recorded plainly so the next reader is not left
> reconciling a "hold" against a landed implementation: the recommendation was overridden, and the
> measurement that produced it still stands. See slice 5 in §8.
>
> The nearest thing to a trigger was structural rather than one of these three: D76's phases A–D
> removed every obstacle — one `Const`, the environment in the judgment, a slot for level arguments —
> so the cost side of this recommendation had changed even though the demand side had not.

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
- Declarations carry `uparams`; the elaborator generalises free levels. **~~no new ESL syntax~~** —
  **superseded by §3's `2026-08-23` revision**, which adopts Lean's surface syntax; this line was
  never updated and contradicted it.
- ~~The decoder accepts the legacy integer form~~ — **withdrawn by slice 4's build log**, which
  removed that arm as a bridge on top of a design already concluded: the manifest move makes the old
  stores unresumable, so the state it served cannot arise.
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
`core:integer` to it. `encode_level_json` / `decode_level_json` in the D47 codec. Manifest moved on exactly one layer,
`eigentt-type-fragment`; **#213 rides that reseed**.

Two corrections this slice forced:

- **§4's "no chain source carries an encoded sort" measured the wrong thing.** It grepped committed
  JSON; the bootstrap ontologies carry `type_expr(...)` in ESL, which is encoded at build time.
  Retyping the ctor failed Rule 16 across `lexicon`, `closed-class` and others immediately. Those
  terms re-encode from source so nothing had to be rewritten — but the surface was not zero. That
  same fact is why §4's second bullet was withdrawn: a reseed rewrites the chain, so nothing in the
  old encoding survives.
- **§3's "no ESL syntax" has a sharper edge than stated.** A polymorphic level cannot be printed as
  source at all. The printer now fails loudly rather than emitting `Sort(Succ(Zero))` ctors that
  reparse into nothing, and `a_polymorphic_level_refuses_to_print_rather_than_emitting_garbage`
  pins it. So the asymmetry is: reading the old numeral encoding is supported, writing it is not;
  and a polymorphic term is **chain-writable and source-unwritable**. Nothing on a chain today is
  affected, since every level is a numeral — but slice 5 introduces the first terms that cannot
  round-trip through ESL, and that is a constraint on it rather than a surprise to discover.
- **A legacy decode arm was written and removed** (`2026-08-23`, on review). The reasoning that
  produced it — served stores carry the old form and do not re-encode — ignored that the manifest
  move makes those stores unresumable, so the reseed is not optional and it rewrites the chain.
  This is the "bridge on top of a design already concluded" shape the project posture names; the
  giveaway was calling it "permanent rather than a migration window" while the state it served
  could not arise.

**Slice 5 — `uparams` and instantiation (`2026-08-25`).** Picked up as D76 Phase E2, **overriding §6's
"hold"** — a decision, recorded here so the contradiction is not left for a reader to resolve.

**The audit found most of slice 5 already landed.** §3's ESL half was done by 5c and the "remaining
slices" list above was stale: `SortKind` already has `Sort(LevelExpr)`, `LevelExpr` has
`Var`/`Add`/`Max`/`IMax`, and `lower_level` already produces `Level::Param`. Measured rather than
assumed — `data p:Box(A : Sort u) : Sort u { mk(A), }` **already compiled, persisted and validated
with zero errors**. The gap was entirely on the reference side: nothing bound `u` and nothing
instantiated it, which is precisely the "implemented and unreachable" state §3 warned about, one step
further along than §3 describes it.

So the slice reduced to four things:

- **`uparams` on `InductiveDecl`**, generalised at compile in **first-mention order**. `universe u;`
  is file-scoped, so what binds `u` on a declaration is that the declaration *uses* it. The order is
  the instantiation contract — a reference substitutes by position — and a `BTreeSet` would make it
  alphabetical, silently permuting a two-parameter declaration's arguments.
- **`core:universe_params`** on the wire, plus `instantiate_levels`, which **consumes** the
  parameters so a declaration cannot be instantiated twice.
- **`Exp::subst_levels`**, written with **no catch-all arm**: a future level-carrying variant must
  fail to compile there rather than silently keep an uninstantiated `Param`.
- **Levels as an OPTIONAL TRAILING `ConstRef` argument.** Emitting `[]` unconditionally is more
  uniform and was rejected: it rewrites every `ConstRef` on the chain, and every one is monomorphic.
  Keeping those bytes identical is what lets the reseed's parity check stay a *comparison* rather than
  a wholesale rewrite in which nothing could be noticed.

One error caught in review: `core:string_array` was invented as `universe_params`' datatype. It does
not exist — the declared array types are `resource_array` and `value_array`, and `value_array` carries
a conditional `then_requires: element_type`. Corrected to `value_array` + `element_type: string`.

The manifest moved on `core` (the property is declared there, because `result_sort` already needs
`core:Level`), so the pin was updated in the same commit as the ontology edit, per
`bootstrap_manifest_pinned`'s own instructions.

Pinned by `nbe::positivity::universe_polymorphism`: a declaration binds what it mentions, a reference
instantiates at its level argument (`Box.{0}` and `Box.{1}` differ — #188's residual, closed), and
level arguments round-trip through the wire while a monomorphic `ConstRef` keeps its single argument.

**Remaining slices**, in order:

2. `Exp::Sort(Level)` / `Val::Sort(Level)` — the mechanical change across ~509 non-test sites.
   `Level::of_nat` at construction, `as_nat()` guards where a site pattern-matched a numeral.
3. `conv.rs` cumulativity → `Level::leq`; `check_infer`'s `Sort(n) : Sort(n+1)` → `Sort(l.succ())`;
   `infer_dependent_sort`'s two-case Pi rule → `IMax`.
5. `uparams` on declarations + **the ESL level syntax of §3** — without it nothing can author a `Level::Param` and generalisation has nothing to generalise.
6. Reseed with **#213** folded in; full gate plus the WRN demo and both parse baselines.
