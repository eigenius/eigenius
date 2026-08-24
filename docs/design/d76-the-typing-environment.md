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

- **Q1 — the environment belongs to `check` and `conv`, not `eval`.** `eval` already produces opaque
  values for chain references (`Val::EigonClass(iri)`, `Neut::EigonAxiom(iri)`), so it builds
  neutrals and defers — nanoda's shape. 195 `eval` sites are untouched; 23 `eq_nf` sites are the gap.
  Not a materialised projection: that is the full-chain-scan antipattern behind two prior OOMs. The
  memo at `layer/mod.rs:678` is already the right shape — a lazy cache keyed by `(LayerId, Iri)`.
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
   one kind of global. What conversion needs is *what kind is this IRI, and if transparent, what does
   it unfold to*. That is one method returning a small enum, not a `Val`:

   ```rust
   enum Global { Transparent(Val), Opaque, Absent }
   fn lookup(&self, iri: &Iri, layer: &Env) -> Global;
   ```

   `Opaque` is the answer for classes and axioms (§1, Q2) and is what lets conversion stop without
   materialising anything.

2. **The `Option` goes.** `CheckCtx.layer: Option<Arc<Layer>>` and `resolve_class_cached`'s *"no layer
   access in pure check mode"* error are what let the three surfaces diverge (§0). An environment is
   a component of the judgment, so it is not optional — a caller with nothing to resolve against
   passes an **empty** environment, not `None`.

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

Whether that is unsoundness (a non-positive mutual pair admitted) or mere incompleteness is
**untested**, and it is the first thing to establish if a mutual pair is ever written. Recorded as a
question, not a claim — §6.4 says nothing forces the issue today.

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
linearised. The last one is what §6.5 needs — an inductive SCC is #20's trigger.

**Reusable:** `Exp::record`'s Kahn sort (D78 §1) is the same algorithm on a different graph.

---

### Phase B — remove the self-reference stub

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

---

### Phase D — δ in conversion

**Lands:** `eq_nf` / `subtype_of` / `subtype_of_with_hyps` gain the environment — **54 call sites**
(§3) — with §5's lazy-δ: equal names and levels compare equal *without resolving*.

**This is the phase D78 has been waiting on.** D76 §2.1's parked obligation discharges here, and
`entailment_beyond_set_inclusion_is_not_yet_decided` must **flip**.

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
A (order) ──▶ B (stub) ──▶ C (interface) ──▶ D (δ in conv) ──▶ E (Const) ──▶ F (4c)
                                                  │                  │
                                       unblocks D78 §3.1      reseed required
```

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
