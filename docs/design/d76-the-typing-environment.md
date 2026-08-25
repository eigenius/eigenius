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

**Signal:** `large_elimination_is_capped_at_set_not_type_n` (`nbe/level.rs`) must flip.

### 2.3 #188's residual — declaration-level `uparams` and level arguments

Affordable only after consolidation to one `Const`, which is what removes the self-reference stub and
`PartialEq`-by-IRI. Without that, levels land on five variants and `List.{0}` compares equal to
`List.{1}`.

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

So D76 and D78 can interleave. The only ordering that matters is that D76 is a **chain-format change**
and D78 is not.

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

**This is the chain-format change**, and the sharpest difference from D78, which was additive
throughout (D78 §7).

`InductiveType(decl, args)` → `App(Const(iri), args)`, and the inlined `Arc<InductiveDecl>` stops
being carried in the term. What that touches:

- **606 `Exp` construction sites and 128 `Val` sites** (§3).
- **Readback**, which must produce a `Const` rather than reconstructing a decl.
- **The D47 codec**, both arms — and therefore **every persisted term containing an inductive
  reference**. A reseed is required; unlike D78's, it is not optional.
- **`PartialEq` on `InductiveDecl`**, which is by IRI today (`term.rs:365`) because a constructor's
  type carries a *stub* decl that must compare equal to the full one. With `Const` there is no stub,
  so equality can be structural — and that is what makes universe levels able to distinguish
  `List.{0}` from `List.{1}` (D75 §3.2).

**A pass that does not exist yet.** §6.2a: the declaration order prefix-visibility needs must be
*derived*, because `LayerBuilder` stores resources in a `BTreeMap<Iri, …>` and the input order is
gone. So the migration adds a dependency-ordering pass over each layer — it is not a refactor of
something already there.

**Order within the migration.** The stub is the thing to remove first: it is what forces by-IRI
equality, which is what makes levels unsound, which is what #188's residual is blocked on. Everything
else in this section is mechanical once it is gone.

## 8. Phases

**Each phase opens by reading the code it names, and correcting this document before writing any.**

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

- **It is a chain-format change.** D78 was additive to the chain throughout and forced no reseed
  (D78 §7). Phase E here rewrites every persisted term containing an inductive reference, and the
  reseed is mandatory.
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

### Phase B — remove the self-reference stub

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


**Lands:** a constructor's type refers to its inductive without the empty-`ctors` stub decl, so
`PartialEq for InductiveDecl` (`term.rs:365`) stops being by-IRI and becomes structural.

**Why second, and why not later.** §7: the stub forces by-IRI equality, which makes `List.{0}` compare
equal to `List.{1}`, which is what blocks #188's residual. Everything downstream in the migration is
mechanical once it is gone; nothing downstream is safe while it remains.

**Gate:** structural equality holds; the three stub sites (`term.rs:447`, `check/mod.rs:339`,
`eval/mod.rs:604`) are gone; existing inductive tests unchanged.

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

**Gate:** the parked test flips; `a_class_and_its_own_unfolding_are_not_definitionally_equal` is
**re-examined, not assumed** — Q2 says the reconciliation is to stop `check` treating its unfolding as
definitional equality, so that test's *class* case should still hold while the mechanism beneath it
changes. Plus: no resolve on the equal path, asserted by instrumentation rather than by reading.

---

### Phase E — consolidate to `Const`

**Lands:** `Exp::Const(iri, levels)`; `EigonAxiom` and the inductive trio fold into it (§8a keeps
`EigonClass` out);
`InductiveType(decl, args)` → `App(Const(iri), args)`. **606 `Exp` sites, 128 `Val` sites** (§3), plus
readback and both D47 codec arms.

**⚠ The chain-format change.** Every persisted term containing an inductive reference changes shape.
**A reseed is mandatory** — not optional as D78's was.

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

**Gate:** `large_elimination_is_capped_at_set_not_type_n` must **flip**; `large_elim_admitted` keeps
its exact meaning and call site, the two-way choice becoming *`u` pinned to 0* vs *`u` free*.

---

### Ordering

```
A (order) ──▶ C (interface) ──▶ B (stub) ──▶ D (δ in conv) ──▶ E (Const) ──▶ F (4c)
   done          eval IS in scope     needs C      │                  │
                                          unblocks D78 §3.1    reseed required
```

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
