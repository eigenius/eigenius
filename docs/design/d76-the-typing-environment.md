# D76 — The typing environment

**Status: skeleton.** The design is not written. This file exists so the decisions already made, and
the obligations other work has parked here, have a home rather than living in another document's
appendix.

Implements **Seam A** of `docs/design/d75-fusing-eigentt-and-the-knowledge-graph.md`.

## 0. Scope

The chain of layers *is* the environment `Γ_env` of the judgment `Γ_env; Γ ⊢ e : T`. Today the layer
is an **effect capability** on `EvalCtx`, and the type theory has no environment on two of its three
surfaces (D75 §2):

| surface | environment today |
|---|---|
| `check` | partial — `CheckCtx.layer: Option<Arc<Layer>>` + `type_cache` + `CheckHooks::resolve_class`, classes only |
| `eval` | none |
| `eq_nf` / `subtype_of` | none — no context parameter at all |

## 1. Already decided (D75 §8)

Carried in so D76 starts from them rather than reopening them.

- **Q1 — CORRECTED `2026-08-24`.** As originally stated: *the environment belongs to `check` and
  `conv`, not `eval`*, on the evidence that `eval` produces opaque values for chain references
  (`Val::EigonClass(iri)`, `Neut::EigonAxiom(iri)`) and so builds neutrals and defers.

  **That is true for the opaque kinds and false for inductives.** `iota_reduce_impl`
  (`nbe/eval/iota.rs:41-69`) takes `decl: &Arc<InductiveDecl>` and reads **`decl.ctors`** to reduce —
  during *evaluation*. It needs no environment today only because the declaration is **inlined in the
  term**. De-inline it (§7, Phase E) and iota must resolve an IRI to reduce, so **`eval` needs the
  environment after all**.

  Q1 generalised from the opaque cases to every case. The 195-vs-54 site framing that made
  "environment in conv only" look cheap rests on that overgeneralisation.

  **The reference confirms the corrected reading**: nanoda's `Tc` holds the `Env` and performs both
  whnf and `def_eq` — evaluation and conversion share it. There is no version of this where the
  evaluator reduces recursors without an environment *and* declarations are not inlined.

  What survives: not a materialised projection — that is the full-chain-scan antipattern behind two
  prior OOMs — and the memo at `layer/mod.rs:678` is the right shape, a lazy cache keyed by
  `(LayerId, Iri)`.
- **Q1 — the `Option` goes.** "No layer access in pure check mode" is what let the three surfaces
  diverge.
- **Q2 — δ is decided per kind**, so no transparency annotations are needed: definitions
  **transparent** (pinned by D66 §4 and the proposition hash), axioms **opaque**, classes **opaque**
  (unfolding them would make class identity structural — and 749 of 894 shipped classes already
  resolve to the same `Val::One`, so opacity is the sole mechanism distinguishing most of the
  ontology), inductives deferred to Q3.
- **Q2 — §3.3 reconciles by fixing `check`, not `eq_nf`.** `find_sigma_field`'s unfolding must become
  a **projection rule** that consults the environment to type a field access without asserting the
  class equals its unfolding.
- **`Val` does not capture the environment.** The neutral carries the IRI; `Γ_env` lives in the
  conversion context, as nanoda's `Tc` holds `Env` while `Const` holds name + levels.

## 2. Inbound obligations — work parked here by other documents

**Each of these is blocked until D76 lands. This section is the reason this file exists.**

### 2.1 D78 §3.1 — the complete `Refine` subtyping rule

`Refine(R, S) <: Refine(R′, S′)` requires `⋀S ⊨ D` for every `D ∈ S′`. Entailment resolves class IRIs
against the layer chain, and `subtype_of(level, sub, super_)` / `eq_nf(level, v1, v2)`
(`nbe/check/conv.rs:290`, `:30`) take **no context at all**.

D78 Phase A shipped **set inclusion** (`S ⊇ S′`) instead — sound, because a constraint present in `S`
is trivially entailed by `⋀S`; **incomplete**, because it rejects the case where `S` entails `D`
without containing it. Strengthening it to the full rule is a **one-arm change** in
`subtype_of_inner` once conversion carries `Γ_env` — *provided D78 Phase B has landed the entailment
algorithm*. Phase B ships it with no consumer precisely to keep this obligation to one arm; if Phase B
is skipped, this becomes "write entailment, then wire it".

**Signal that this is done:** `entailment_beyond_set_inclusion_is_not_yet_decided`
(`kernel/src/nbe/readback.rs`) currently asserts the *rejection*. It must **flip** — it pins the
limitation, not the semantics.

### 2.2 Q4 / 4c — the recursor motive's codomain (#228)

The motive codomain is the constant `Exp::sort(2)` (`nbe/check/inductive.rs:589-594`), which caps
large elimination at **Set**: a motive returning `Sort(k)` has type `Sort(k+1)`, and only `k ∈ {0,1}`
pass. 4c makes it a level parameter, `I.rec.{u}` with motive `I(params) → Sort u`, which needs
`Const(iri, levels)`. `large_elim_admitted` keeps its exact meaning and call site — the two-way choice
between `sort(0)` and `sort(2)` becomes *`u` pinned to 0* vs *`u` free*.

**Signal:** ~~`large_elimination_is_capped_at_set_not_type_n` must flip~~ — **it could not, and the
marker was wrong.** It asserts `Level::of_nat(k+1).leq(&Level::of_nat(2))`, a fact about `leq` that
stays true however the recursor behaves: it *modelled* the constant instead of exercising it, so
removing the constant left it green. Replaced by
`check::inductive::tests::a_type_1_valued_motive_is_admitted`, which runs an actual recursor at each
level; the arithmetic is kept in `nbe/level.rs` as
`a_fixed_sort_2_codomain_would_cap_elimination_at_set`, renamed so it no longer claims to gate.

### 2.3 #188's residual — declaration-level `uparams` and level arguments

Affordable only after consolidation to one `Const`, which is what removes the self-reference stub and
gives a reference a slot for its level arguments. Without that, levels land on five variants — and on
none of them is there anywhere to put a level except inside the declaration, where identity-by-IRI
cannot see it, so `List.{0}` compares equal to `List.{1}` (Phase B's fourth audit note; pinned by
`nbe::positivity::level_slot`).

### 2.4 D77 — `InvalidatedSignature`

D20 names the missing merge cascade kind **type-checker driven** (`layer/merge/cascade.rs:24`). The
rule-driven half of D77 needs nothing from here (`validation/retroactive.rs` discharges the linear
form without touching the type checker), but the *designed* backstop is a type-level check and does.

## 2a. What D76 does **not** need from D78

D78 is not a prerequisite. The two meet at one trait method,
`CheckHooks::resolve_class(iri, layer) -> Val`, and are otherwise orthogonal:

- **The environment trait** (§3) does not need records.
- **δ mechanics** does not: Q2 makes classes **opaque**, so δ never unfolds a class and never sees
  what one resolves to.
- **Consolidation** (`EigonClass` → `Const`) changes the *reference form*; what the reference resolves
  to is `resolve_class`'s business and can be a Σ-chain or a `Record` either way. D75 §8a's
  caveat that Q3 "rests on the record model" is about the **argument** that produced Q3, not a code
  dependency.

So D76 and D78 can interleave. ~~The only ordering that matters is that D76 is a chain-format change
and D78 is not.~~ **Corrected `2026-08-24`: de-inlining is not a format change either** — the encoder
already writes `ConstRef(iri)` + an `App` spine (Phase E). Only *levels on the wire* are, and those
are #188's residual.

## 3. The census

Measured `2026-08-24`. These are the numbers the migration is priced against.

| variant | `Exp::` sites | `Val::` sites |
|---|---|---|
| `EigonClass` | 94 | 33 |
| `EigonAxiom` | 61 | 1 |
| `InductiveType` | 176 | 93 |
| `InductiveCtor` | 241 | 0 |
| `InductiveRec` | 34 | 1 |
| **total** | **606** | **128** |

And the number that prices §4: **54 call sites** of `eq_nf` / `subtype_of` / `subtype_of_with_hyps`.
That is what gains a parameter when conversion carries `Γ_env` — not the 195 `eval` sites, which Q1
established do not need one.

`Exp::Const` does not exist today; it is new.

## 4. The environment interface

### 4.1 What it is

`CheckHooks` (`nbe/check/hooks.rs:34`) is already the boundary and already has the right shape — it
is **stateless**, taking the layer per call so one shared instance serves every `CheckCtx`:

```rust
pub trait CheckHooks: Send + Sync {
    fn resolve_class(&self, iri: &Iri, layer: &Arc<Layer>) -> Result<Val, CheckError>;
    fn synthesize_chain_witness(&self, …, layer: Option<&Arc<Layer>>) -> …;
}
```

Two things change:

1. **A general lookup replaces the class-specific one.** `resolve_class` answers one question about
   one kind of global. What the three consumers need is *what kind is this IRI, and what does that
   kind's consumer need from it*:

   ```rust
   pub enum Global {
       /// Unfolds during conversion (D66 §4 pins this).
       Definition(Val),
       /// Nominal, never unfolds — but `check` needs the record to project.
       Constraint(Val),
       /// Postulated; nothing to unfold.
       Axiom,
       /// Carries the declaration `iota_reduce` needs to reduce.
       Inductive(Arc<InductiveDecl>),
       Absent,
   }
   ```

   **One variant per kind Q2 named, each carrying exactly what its consumer needs.** The split that
   matters is `Definition` vs `Constraint`: both resolve to a `Val`, and only the first may be
   unfolded in conversion. Collapsing them to a single `Transparent` — as an earlier draft of this
   section did — would make Q2's opacity a caller convention instead of a type distinction.

   `Inductive` exists because of §1's Q1 correction: `iota_reduce_impl` needs `decl.ctors`, so
   evaluation is a consumer of this lookup, not exempt from it.

2. **The `Option` goes.** `CheckCtx.layer: Option<Arc<Layer>>` and `resolve_class_cached`'s *"no layer
   access in pure check mode"* error are what let the three surfaces diverge (§0). An environment is
   a component of the judgment, so it is not optional — a caller with nothing to resolve against
   passes an **empty** environment, not `None`. Emptiness stays an implementation detail: a caller
   cannot ask "do I have a layer", only look up and get `Absent`.

3. **`EvalCtx` holds one too.** Not in the original draft — see §1's Q1 correction. `EvalCtx` already
   carries `layer: Option<Arc<Layer>>` in its `Effectful` arm as an *effect capability*; it becomes
   the same environment every other surface holds, for the same reason.

4. **`lookup` is not the whole interface — Phase C audit, `2026-08-24`.** Classifying every reader of
   `CheckCtx.layer` shows they do not all want a global lookup:

   | use | sites | wants |
   |---|---|---|
   | `layer.is_subclass_of(sub, sup)` | 3 | the **nominal subclass lattice** |
   | `resolve_class_cached` | 1 | a global lookup |
   | `resource_record(r, layer)` | 1 | property-type resolution |
   | context propagation, `EvalCtx` construction | 3 | mechanical |
   | witness synthesis | 1 | the layer itself |

   The subclass query is the substantive one. It is the **nominal** relation D78 §8a argues is
   load-bearing — and the counterpart to D78 Phase B's `entails`, which is the structural one; the
   two halves of Q10. It is a fact about declarations that the judgment consults, so it belongs on
   `Env`.

   So the interface is `lookup` **and** `is_subclass_of`, plus a `layer()` escape hatch for the
   consumers that genuinely need the layer (witness synthesis, `resource_record`) until they are
   migrated. **"The Option goes" holds for the judgment's view of globals and not yet for those** —
   an honest partial, not a completed removal.

5. **Consistency check on the 101 construction sites.** Worth doing because a wide mechanical change
   is where an inconsistency hides:

   - **Every layer-less construction is a test.** All 23 `CheckCtx::new` calls sit inside
     `#[cfg(test)]` modules and none exists outside `kernel/src/nbe/check/`; production uses
     `with_layer` (78 sites) throughout. **The `Option` serves test convenience, not a production
     need** — which makes removing it safer than the count suggests, since no production path
     depends on layer-less checking.
   - **Nothing asserts on the "no layer access in pure check mode" error.** It appears only at its
     definition. So converting a `None`-error into `Global::Absent` breaks no assertion.
   - **That error is swallowed at one of its two call sites.** `find_sigma_field` does
     `ctx.resolve_class_cached(iri).ok()?`, discarding a formatted string it allocated;
     `Construct` propagates. Under `lookup` returning `Absent`, `Construct` produces its own
     message, which can be more specific than the generic one it forwards today.

### 4.2 Who owns the memo

Not the trait. Two memos already exist with the right lifetime and different keys —
`RESOLVE_MEMO` (`layer/mod.rs:362`, keyed by every resolved IRI) and D78's `CLASS_FIELDS_MEMO`
(`validation/mod.rs`, keyed by class). Both are thread-locals with RAII scopes, sound because the
chain is immutable for the duration of a pass.

A third, keyed by `(LayerId, Iri) → Global`, belongs beside them rather than inside `CheckHooks`,
which must stay stateless to remain shareable. **Boundedness is the design constraint**, not speed:
D78's memo grows with the ontology's class count; this one would grow with every *resolved* IRI,
like `RESOLVE_MEMO`, which over a 9.4M chain is the cost that has to be justified rather than
assumed.

### 4.3 When to build it: Phase D, and only if measured

Not in Phase C, and the reason is that a shared memo buys nothing yet.

- **`check` is the only consumer today, and it already caches** — `type_cache`, per-`CheckCtx`, keyed
  by IRI. A second cache in front of it would serve one client that has one.
- **`conv` becomes a consumer in Phase D**, and it has no `CheckCtx`, so `type_cache` cannot help it.
  That is the first moment two surfaces would each pay.
- **`eval`/iota becomes one in Phase E** (§1, Q1 correction).

**And §5's fast path is the first-order control, not the memo.** Lazy-δ compares names before
resolving, so equal-name conversions — the overwhelming majority — never reach a lookup at all. The
memo only helps the *mismatch* path. Building it before measuring whether the fast path suffices
would be optimising the case that was already designed away.

**So: measure in Phase D, build if the measurement warrants it.** The measurement is only meaningful
once `conv` is a consumer, and §4.2's real question — does this thing stay bounded over a 9.4M
chain — cannot be answered without a representative workload.

**Deferring is cheap to reverse.** `Env::lookup` is the single entry point, so a memo goes behind one
function. The opposite — building it now — costs a cache with no second client and a boundedness
claim nothing can check.

## 5. δ mechanics

Q2 fixed the policy per kind. The mechanism is the open part, and it has one hard requirement:
**conversion must not resolve on the equal path.**

The lazy-δ shape, which nanoda and Lean both rely on:

```
conv(Const(a, ls), Const(b, ms)):
    if a == b && ls ≡ ms      → equal, WITHOUT resolving        ← the hot path
    else                       → look both up; unfold the transparent ones; recurse
```

Two consequences:

- **The common case costs one IRI comparison.** Same-name comparison is the overwhelming majority,
  and it never touches the environment. This is what makes the 54 threaded call sites affordable.
- **Only a mismatch pays.** Which is also where the answer is actually needed.

**Unfolding order when both sides are transparent** is the remaining choice. Lean unfolds the one with
greater definition height first; nanoda approximates. Neither is required for correctness — both
terminate — so this is a performance decision to make with a measurement, not ahead of one.

**What is already fixed and must not be re-opened:** definitions are transparent because
`decode_type` unfolds them and §3.4's proposition hash is taken over the decoded term (D66 §4);
classes are opaque because unfolding them makes class identity structural, and 749 of 894 shipped
classes have identical (empty) field sets.

**Correction `2026-08-24`: Eigenius already has per-declaration transparency.** An earlier draft of
this section said δ is decided per kind *"so Eigenius does not need nanoda/Lean's transparency
annotations at this stage"*. It has one. `definition_is_opaque`
(`program/eigentt_type_mirror.rs:871`) reads a boolean on a `Definition` resource, and an opaque
definition decodes to `Exp::EigonAxiom` — rigid, never unfolded, identity is the folded name
(D66, #95).

So the picture is: **kind decides the default, and definitions carry an override.** D76 does not have
to invent the annotation — it has to avoid *losing* it when lookup replaces decode as the place the
distinction is made. A `Global::Definition(_)` returned for a declaration flagged opaque would
silently make a rigid definition unfold.

## 6. The layer under construction — a `letrec` group

**Processing a layer is the fixpoint computation a `letrec` group needs.** The declarations are the
bindings, references among them are the dependency edges, and an intra-layer forward reference is
mutual recursion. That framing supplies the algorithm and corrects the option list an earlier draft
had.

### 6.1 What the reference actually does

nanoda's `Env` (`references/nanoda_lib/src/env.rs:218-228`) is an **ordered** `FxIndexMap` with a
`cutoff` field — *"used to mark the end of what should be the visible environment"* — and
`EnvLimit::ByIndex(idx)`. **Visibility is a prefix of the declaration sequence**: a declaration sees
its predecessors and not its successors. Plus exactly one escape hatch, `temp_declars`, *"used for
checking nested inductives"* — a temporary extension for the one case where a group's members must
see each other.

So Lean/nanoda is prefix-visibility with a narrow special case. **But the kernel gets its
declarations already ordered**: the frontend does the dependency analysis and emits them in
topological order. Eigenius has no such guarantee — an ESL layer's declarations arrive in *file*
order.

**That is the actual gap.** Not "what should be visible", but "who sorts them". Lean's answer lives
in a frontend Eigenius does not have.

### 6.2 The algorithm, which is already in the tree

Dependency analysis à la Haskell: build the reference graph over the layer's declarations, collapse
**strongly connected components**, topologically sort the SCCs, process in that order. Each SCC is a
`letrec` group; a singleton SCC with no self-edge is an ordinary declaration checked against fully
checked predecessors.

This is the same algorithm `Exp::record` already runs for field dependencies (D78 §1) — Kahn's
sort with a deterministic tie-break, cycles detected as the sort failing to place everything.
Different domain, same shape, and the tie-break matters for the same reason: it makes the order
**canonical**, so a layer's verdict does not depend on which valid order was chosen.

That dissolves the objection to the earlier option (a): order-dependence is not a hazard once the
order is derived rather than incidental.

### 6.2a The order must be derived — there is none to inherit

**The implementation obligation, stated concretely.** A loader cannot process a layer's declarations
"in the order they arrived", because that order does not survive loading:

- `LayerBuilder.resources` is a **`BTreeMap<Iri, Resource>`** (`layer/mod.rs:926`). An Eigon-JSON
  document is an array, and its order is discarded the moment resources are added.
- Everything downstream iterates `Layer::defined_iris()` — **IRI-lexicographic** order.

So the choice is not between "trust the input order" and "derive one". There is no input order left.
The only order available for free is alphabetical by IRI, which bears no relation to dependency.

**This makes a naive prefix-visibility implementation quietly wrong**, which is the hazard worth
recording. Extending the environment while iterating `defined_iris()` compiles, runs, and produces a
visibility rule determined by *IRI spelling*: `urn:x:Apple` would see nothing and `urn:x:Zebra` would
see everything, regardless of what either references. Renaming a declaration would change what
type-checks. Nothing would fail loudly.

So §6.2's SCC-and-topological-sort is not an optimisation or a nicety — **it is the only thing that
makes prefix visibility mean anything**. It has to be built before the visibility rule it orders, not
after.

For the same reason the tie-break must be deterministic (IRI, as `Exp::record` already does): with
`defined_iris()` gone as the ordering, something must make the result canonical, or a layer's verdict
depends on which valid topological order the sort happened to produce.

### 6.3 Where the `letrec` analogy stops

**Signature-then-body separation is an ML move that does not transfer cleanly.** `letrec` typing puts
every binding's type in `Γ` first and then checks bodies, which works because in ML a *type* never
depends on a *term*. In a dependent theory it can: a declaration's signature may mention an earlier
declaration's **value**, so "collect all signatures, then check all bodies" is not always possible.

This is why Lean does not have general mutual recursion at the kernel level, and why its `mutual`
blocks carry restrictions rather than being sugar for a `letrec`. nanoda's `temp_declars` is the
shape of the concession: a *narrow* temporary extension for one construct, not a general fixpoint.

### 6.4 What this leaves to decide

The structure is settled — SCC decomposition, topological order, canonical tie-break. What is not:

- **What a non-singleton SCC is allowed to be.** Following the reference: mutually-recursive
  inductives, and nothing else. Anything wider needs its own argument, because §6.3 says the general
  case is not available.
- **Whether Eigenius has any non-singleton SCCs today.** The cheap measurement: build the reference
  graph over each shipped layer and count SCCs of size > 1.

That measurement is sharper than the earlier draft's "how many declarations forward-reference": a
forward reference inside a DAG is fine — the sort handles it. **Only a cycle needs the special case.**

### 6.5 This is eigenius#20, arrived at from the other end

**A non-singleton SCC of inductive declarations *is* a mutual inductive block.** #20 ("Mutual
inductive types", deferred from D19 §16) asks for the surface syntax and the kernel machinery —
simultaneous positivity across the block, one recursor per type, cross-type iota. §6 asks what
happens when a layer's declaration graph has a cycle. Same question:

| | |
|---|---|
| **§6** | tells you **where** a mutual block is needed — SCC decomposition finds it, and nothing else in a layer can legitimately cycle (§6.3) |
| **#20** | is **what to do** when one is found |

Two things follow.

**#20 gains a trigger condition it has lacked since D19.** The issue defers on *"no immediate
life-science requirement demands mutual inductives"* — an assertion about need. §6.4's measurement
turns it into a decidable one: **a layer with an inductive SCC of size > 1 requires #20; a layer whose
declaration graph is a DAG does not.** If the shipped chain has no such SCC, #20 stays deferred *with
evidence* rather than by assumption, and the SCC count is the alarm that changes that.

**And the escape hatch found in §6.1 is #21's.** nanoda's `temp_declars` is documented as *"used for
checking nested inductives"*, and #20 records itself as a prerequisite for #21 (nested inductives).
So the reference's one concession to non-prefix visibility is precisely the #20 → #21 pair, which is
a second confirmation that the SCC case has exactly one legitimate occupant.

**§6 does not make #20 smaller. It tells you when you need it.** D19's costing — simultaneous
positivity, one recursor per type, cross-type iota — stands unchanged.

**And the failure mode today may be worse than an error.** `check_positivity(decl)` scans each
constructor for occurrences of **`decl` itself** (`nbe/positivity.rs:163-168`); a sibling inductive
is not `decl`. So if `A`'s constructor mentions `B` and `B`'s mentions `A`, each is checked in
isolation and the cross-type occurrence is not seen as recursive. The reference *resolves* — a layer
is built before it is validated, so `Layer::resolve` finds the sibling — so this does not fail
loudly.

**Tested `2026-08-24`: incompleteness, conditionally.** Three tests in `nbe/positivity.rs`
(`mutual_positivity_gap`):

- `a_self_negative_occurrence_is_rejected` — the control. `mk : (Bad → Bad) → Bad` is rejected, so
  the checker rejects things and the result below is not vacuous.
- `a_cross_type_negative_occurrence_is_not_caught` — the finding. Given `A ::= mkA (B → A)` and
  `B ::= mkB ((A → A) → B)`, with `A` to the **left of an arrow** in `B`'s constructor, **both pass**.
  `check_positivity(&b)` scans for occurrences of `B`; the offending occurrence is `A`.
- `no_eliminator_spans_the_pair` — why it is not exploitable: `InductiveDecl` carries only its own
  constructors, so a mutual block has no representation and no shared recursor. There is no
  cross-type recursion to carry a non-terminating term.

**So: incompleteness in a per-declaration checker, not a hole in the rule it implements** — there is
no mutual-block checker for it to be incomplete *for*. It becomes **unsoundness the moment mutual
blocks are admitted without simultaneous positivity**.

**That inverts the risk.** The hazard is not leaving #20 deferred; it is implementing #20's
*representation* — a mutual block the kernel accepts — before its *positivity checking*. **A
half-done #20 is worse than no #20**, and the sequencing constraint belongs on that issue.

**And the system does not prevent the case from arriving.** `a_mutual_pair_commits_clean_today`:

```
data t:A { mkA(t:B) }
data t:B { mkB(t:A) }
```

compiles from ESL to two resources and validates with **zero errors**. It then sits in the chain
uneliminable, since no shared recursor exists. With the positivity gap above, a *non-positive* pair
commits clean too. Nothing failed; the wrong thing succeeded.

**Phase A is the fix, at no extra cost.** The SCC pass (§6.2) computes exactly "is there an inductive
SCC of size > 1". Rejecting that, with a diagnostic naming #20, turns silent acceptance into a
tracked limitation — and the detector is being built anyway for ordering.

**This is not the Band-Aid CLAUDE.md warns about.** That guidance is about refusing input *that
should be expressible*, papering over a wrong AST or grammar. A mutual block *should* be expressible
once #20 is built; what is wrong today is accepting it and producing something uneliminable.
Fail-closed states the limitation instead of hiding it — and it is what makes "a half-done #20 is
worse than none" enforceable rather than advisory.

**So Phase A gains a second gate:** an inductive SCC of size > 1 is rejected, naming #20.

### 6.6 What #20 actually blocks: the kernel's own semantics

The shipped chain has no mutual inductives, but one thing plainly wants them, and it is not
hypothetical.

`Exp`, `Val` and `Neut` are the kernel's triad:

```
Val::Nt(Neut)                          Val → Neut
Neut holds Val in 3 variants           Neut → Val      ← the cycle
Val::Sig(_, Clos), Clos { body: Exp }  Val → Exp       (one way; Exp does not reference Val)
```

`{Val, Neut}` is a genuine 2-cycle; `Exp` sits outside it.

**That is why `eigentt:TypeExpr` exists and `eigentt:Val` does not.** `Exp` is self-recursive, so one
inductive expresses it — and one does, which is what the D47 codec encodes and decodes. `Val` and
`Neut` need a mutual block, so they are absent. Not an oversight: #20.

**Eigenius mirrors its syntax into the chain and cannot mirror its semantics.** A term is
chain-resident and inspectable; the value it evaluates to is not expressible. For a system whose core
ontology is self-describing — `core:Class is_a core:Class` — the self-description stops at `Exp`.

A sharper motivating case than the `Expr/Stmts/Stmt` sketch in #20's body: it is real, it is this
system's own machinery, and it says what the deferral *costs* rather than what it postpones.
as a question, not a claim.

## 7. The consolidation migration

~~This is the chain-format change.~~ **Corrected `2026-08-24` — see Phase E.** The encoder already
emits `ConstRef(iri)` plus an `App` spine, so de-inlining makes the in-memory form match the
persisted one and the bytes do not move. Only *levels on the wire* are a format change, and that is
#188's residual.

`InductiveType(decl, args)` → `App(Const(iri), args)`, and the inlined `Arc<InductiveDecl>` stops
being carried in the term. What that touches:

- **606 `Exp` construction sites and 128 `Val` sites** (§3).
- **Readback**, which must produce a `Const` rather than reconstructing a decl.
- **The D47 codec**, both arms — and therefore **every persisted term containing an inductive
  reference**. A reseed is required; unlike D78's, it is not optional.
- **`PartialEq` on `InductiveDecl`** — **corrected `2026-08-24`, see Phase B's fourth audit note.**
  It is by IRI today (`term.rs:365`), and it stays by IRI: nanoda compares declarations by name and
  levels by reference (`tc.rs:886`). What distinguishes `List.{0}` from `List.{1}` (D75 §3.2) is that
  `Const(iri, levels)` *has a slot for the level* and the fused node does not — not a change of
  equality.

**A pass that does not exist yet.** §6.2a: the declaration order prefix-visibility needs must be
*derived*, because `LayerBuilder` stores resources in a `BTreeMap<Iri, …>` and the input order is
gone. So the migration adds a dependency-ordering pass over each layer — it is not a refactor of
something already there.

**Order within the migration.** The stub is the thing to remove first: it is what forces by-IRI
equality, which is what makes levels unsound, which is what #188's residual is blocked on. Everything
else in this section is mechanical once it is gone.

## 8. Phases

**Each phase opens by reading the code it names, and correcting this document before writing any.**

**Compare against the code and against nanoda — not against pre-D50 design docs.** Anything older
than D50 has drifted, and the audit has repeatedly found the drift *in the docs* rather than in the
code: D19 never defines the stub its own subject depends on, and D48's status line conflates it with
the arity-skip. A pre-D50 doc is evidence of intent at the time, not of current behaviour.

Not a general principle — a response to this document's own error rate. §3–§8 were written in one pass
from a `grep -c` census, which measured the code's *shape* and not its *behaviour*. Four claims were
wrong, and each was caught only when a phase reached the code its section had named:

| claim | reality | found at |
|---|---|---|
| Q1: `eval` needs no environment | `iota_reduce_impl` reads `decl.ctors` to reduce | Phase B |
| §7: remove the stub first | the stub also serves cross-inductive references, so it needs the environment | Phase B |
| §5: no transparency annotations needed | `definition_is_opaque` is one, and is honoured | Phase C |
| Phase A: reuse `layer::supporting`'s walker | it skips `Value::Json`, so it would find no inductive edges | Phase A |

**Bounded, so this does not become the work.** Read the functions the phase names, correct what is
wrong, implement. Not an audit of the whole document, and not a re-derivation of settled decisions —
the outcomes in §2 and the ordering below are the goal, and the reading serves them.

---

Six phases, drawn where the **risk class changes**, as D78's were. Two things make this phasing
different from D78's and shape every boundary below:

- **Only E2 is a chain-format change** — corrected `2026-08-24`. De-inlining (E1) leaves the bytes
  untouched, because the encoder already writes `ConstRef(iri)` + an `App` spine. Levels on the wire
  (E2) are the format change, and are #188's residual by another name.
- **The first phase is a new pass, not an inert addition.** D78 opened with "constructs exist, nothing
  produces them". Here the ordering pass (§6.2a) must land *before* the visibility rule it orders,
  so Phase A is load-bearing from the moment it exists.

---

### Phase A — derive the declaration order

**Lands:** a dependency-ordering pass over a layer — reference graph, SCC collapse, topological sort,
IRI tie-break (§6.2). Nothing consumes the order yet.

**Why first.** §6.2a: `LayerBuilder` stores resources in a `BTreeMap<Iri, …>`, so input order is gone
and `defined_iris()` is alphabetical. A visibility rule built on that order would be determined by
IRI *spelling* and would not fail loudly. **The sort has to exist before anything depends on
ordering.**

**Gate:** the derived order respects every reference edge; it is **canonical under input
permutation** (shuffle the document, get the same order); cycles are reported, not silently
linearised; and **an inductive SCC of size > 1 is rejected with a diagnostic naming #20** (§6.5).
That last one closes a live hole: today such a pair commits clean and is uneliminable.

**Reusable:** `Exp::record`'s Kahn sort (D78 §1) is the same algorithm on a different graph.

**Status: complete** — `kernel/src/layer/declaration_order.rs`, six gate tests.

Two implementation findings worth carrying forward:

- **`layer::supporting`'s reference walker cannot be reused.** It skips `Value::Json`, documented as
  *"never carries typed-reference semantics here"* — true for its purpose. But an inductive's
  constructor argument types are D47-encoded JSON
  (`"type_name": {"ctor": "ConstRef", "args": ["urn:…"]}`), so reusing it would build a graph with
  **no inductive-to-inductive edges** and the mutual gate would never fire, on precisely the case it
  exists to catch. This module has its own walker; `a_d47_encoded_reference_is_an_edge` pins it.
- **The graph covers declarations, not instances** — classes, properties, inductives, data types.
  Running it over 9.4M lexicon entries would be the O(chain) antipattern again. Bounded by ontology
  size, like D78's class memo. Pinned by
  `instances_are_not_declarations_and_do_not_enter_the_graph`.

**Noticed, not chased:** if `layer::supporting` misses D47-encoded references, its supporting-layer
computation may be too shallow for a resource whose only reference to a lower layer sits inside an
encoded term. A possible latent defect in a different subsystem.

---

### Phase B — de-inline: `Const` + `App`, stub removed, the level slot created
*(absorbs what was Phase E1)*

**⚠ Reordered `2026-08-24`, before implementation.** Phase B was placed before Phase C on the
argument that the stub blocks everything downstream. Reading the stub's own documentation shows it
serves **two** uses, not one: *"self-references during ctor-type construction, **cross-inductive
argument-type references**"* (`term.rs:361-367`). Both are "name a declaration whose full form is not
available here" — which is what an environment is *for*. **The stub cannot be removed before the
environment exists.**

Combined with the Q1 correction in §1 — evaluation needs the environment too, because iota reads
`decl.ctors` — the order becomes **C → B**, and `eval` is inside the environment's scope rather than
outside it.

The rest of this phase's content stands; only its position moved.


**Lands:** a constructor's type refers to its inductive without the empty-`ctors` stub decl, and a
reference gains the slot a level argument goes in. **Superseded in part** — the fourth audit
correction below shows `PartialEq for InductiveDecl` (`term.rs:365`) should *stay* by-IRI.

**⚠ Audit `2026-08-24`: the stub is also a behaviour flag, and the flag is conflated.** Two dispatch
sites test for a stub by its *shape* rather than by anything that says "stub":

- `eval/mod.rs:637` — `if decl.indices.is_empty()` selects **pre-D48 behaviour**: all arguments
  treated as parameters, **no arity check**. The comment says so: *"Stubs are detected by
  `decl.indices` being empty … so the stub-Arc pattern keeps working."*
- `check/mod.rs:373` — `check_inductive_type_args` tolerates a stub because it *"carries no telescope
  to check against"*.

**A genuine non-indexed inductive has empty indices too.** `Nat` is indistinguishable from a stub by
that test, so the lenient path is taken for *every* non-indexed inductive, not only for stubs.
Arity checking on their type applications is skipped as backward compatibility, not as a decision.

So Phase B is wider than "remove the stub":

1. represent the self-reference without a stub — **now possible, `Env` exists** (Phase C);
2. give the two dispatch sites a discriminator that is not `indices.is_empty()`;
3. **decide whether non-indexed inductives get arity checking** — a behaviour change the stub has
   been hiding, and not mechanical;
4. only then do stubs disappear — as a consequence, not a step (fourth audit note).

**Sized: all 10 shipped inductives have no indices**, so the lenient path is taken by *every*
inductive application in the chain, not by an edge case. Turning arity checking on would newly check
all of them.

**So Phase B splits, one risk class each** — the same principle that draws the phase boundaries:

- **B1 — representation.** Self-reference without a stub, a discriminator that is not
  `indices.is_empty()`, stubs gone. **No verdict change**; gate is kernel tests.

  **⚠ B1 audit, continued.** The stub's shape is **inconsistent between its builder and its
  consumers**, and that has to be reconciled before it can be removed:

  | site | says a stub is |
  |---|---|
  | `ground.rs:424-431` (builder) | empty `params`, empty `ctors`, **real `indices`** — deliberate, #72 / D48, *"so ctor-internal self-references like `Vec(A, n)` decode against the same shape the check pass expects"* |
  | `check/mod.rs:373` (tolerance) | *"`params` and `indices` both empty"* — carries no telescope |
  | `eval/mod.rs:637` (dispatch) | `indices.is_empty()`, labelled stub-detection |

  Three descriptions, no two the same. And `check_inductive_type_args` zips
  `params.chain(indices)` against `args`, so for an indexed inductive a builder-shaped stub
  (`params` empty, `indices` real) would pair the **index's type against the first argument**, which
  is a parameter. Whether that misalignment is reachable is **untested** — the tolerance at `:373`
  describes a stub the builder does not produce.

  **There is no definition to reconcile them against.** D19 — the design doc *for inductive types* —
  never mentions the concept. D48 mentions it only in status lines, as a preserved artifact:
  *"stub-Arc pattern preserved (eval skips arity check when `decl.indices.is_empty()`)"* — which
  conflates the stub with the arity-skip in the same sentence, so the conflation in the code is
  inherited from the doc that shipped it. The three code sites are the only descriptions that exist,
  and they disagree because each wrote down the part it needed.

  **nanoda has the definition, and it is not a declaration.**
  `st.ind_consts.push(self.ctx.mk_const(ind.name, st.uparams))`
  (`references/nanoda_lib/src/inductive.rs:506`): a self-reference is a **`Const(name, levels)`** —
  an ordinary expression, the same form as any other reference — held in the check state for the
  duration of the declaration. There is no hollowed-out decl because none is needed.

  **So the stub exists because `Exp::InductiveType` has no way to say "the one being declared".** Its
  slot holds an `Arc<InductiveDecl>`, so a self-reference has to *be* some decl, and the least-wrong
  one is a copy with the unavailable parts left empty. The three-way disagreement follows: each site
  guessed differently about which parts those are.

  **B1 therefore aligns with nanoda: the self-reference becomes a `Const`.** That merges B1's
  construct with Phase E's — the same `Exp::Const(iri, levels)` serves both — so B1 is no longer a
  reconciliation with no fixed point but a well-defined change with a reference implementation.

  **⚠ And B1 must merge into E1, `2026-08-24`.** An earlier answer kept them apart on blast radius:
  B1 touches four sites, E1 touches 734. That was given before checking how a self-reference is
  *used*.

  It is **applied**: `Exp::InductiveType(self_ref, vec![Exp::Var("A")])` (`term.rs`, `build_list_decl`).
  So replacing the stub yields `App(Const(List), Var("A"))` — **replacing a stub entails de-fusing the
  application it heads**, because `Exp::InductiveType(stub, args)` has no `Const`-shaped equivalent
  that keeps the args fused.

  And B1's payoff needs *all* stubs gone: a hollow decl standing in for a full one is what hides the
  `ctors` iota needs and misleads the two `indices.is_empty()` dispatch sites. Three of the four are
  parametric, so B1 cannot stop at the non-parametric case.

  The alternative — `InductiveType` surviving for resolved references while self-references become
  `Const` + `App` — is worse than either: two forms for one thing, and every consumer handling both.
  That is the stub's own failure mode repeated.

  **So: B1 and E1 are one phase.** Its payoff is still #188 unblocked — via the level slot, per the
  fourth audit note — and its size is E1's.

  **And `ind_consts` is a `Vec`.** nanoda's positivity scans
  `has_ind_occ(ctor_type, st.ind_consts.as_ref())` — *all* the block's constants, which is exactly
  why it catches the cross-type occurrence `check_positivity(decl)` cannot (§6.5,
  `nbe::positivity::mutual_positivity_gap`). Aligning the representation makes the mutual-positivity
  fix a change of arity on one function rather than a new mechanism.
- **B2 — arity checking.** Turn on the check the stub was suppressing. **Verdict-affecting over the
  whole chain**, so it follows the #194/#92 protocol: instrument to log without rejecting, run the
  suites and the shipped ontologies, count, then enforce. A non-zero count is a finding about the
  chain, not a blocker for B1.

**⚠ Audit `2026-08-24`, before the sweep — three corrections.**

*1. The site count is 168, not 606.* `Exp::InductiveType` occurs 168 times — **85 production, 83
test** — across 34 files. The "606 `Exp` sites" §3 reports is the whole `Exp` reference surface, all
variants; it was never a count of this one. The production concentration is not in the kernel core:

| area | sites |
|---|---|
| `dcg/` (parser, rules, verbalize, chart) | 35 |
| `program/` (codec, ground, expr) | 13 |
| `nbe/` (check, eval, positivity, subst, readback, recursor, term) | 22 |
| `layer/`, `validation/`, `esl/`, `crates/` | 15 |

*2. `EvalCtx::Pure` has no environment, and the type checker uses it.* The environment sits in the
`Effectful` arm only:

```rust
pub enum EvalCtx {
    Pure,
    Effectful { layer: Option<Arc<Layer>>, hooks: Arc<dyn EffectHooks> },
}
```

`eval(exp, rho)` — 161 call sites, 67 of them in `check/` — hardcodes `Pure`. `eval`'s `Const` arm
resolves through `ctx.layer()`, so under `Pure` a de-fused `App(Const(List), A)` evaluates to a
**neutral** instead of a `Val::InductiveType`. Every inductive reference would stop being a type the
moment it is de-inlined.

The shape says why: **the environment is filed under the effect capability.** They are independent —
a pure evaluation needs `Γ_env` exactly as much as an effectful one; what `Effectful` adds is IO. So
`EvalCtx` carries `env: Env` in every mode and `hooks` becomes the optional part. This is the Q1
correction reaching its second surface: §1 established that *evaluation* consumes the environment,
and this is where the type says so.

*3. De-inlining moves declaration decoding from decode-time to eval-time, so the §4.3 memo is Phase
B's, not Phase D's.* `Env::lookup` on an inductive calls `resolve_class_type` →
`resolve_inductive_type`, which decodes params, indices, and every constructor type — on **every
call**. `RESOLVE_MEMO` does not cover it: that memo caches `Layer::resolve`, the resource lookup,
not the decode above it.

Today the decode happens **once**, in `resolve_const_ref`, and the result is inlined into the term —
which is precisely what "the declaration is inlined" buys. De-inlining moves it to once **per
occurrence per evaluation**. So the `(LayerId, Iri) → Global` memo §4.3 defers to Phase D on the
argument that Phase D adds the second consumer is superseded: Phase B *creates the need*, and
shipping the sweep without it trades a correctness fix for a decode in the evaluator's inner loop.
It lands here, shaped after `CLASS_FIELDS_MEMO` (D78 Phase D) — thread-local, RAII scope, keyed by
`LayerId`, `BTreeMap` at both levels.

**⚠ And a fourth correction, `2026-08-24`: B-c is the wrong change.** This phase has said throughout
that its payoff is `PartialEq for InductiveDecl` becoming structural, and that structural equality is
what unblocks #188's residual. Both halves are wrong.

**nanoda compares declarations by name.** `def_eq_const`
(`references/nanoda_lib/src/tc.rs:886`):

```rust
(Const { name: x_name, levels: x_levels, .. }, Const { name: y_name, levels: y_levels, .. }) =>
    x_name == y_name && self.ctx.eq_antisymm_many(x_levels, y_levels),
```

Name equality plus **level** equality. Declarations are never compared structurally — they are stored
in `Env` with their `uparams` uninstantiated, and instantiation happens at the *reference*. Adopting
structural comparison would make two lookups of one declaration compare unequal if any decode detail
differed between them.

**So what actually blocks #188 is the missing slot, not the equality.** `Exp::InductiveType(decl,
args)` has two slots — a declaration and value arguments — and a level argument is neither. The only
place `List.{0}` and `List.{1}` can differ is *inside* the declaration, and declaration identity is
the IRI, which does not see it. Pinned by `nbe::positivity::level_slot`:

| test | shows |
|---|---|
| `the_fused_form_has_nowhere_to_put_a_level_but_inside_the_declaration` | two decls differing only in `sort` compare **equal** |
| `the_de_fused_form_carries_the_level_on_the_reference_and_equality_sees_it` | `Const(iri, [0])` ≠ `Const(iri, [1])` |
| `identity_by_iri_is_the_reference_behaviour_not_a_workaround` | two lookups of one decl compare equal, as they must |

De-inlining *creates the slot*. That is why Phase B precedes E2, and the reason is structural rather
than about equality.

**By-IRI `PartialEq` therefore stays, and stops being a workaround.** Its docstring justifies itself
by the stub — *"a 'stub' `Arc<InductiveDecl>` carrying just the IRI can stand in for the full
declaration"* — which is backwards: identity-by-name is the correct discipline, and the stub is what
exploited it. Once no `Exp` carries a declaration, every decl in hand comes from `Env::lookup` and is
the full one, so by-IRI equality compares only complete declarations. The stub disappears as a
*consequence* of de-inlining rather than as a step in it.

**So Phase B sequences into three steps**, each compiling and tested:

| step | change | risk class |
|---|---|---|
| B-a | `EvalCtx` carries `Env`; `Global` memo | no de-inlining yet — verdict parity expected |
| B-b | delete `Exp::InductiveType`, sweep 168 sites | the sweep; compiler-enumerated |
| B-c | stubs gone as a consequence; by-IRI `PartialEq` re-justified | the payoff |

**⚠ Fifth correction, `2026-08-24`: four `Exp` variants carry a declaration, not one.**

| variant | sites | becomes |
|---|---|---|
| `Inductive(Arc<InductiveDecl>)` | 22 | `Const(iri, levels)` — it *is* the unapplied former |
| `InductiveType(Arc<InductiveDecl>, Vec<Exp>)` | 190 | `const_applied(iri, levels, args)` |
| `InductiveCtor(Arc<InductiveDecl>, Name, Vec<Exp>)` | 255 | `InductiveCtor(Iri, Name, Vec<Exp>)` |
| `InductiveRec { decl, .. }` | 34 | `InductiveRec { iri, .. }` |

Deleting only `InductiveType` would leave the gate unmet, because the codec hands the **same stub** to
the constructor arm: `resolve_inductive_decl_for_ctor`
(`program::eigentt_type_mirror`) short-circuits to `ctx.self_ref` when the `CtorApp` targets the
inductive being assembled. A stub in `InductiveCtor` is the same hollow declaration in a different
slot.

**`Inductive(d)` is a second spelling of `InductiveType(d, [])`, and the tree knows it.**
`positivity::rejects_disguised_inductive_negative_occurrence` exists because a negative occurrence
written in the `Exp::Inductive` form once evaded the checker; its comment calls the form *"non-canonical
spelling"*. One `Const` retires the spelling rather than continuing to test around it. Its one
production construction — `validation/rules/inductive_decl.rs:89`, building `Exp::Inductive(decl)` to
check a declaration — is not a reference at all but an *admission*, and becomes a direct call. That is
nanoda's split too: `check_inductive_declar` takes the declaration, not an expression.

**Constructors keep `(inductive IRI, ctor name)` rather than becoming declarations of their own.**
nanoda gives each constructor its own `Const`, but its constructors are environment entries with
names. Here they are not chain-resident: `InductiveCtorDecl { name, typ }` lives *inside* the
inductive's resource and has no IRI. Minting constructor IRIs is a chain-format change, so it belongs
with E2; swapping the declaration for the IRI removes the stub now without it. The wire already agrees
— `CtorApp(D, c)` plus an `App` spine is head-and-spine already.

The payoff there is a deletion: `resolve_inductive_decl_for_ctor` currently resolves the **entire**
target inductive — every constructor type decoded — for no purpose but checking that a name is in the
list. Carrying the IRI removes both the eager resolution and the self-reference short-circuit that
exists to stop it recursing.

**⚠ Sixth correction, `2026-08-24`, found by the sweep's first two sites: the environment did not know
the kernel's own declarations.** De-inlining `list_decl` and `option_decl` — the two `term.rs` stub
sites, the smallest change in the phase — broke `collective_np_coordination_parses`, a felicity test.

`core:List` is built in `nbe::term::list_decl` and is **not** a chain resource. `decode_type`'s
`ConstRef` arm has always special-cased it:

```rust
wk::LIST => return Ok(Exp::InductiveType(crate::nbe::term::list_decl(), Vec::new())),
```

`Env::lookup` did not. While `list_decl` inlined the declaration into its own constructor types the
gap was invisible; written as `Const(core:List)` the name resolved to nothing and evaluated to a
neutral, so the constructor types stopped being types and felicity filtering rejected the parse.

This is exactly the divergence §2 names — one surface knowing something another does not — and the
fix belongs on `Env`: `Env::intrinsic` answers for the kernel-provided declarations, in **every**
environment including the empty one. They are not chain content and cannot be shadowed, so "knows
nothing" means nothing *about the chain*.

**`core:Option` is deliberately not intrinsic.** It exists twice — as a chain resource *and* as
`nbe::term::option_decl` — and `List` does not. Answering it from the kernel's copy would hide any
disagreement between the two; `the_chain_and_the_kernel_agree_about_option` asserts they match
instead. They do today.

**The general lesson for the rest of the sweep:** inlining hid every place a name could not be
resolved, because the answer travelled inside the term. Each de-inlined site is a new demand on the
environment, and a missing answer degrades to a neutral rather than an error — silent, and visible
only as a downstream verdict change. The felicity suites are load-bearing here, not incidental.

**So B-b splits in two commits**, each compiler-enumerated: `Inductive` + `InductiveType` → `Const`
(212 sites), then `InductiveCtor` + `InductiveRec` → IRI (289 sites).

**Status of B-b1 (`Inductive` + `InductiveType` → `Const`): complete.** 1761 kernel tests, workspace
and clippy clean. Four defects surfaced, none of them test plumbing:

| defect | why it was silent |
|---|---|
| `eval`'s `Const` arm rebuilt an `Env` from `ctx.layer()` instead of using the context's own | discards the intrinsics and any declaration in progress; an unresolved name degrades to a *neutral*, so `Nat.zero` inferred as `Nt(Const(Nat))` rather than its type |
| the new `check_infer` arm sat **after** `App` | `App` infers its head and demands a Π of it, and a type former's type is a *sort* — every applied former failed `expected Pi type, got Sort(Zero)`, and the core ontology stopped loading |
| de-fusing **lost** the indexed-arity check | fused, eval got the whole argument vector and could count it; de-fused, arguments arrive one at a time and eval never knows more are not coming |
| `Env` had no place for a declaration in progress | the stub *was* that place |

The third is the one to note: a representation change silently dropped a check. It moved to
`check_inductive_type_args`, unchanged in what it accepts and scoped to indexed declarations — the
lenient path for un-indexed ones is still B2's, under the measurement protocol.

The fourth is nanoda's `temp_declars` (`references/nanoda_lib/src/env.rs:221`, consulted ahead of the
committed declarations at `:259`). A declaration is in scope for its own constructor types before it
is committed anywhere, which is exactly what the stub was faking; `Env::declaring` is the honest
version.

**Two eager resolutions deleted.** `resolve_inductive_decl_for_ctor` and `inductive_stub_for` each
decoded an entire inductive — every constructor type — to fill a slot that no longer exists; both are
existence predicates now (`names_an_inductive`, `inductive_iri`). And `resolve_const_ref`'s
self-reference short-circuit is gone: it existed only because resolving a name produced a
*declaration*, so a constructor body mentioning its own inductive recursed unboundedly. A name
resolves to `Const`, so there is nothing to recurse into.

**What `check_infer`'s new arm is not.** It reads `decl.sort` from `Γ_env` and hands back a sort,
leaving `check_inductive_type_args` to check the arguments. nanoda has no such arm: a type former is a
`Const` whose type is the Π-telescope `Π(params)(indices). Sort l`, so `infer_app` walks it and the
**ordinary application rule** checks the arguments. Adopting that deletes `check_inductive_type_args`
outright — and it is B2's change, because the ordinary rule checks arity where the fused node's rule
did not. The arm is the minimal shape that keeps Phase B verdict-neutral.

**Status of B-b2 (`InductiveCtor` + `InductiveRec` → IRI): complete, and the phase gate is met.** No
`Exp` variant carries an `Arc<InductiveDecl>`; `self_ref` occurs zero times in `program::ground` and
`program::eigentt_type_mirror`. 1761 kernel tests, workspace and clippy clean.

**The site split is the reverse of B-b1's: 179 production, 77 test**, concentrated in the parser's
category machinery (`constructions.rs` 55, `category.rs` 30, `combinators.rs` 27), because
`Exp::InductiveCtor` is how the DCG builds categories — `cat_np(T, num)` and `cat_s(mood, fin)` are
constructor applications of `lexicon:Cat` / `Mood` / `Num`. The *edit* was simpler than B-b1's even so:
the variant keeps its shape and only its first field's type changes, so there is no spine to walk and
a match that ignores the declaration does not change at all.

**`Val::InductiveVal` had to follow, and the measurement decided it.** Making the *term* name its
inductive made *evaluating* a constructor application a lookup — and readback applies closures with
**no environment, deliberately**, because it must not unfold. Constructor applications inside closure
bodies started failing.

| way out | cost |
|---|---|
| thread an environment through readback | **114** `readback_val` call sites |
| let `Val::InductiveVal` carry the IRI as well | **93** sites, only **4** reading the declaration |

The second is also the better end state: value and term name their inductive the same way, evaluation
becomes total again — no lookup, so no failure mode — and readback stays environment-free.
`iota_reduce` is unaffected; it takes the declaration from the *recursor*, which resolves it where the
environment is in hand.

**A lossy fallback caught before it shipped.** When the environment could not resolve a constructor's
inductive, an intermediate version of the eval arm fell back to `Const(iri)` applied to the arguments
— which **drops the constructor name**, making `succ x` and `zero x` the same value. `Neut` has no
constructor form, so there was nothing to fall back *to*. Moot once the value carries the IRI, but it
is the shape of mistake that passes tests.

**`Clos::apply` and `Val::app` were the `EvalCtx::Pure` conflation one level down.** Both default to
an evaluation with no environment, and the checker applies Π-closures constantly. `CheckCtx::apply` /
`CheckCtx::app` carry it now; readback's uses stay environment-free, which is correct there and is
now said rather than assumed.

**Deleted:** `resolve_inductive_decl_for_ctor` (decoded the whole target inductive to validate a name
the checker validates anyway), `DecodeCtx::self_ref` and `decode_type_with_self_ref` (the recursion
guard that existed only because resolving a name produced a declaration), `category::resolve_inductive`
(superseded by `inductive_iri`), and the stub in `resolve_inductive_type` itself.

**B-b deletes the variant rather than deprecating it.** Leaving it in place while migrating callers
would make every un-migrated match arm a *silent* non-match — the failure mode that a spine walker
looks like a plain `App`. Deleting it turns all 168 into compile errors, so the sweep is enumerated
by the compiler instead of by grep.

B1 is the part #188 is blocked on — **corrected below**: it is the missing level slot on the
reference, not `PartialEq`. B2 can follow at its own pace.

**Why second, and why not later.** §7 put it as: the stub forces by-IRI equality, which makes
`List.{0}` compare equal to `List.{1}`. The fourth correction below sharpens this — by-IRI equality is
right, and what makes the two compare equal is that the fused node has no slot for a level, so the
only place to put one is inside the declaration where equality cannot see it. Everything downstream in the migration is
mechanical once it is gone; nothing downstream is safe while it remains.

**Gate:** no `Exp` carries an `Arc<InductiveDecl>`, so every declaration in hand comes from
`Env::lookup`; the three stub sites (`term.rs:447`, `check/mod.rs:339`, `eval/mod.rs:604`) are gone;
`nbe::positivity::level_slot` holds; existing inductive tests unchanged.

---

### Phase C — the environment interface

**Lands:** `lookup(iri) → Transparent(Val) | Opaque | Absent` replacing `CheckHooks::resolve_class`;
`CheckCtx.layer: Option<…>` becomes a non-optional environment; the `(LayerId, Iri) → Global` memo,
sited beside `RESOLVE_MEMO` and `CLASS_FIELDS_MEMO` rather than inside the trait (§4.2).

**Behaviour change:** none intended — `check` already resolves classes; this changes *how* it asks.

**Gate:** verdict parity on the shipped ontologies. **Memo boundedness measured, not assumed** —
unlike D78's class-keyed memo this one grows with every resolved IRI, which §4.2 flags as the cost
needing justification.

**Status: complete.** `CheckCtx.layer: Option<Arc<Layer>>` is now `env: Env`, and
`resolve_class_cached` goes through `Env::lookup`. 1745 kernel tests, workspace and clippy clean.

Routing through `lookup` surfaced a conflation `CheckHooks::resolve_class` could not express: it
returned a bare `Val` for any IRI, so "this class did not resolve" and "this is not a class" were the
same outcome. The new path matches on `Global` and says which —
*"'urn:x:Foo' is not a class — the environment classifies it as an inductive"*.

**Still on `Env::layer()`, deliberately:** witness synthesis, `resource_record`, `EigonAxiom`'s
`axiom_env`, and `EvalCtx` construction. Those want the *layer*, not the environment, so the `Option`
survives for them. `EvalCtx`'s belongs with the Q1 correction, in Phase D or E.

**The `(LayerId, Iri) → Global` memo is deliberately not built here** — §4.3. `check` is its only
consumer today and already caches via `type_cache`; `conv` becomes the second in Phase D, which is
also the first point its boundedness can be measured against a real workload.

---

### Phase D — δ in conversion

**Lands:** `eq_nf` / `subtype_of` / `subtype_of_with_hyps` gain the environment — **54 call sites**
(§3) — with §5's lazy-δ: equal names and levels compare equal *without resolving*.

**This is the phase D78 has been waiting on.** D76 §2.1's parked obligation discharges here, and
`entailment_beyond_set_inclusion_is_not_yet_decided` must **flip**.

**Also decides §4.3:** measure whether the lazy-δ fast path leaves enough lookups to warrant the
shared memo, and build it only if so.

**Status: complete `2026-08-25`.** The parked test flips —
`entailment_beyond_set_inclusion_is_now_decided` — and `subtype_of_inner`'s `Refine` arm implements
D78 §3's full rule. `a_class_and_its_own_unfolding_are_not_definitionally_equal` still holds, but its
stated reason (*"`eq_nf` takes no context at all"*) was corrected: the behaviour is now a *choice* Q2
requires, not something the absence of a layer decided.

**Gate:** the parked test flips; `a_class_and_its_own_unfolding_are_not_definitionally_equal` is
**re-examined, not assumed** — Q2 says the reconciliation is to stop `check` treating its unfolding as
definitional equality, so that test's *class* case should still hold while the mechanism beneath it
changes. Plus: no resolve on the equal path, asserted by instrumentation rather than by reading.

**⚠ Audit `2026-08-25`, before implementation — the phase is narrower than this says.**

*1. `subtype_of_with_hyps` does not exist.* It is `subtype_of_deferring_indices`
(`check/conv.rs:316`). Three doc comments still name the old one (`conv.rs:310`, `conv.rs:776`,
`inductive.rs:505`). The threaded surface is **`eq_nf` (30 sites) + `subtype_of` (26) = 56**, not 54 —
D78 added two. `def_eq_at_type` and `is_propositional_in_ctx` already take a `CheckCtx`, so they are
not part of the thread.

*2. **δ never reaches conversion, so §5's lazy path has no case to optimize.*** §5 states the
requirement as *"conversion must not resolve on the equal path"* and gives nanoda's shape:
`conv(Const(a, ls), Const(b, ms))` compares names first and unfolds only on mismatch. That shape
presumes conversion is where δ happens. In Eigenius it is not:

| where a definition unfolds | since |
|---|---|
| `decode_type`'s `ConstRef` arm returns the decoded **body** for a transparent definition | D66 |
| `eval`'s `Const` arm returns `Global::Definition(v)` — the **value** | Phase B |

Both are **eager**. By the time conversion sees a value, a transparent definition is already its body;
there is no folded definition name for a fast path to compare. What conversion *does* compare by name
— `EigonClass`, `EigonAxiom`, `InductiveType`'s declaration, `Neut::Const` — is exactly the set that
must **not** unfold (Q2: classes opaque because unfolding makes class identity structural; axioms and
opaque definitions rigid).

So the requirement is already met, and trivially: conversion resolves nothing at all today. The lazy-δ
mechanism is **not built**, because there is no folded-definition case for it to serve. Building it
would be a fast path for a state that cannot arise.

*3. What Phase D therefore is.* One arm, plus the threading that arm needs:

- thread `&Env` through `eq_nf` / `subtype_of` / `subtype_of_inner` / `subtype_of_deferring_indices`;
- replace `subtype_of_inner`'s `Refine` rule — set inclusion `S ⊇ S′` — with **inclusion first, then
  `conjunction_entails`**. Inclusion stays the fast path, so the environment is consulted only where
  the conservative rule was about to *reject*. That preserves §5's "no resolve on the equal path"
  by construction rather than by a mechanism.

*3a. Correction found while implementing: `eq_nf` does not need the environment, and neither does
`unify`.* Both were threaded per this phase's wording; clippy reported the parameter as unread in one
and recursion-only in the other.

The reason is that **entailment is a subtyping notion**. `⋀S ⊨ D` decides whether one constraint set
is at least as strong as another — the asymmetric question. For *equality* the sets must match, and
relaxing that to mutual entailment would make refinement identity structural, which is the same
objection Q2 raises against unfolding classes. `unify` follows: it falls back to `eq_nf` and otherwise
recurses, never reaching the `Refine` arm.

So the threaded surface is **`subtype_of` (26 sites), `subtype_of_deferring_indices` and
`subtype_of_inner`** — not the 54-or-56 the phase implies. A parameter that goes unread is a parameter
that lies about what the function needs, so it was removed rather than underscored.

*3b. A consistency defect the audit surfaced: two neutral forms for one rigid name.* `Neut::EigonAxiom`
and `Neut::Const` both mean "a name that does not unfold, compared by IRI", and they read back
differently — `EigonAxiom(x)` versus `Const(x, [])` — so they do not compare equal. Which one a term
carried depended on how it arrived: `resolve_const_ref` emits `EigonAxiom` for an axiom or an opaque
definition, while `eval`'s `Const` arm mapped `Global::Axiom` to `Neut::Const`. Eval now produces the
existing form. Two forms for one thing is the stub's own failure mode
(`a_transparent_definition_never_reaches_conversion_folded` pins both halves).

*4. §4.3's memo decision follows from that shape.* Entailment is reached only when set inclusion
fails, i.e. only where the answer changes a rejection into an acceptance. Lookups on that path are
rare by construction, so Phase D does not supply the second heavy consumer §4.3 was waiting for. The
`(LayerId, Iri) → Global` memo built in Phase B keeps its one installed scope; see the
`2026-08-25` reseed note for why its boundedness is still unmeasured.

---

### Phase E2 — levels on the wire

**Lands:** `Exp::Const(iri, levels)`; `EigonAxiom` and the inductive trio fold into it (§8a keeps
`EigonClass` out);
`InductiveType(decl, args)` → `App(Const(iri), args)`. **606 `Exp` sites, 128 `Val` sites** (§3), plus
readback and both D47 codec arms.

**⚠ CORRECTED `2026-08-24`: de-inlining is *not* a chain-format change.** This phase and §7 both said
every persisted term containing an inductive reference changes shape and a reseed is mandatory. The
encoder says otherwise:

```rust
Exp::InductiveType(decl, args) => {
    let mut current = ctor("ConstRef", vec![json!(decl.iri.as_str())]);
    for arg in args { current = ctor("App", vec![current, encode_type_json(arg)?]); }
```

**The wire form is already `App(App(ConstRef(iri), a₁), a₂)` — `Const` plus an `App` spine.** The
`Arc<InductiveDecl>` never reaches the chain; only `decl.iri` does, and the encoder already de-fuses
the application. `InductiveCtor` likewise emits `CtorApp(iri, ctor_name)` + spine.

So de-inlining makes the **in-memory form match the form already persisted**. Encode becomes close to
an identity, decode stops reconstructing a decl — and **the bytes on the chain do not change. No
reseed.**

**What *is* a format change is levels.** `Const(iri, levels)` with a non-empty level list is new on
the wire, and that is #188's residual — not part of de-inlining. So E splits:

- **E1 — de-inline.** In-memory only, no wire change, no reseed.
- **E2 — levels on the wire.** Chain-format change, reseed, and #188's residual by another name.

**Gate:** `every_shipped_ontology_document_round_trips`; a full `--umls-all` reseed at
**9,439,633 resources, 0 errors**; the WRN demo at **56 Holds / 0 Fails**; the parse gate compared by
`--replay`, never live (see `docs/notes/parse-gate-drift-2026-08-24.md`).

### 8a. `EigonClass` or `Const` — the Phase E decision

D75 moved on this three times: §3.1 folded it in, Q2 refuted transparent folding, option C separated
it, and §6.3's synthesis folded it back. The synthesis argument was: a class is a resource, a
reference to a resource is `Const(iri)`, so `EigonClass` is not a distinct kind of *reference*.

That argument is sound and **not decisive**, because the question is not whether a class reference
*could* be a `Const` — it is whether the kind distinction should be **syntactic or looked-up**.

**For folding in**

1. *"Everything is a Resource"* — the founding principle. `Val::EigonClass(Iri)` sitting beside
   `Val::ResourceVal(Box<Resource>)` already violates it (D75 §6.3).
2. **Reference form and resolution shape are orthogonal**, established empirically in D78 Phase A:
   they meet at one trait method, so `Const(iri)` can resolve to a constraint without the reference
   being special.
3. **One lookup path.** `lookup → Transparent | Opaque | Absent` covers every global; a separate
   variant means conversion has two paths for "a named chain thing".
4. 127 sites stop being a distinct case.

**Against — keep it distinct**

1. **A class denotes a predicate, not a type** (§6.1). `Const` means *a name for a declaration that
   has a type and may unfold to a value*. A class has no value to unfold to — it denotes a condition
   over records. Folding makes `Exp` misdescribe its own domain.

   **D78 has already committed to this distinction in shipped code.** `Refine(Box<Val>,
   BTreeSet<Iri>)` (`nbe/val.rs:62`) holds the record type as a **`Val`** and the constraints as
   **`Iri`s**. The type is inhabited; the constraint names are not values at all. A class reference is
   a name in that second space, and `EigonClass` is what names it.

   *This is the load-bearing argument.* The two below are consequences worth having, not reasons.

2. **Opacity becomes unforgeable rather than maintained.** Q2 makes classes δ-opaque, and 749 of 894
   shipped classes have identical (empty) field sets, so opacity is the only thing distinguishing most
   of the ontology (D78 §1.2). `Exp::EigonClass` makes that structural; `Const` makes it a rule
   someone must keep enforcing.

   **But this is a *guard*, not a reason.** CLAUDE.md: a fix that guards against bad behaviour rather
   than eliminating it is a Band-Aid. The test is whether the justification survives changed
   circumstances — build robust transparency annotations and this reason evaporates, and the next
   person folds `EigonClass` in. Argument 1 does not evaporate.

3. **Conversion can decide inequality without the environment.** Two `EigonClass` with different IRIs
   are unequal immediately; two `Const` must be resolved to learn whether either unfolds. §5's fast
   path covers the equal case — this is the unequal case, and the one that pays. A performance
   consequence of 1, not an independent reason.

4. **It buys little for #188.** Constraints are level-generic (§6.1), so a class carries no levels.
   Folding gets uniformity, not universe-polymorphism progress.

**The middle option, and why not.** `Const(iri, kind, levels)` with the kind tagged would give one
variant *and* syntactic opacity. But a tag is a fact about the environment cached in the term, and a
redefinition can change an IRI's kind — so it needs the same revalidation discipline as D75 §3.5's
merge hazard. That is the inline-the-environment antipattern in miniature, and this document exists
to remove it.

**Recommendation: keep `EigonClass` distinct**, on argument 1 — the categorical one. D75's thesis is
that the *environment* belongs in the judgment; it does not follow that a *kind distinction* the
theory makes should be resolved rather than written. And D78's `Refine` already writes it: record
types are `Val`s, constraints are IRIs.

**This corrects §6.3's conclusion**, which folded `EigonClass` in without weighing what the fold costs
in enforceability. §6.3's Q3 answer — five variants to one — becomes **five to two**: `Const` for
declarations that have types, `EigonClass` for constraints that do not.

---

### Phase F — Q4/4c, the recursor motive

**Lands:** the motive codomain becomes a level parameter, `I.rec.{u}`, replacing the `Sort(2)`
constant whose ceiling is Set (§2.2).

**Last, and gated on Phase E** — it needs `Const(iri, levels)` to exist.

**Gate:** a recursor with a `Type 1`-valued motive type-checks
(`a_type_1_valued_motive_is_admitted`); `large_elim_admitted` keeps its exact meaning and call site,
the two-way choice becoming *`u` pinned to 0* vs *`u` free*. **The originally-named signal could not
serve** — see §2.2.

**Status: complete `2026-08-25`.**

**⚠ Audit `2026-08-25`, before implementation — two corrections.**

*1. The stated dependency is wrong.* *"Last, and gated on Phase E — it needs `Const(iri, levels)` to
exist."* The slot has existed since Phase B, and the codomain never reaches the wire: `codomain_sort`
is a **local `Exp`** built at `check/inductive.rs:611`, used to construct the motive's expected type
and discarded. Nothing persists it. F was never gated on E2 — E2 landing first was fine, but not for
this reason.

*2. `I.rec.{u}` as written is not implementable, and does not need to be.* Making the codomain
`Sort(Param(u))` for a free `u` requires **solving** for `u` from the motive, because
`check(motive, … → Sort(u))` compares `Sort(k+1) ≤ Sort(u)`, which `Level::leq` cannot discharge for
an unbound parameter — every motive would be rejected. Solving needs **level metavariables**
(`Level::Meta` + unification), which Eigenius does not have: `nbe::unify` solves *value* metas, and
`Level` has no meta constructor. Lean does this in the **elaborator**; nanoda is a kernel and receives
the level already solved inside the `Const`.

Nor can the recursor carry the level as an argument today: `Exp::InductiveRec { iri, motive, minors,
major }` has no level slot, and adding one would need surface syntax to author it — N3 §3 deferred
use-site instantiation (`.{u}`) precisely until a consumer needed it.

**But `u` never has to be solved, because the motive determines it.** The codomain level is not an
unknown to unify — it is a *fact about the motive*, readable by applying the motive to fresh generics
(one per index, one for the major) and asking what sort the result inhabits. That is
`ensure_infers_as_sort`, which already exists and already returns the level.

So Phase F is: **derive the codomain rather than fix it**.

| motive | derived codomain | today (`Sort(2)`) |
|---|---|---|
| `λ_. Prop` | `Sort(1)` | accepted |
| `λ_. Set` | `Sort(2)` | accepted |
| `λ_. Type 1` | `Sort(3)` | **rejected** — the ceiling |
| `λx. Nat` | `Sort(1)` | accepted |

The first, second and fourth rows are unchanged, so this is backwards-compatible by construction; the
third is the ceiling lifting. **A bare `λ_. Sort(1)` is not inferable as a whole** — which is why the
motive is *checked* and not inferred, and why the derivation applies it to generics first and infers
only the body.

The Prop gate is unchanged in meaning and becomes a constraint on the derived level: for a `Prop`
inductive failing singleton-elim, the derived level must be `0`. `large_elim_admitted` keeps its
signature and its one call site, exactly as this phase requires.

---

### Ordering

```
A ──▶ C ──▶ B ─────────▶ D ──────────▶ E2 ─────────▶ F
done  done  de-inline    δ in conv     levels/wire   4c
            Const+App    │             │
            level slot   unblocks      reseed
            on the ref   D78 §3.1      (#188 residual)
```

**B absorbs E1.** Replacing a stub entails de-fusing the application it heads (see Phase B), and the
level slot only exists on a de-fused reference — so the two are one change. What remains as **E2** is
levels on the wire, which is the only part that moves the chain format, and is #188's residual under
another name.

**D follows B**, not the reverse: §5's fast path is `conv(Const(a, ls), Const(b, ms))`, which
presumes `Const`s exist to compare.

**B and C swapped `2026-08-24`**, before implementing B — see Phase B's note and §1's Q1 correction.
The stub's two uses both require an environment, and iota reduction puts `eval` inside the
environment's scope, not outside it.

Strictly sequential, unlike D78 where Seam B's phases had slack. A gates everything because
visibility is meaningless without it; B gates E because consolidation on by-IRI equality would bake
in the unsoundness; D is where the parked obligation clears; E is the only phase touching the chain
format.

**Two tests are the progress markers**, both currently passing and both required to fail:
`entailment_beyond_set_inclusion_is_not_yet_decided` at D, and
`large_elimination_is_capped_at_set_not_type_n` at F. They pin limitations rather than semantics, so
they are self-announcing — if they still pass when the phase is called done, it is not.

## 9. References

- D75 §2 (the measurement), §4 (the chain as `Γ_env`), §5 (what it forces and costs), §8 Q1–Q4
- D78 §3.1 (the parked obligation), §7 (why D78 did not wait)
- nanoda_lib at `6ae1f0c` — `env.rs:37 DeclarInfo`, `expr.rs:54 Const{name,levels}`
- #188, #215, #228
