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

**Order within the migration.** The stub is the thing to remove first: it is what forces by-IRI
equality, which is what makes levels unsound, which is what #188's residual is blocked on. Everything
else in this section is mechanical once it is gone.

## 8. Phases

Deliberately unwritten. D78's phase structure worked because each phase had one risk class and one
gate; this document does not yet know enough to draw those lines — §6 is undecided and §5's unfolding
order wants a measurement. **Write the phases after §6 is settled**, not before.

The one thing that can be said now: **Q4/4c is last**, gated on `Const(iri, levels)` existing (§2.2).

## 9. References

- D75 §2 (the measurement), §4 (the chain as `Γ_env`), §5 (what it forces and costs), §8 Q1–Q4
- D78 §3.1 (the parked obligation), §7 (why D78 did not wait)
- nanoda_lib at `6ae1f0c` — `env.rs:37 DeclarInfo`, `expr.rs:54 Const{name,levels}`
- #188, #215, #228
