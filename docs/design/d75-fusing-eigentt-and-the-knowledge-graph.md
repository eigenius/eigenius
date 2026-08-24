# D75 — Fusing EigenTT with the typed knowledge graph

Status: draft. Written `2026-08-24` on `p2-residue`.

Supersedes `docs/notes/ttr-as-the-class-model.md` (deleted; the investigation is preserved in the
nine commits `fa03128..276fa5c`, which reached these findings in discovery order rather than causal
order). The subtyping question that note raised is genuinely independent of this thesis and moves to
`docs/notes/nominal-vs-structural-subtyping.md`.

## 0. The decision

**EigenTT and the knowledge graph were built as two systems that meet at seams, and every defect in
§3 is a seam failure.** They are not two systems. The graph already supplies what the theory is
missing, in both directions:

| | the graph has | the theory does |
|---|---|---|
| **Seam A — environment** | a chain of immutable layers, IRI-keyed, innermost-first lookup with shadowing | inline every global into the term; carry the layer as an *effect capability* |
| **Seam B — records** | resources: sets of IRI-keyed fields, open-world; `is_a` as membership | derive a right-nested `Val::Sigma` from what a *class* declares; `is_a` never becomes `:` |

Seam A: the chain **is** `Γ_env` in `Γ_env; Γ ⊢ e : T`. Seam B: a resource **is** a record type in
Cooper's sense (D62 §3 already says "exactly the Class-as-record-signature Eigenius uses"). The
theory currently reconstructs both from the wrong side.

Six of §3's eight symptoms are Seam A; two are Seam B. #188's residual is one of the eight.

Nothing here is implemented. §3.4 and §3.8 are witnessed by tests; the rest are read from code.

## 1. Thesis

Eigenius is a typed knowledge graph. "Typed" has been taken to mean *there is a type theory in the
repository*, and the two halves were built to meet at interfaces: the graph stores resources, the
theory checks terms, and a codec (D47) carries terms across. A term is type-checked; a resource is
validated; the graph's operations — commit, merge, institution dispatch — move resources.

The claim of this document is that the interface is the defect. The graph is not a store the theory
reads from; the graph is the theory's **semantics**.

**Seam A — the chain is the environment.** A typing judgment names two contexts: local binders `Γ`
and the global environment `Γ_env`. EigenTT has the first (`Rho`) and not the second, so every global
is inlined into the term. Without `Γ_env`, "well-typed" is a fact recorded without recording *checked
against what*, and every operation that changes the environment while leaving the term alone
preserves the record while destroying what it meant.

**Seam B — a resource is a record.** A resource is a set of IRI-keyed fields. That is a record, and
A11.2 builds record types by set union witnessed by field membership. The kernel instead computes a
right-nested Σ-chain from a *class's* `requires` + `recommends`, so a resource's type is its classes'
type and never its own — and the open-world admission the graph grants has no counterpart in the
type.

Neither seam is a missing feature. Both are the same thing built twice, once well on the graph side
and once badly on the theory side.

## 2. Evidence for Seam A: Γ_env is half-built, on one surface of three

`EvalCtx` (`kernel/src/nbe/eval/mod.rs:112-123`) puts the layer in the **effectful** arm, beside IO
and institution dispatch — filing "read a global declaration" as a capability. nanoda resolves
`Const{name, levels}` against `Env`, purely; the capability framing has no precedent in the
references.

But the kernel is not uniformly without an environment. It has one on **one** of its three surfaces:

| surface | environment | how |
|---|---|---|
| `check` | **partial** | `CheckCtx.layer: Option<Arc<Layer>>` + `type_cache` + `CheckHooks::resolve_class` (`check/mod.rs:60-82`, `check/hooks.rs:34-53`) |
| `eval` | **none** | `eval_impl` never resolves; the `EvalCtx` layer has one consumer chain-wide, `institution/eval_hooks.rs:1100` |
| `eq_nf` | **none** | `eq_nf(level: usize, v1: &Val, v2: &Val)` (`check/conv.rs:30`) takes no context at all |

The `check`-side environment is optional (`resolve_class_cached` errors with *"no layer access in
pure check mode"*), covers exactly one kind of global (classes), and is keyed by IRI string rather
than by `(LayerId, Iri)` as the chain's own memo is.

So the diagnosis is not "the theory has no environment." It is that **the environment was built once,
partially, for one global, on one surface** — which is why §3.3's two halves of the checker disagree
about what a class is, and why nothing that runs on the `eval` or `eq_nf` surfaces can see the chain
at all.

## 3. The symptoms

Grouped by seam. Every one is a place where the theory reconstructs something the graph already has.

### Seam A — the chain is the environment

### 3.1 Five reference variants, because each inlines its own environment

| variant | sites |
|---|---|
| `InductiveCtor` | 239 |
| `InductiveType` | 172 |
| `EigonClass` | 86 |
| `EigonAxiom` | 52 |
| `InductiveRec` | 34 |
| | **≈583** |

`EigonClass(Iri)` and `EigonAxiom(Iri)` are the same shape — an IRI resolved through the chain,
differing only in what the resource turns out to be. `EigonAxiom`'s own doc says it round-trips as
`ConstRef(iri)` exactly like `EigonClass`, and eval and readback are identity for both. Merging them
is unconditional: 138 sites become one.

The other three **fuse the former with its application**, which neither reference does — nanoda has
no applied-inductive node, and TTR builds record types by set union (A11.2 cl. 7) with application
separate. De-fused to `Const` + `App`, the count is one variant.

This is what makes **#188's residual** cost ~583 sites instead of ~120: universe levels have to be
threaded through five variants rather than one.

### 3.2 `PartialEq`-by-IRI, and the self-reference stub that forces it

Because declarations are inlined as `Arc<InductiveDecl>`, a constructor's own type must refer to its
inductive somehow. It does so with a **stub** — an `InductiveDecl` with empty params and ctors
(`term.rs:447`, `check/mod.rs:339`, `eval/mod.rs:604`) — which must compare equal to the full
declaration. So equality is by name:

```rust
impl PartialEq for InductiveDecl {          // term.rs:365-369
    fn eq(&self, other: &Self) -> bool { self.iri == other.iri }
}
```

**This makes levels on an inlined declaration unsound.** `List.{0}` and `List.{1}` carry the same
IRI and compare **equal**, identifying exactly the two types polymorphism exists to separate. #188's
residual cannot be done correctly while this holds.

With a `Γ_env`, the stub is unnecessary: a constructor's type names its inductive by `Const`,
resolved in the environment, which is what nanoda does.

### 3.3 δ exists in `check` and not in `eq_nf`, and they disagree — **witnessed**

`check` unfolds a class: `find_sigma_field` on a `Val::EigonClass` resolves through
`CheckHooks::resolve_class` to the Σ-chain whenever inference needs a field. That is δ-reduction,
performed eagerly and outside conversion.

`eq_nf(level: usize, v1: &Val, v2: &Val)` takes no context, so it compares `Val::EigonClass(iri)`
**opaquely**. The two halves of the checker therefore hold different views of what a class is:

```
eq_nf(EigonClass(Dog), EigonClass(Dog))  → Ok
eq_nf(EigonClass(Dog), Σ(name, breed))   → Err     // Dog's own unfolding
```

Witnessed by `a_class_and_its_own_unfolding_are_not_definitionally_equal` (`check/mod.rs`), which
first asserts check-side δ is live and that `Dog` does unfold to a `Val::Sig`, so the inequality is
not vacuous.

`Exp::EigonAxiom` is genuinely opaque by design — an axiom is a postulate with nothing to unfold
(`eval/mod.rs:510`) — and inductives sidestep the question by inlining their declaration. Classes are
the case where a policy exists on one side and not the other.

The δ-policy is also already **partly fixed elsewhere**: D66 §4 has `decode_type` unfold definitions
at decode time, and §3.4's proposition hash is taken over the decoded term. Any δ-policy adopted for
conversion has to agree with that, or the witness index and the conversion checker will disagree
about when two propositions are the same.

### 3.4 Proposition identity is environment-blind — **witnessed**

`hash_proposition_exp(proposition: &Exp)` (`witness/mod.rs:137`) takes only the term, and a class
reference encodes as a bare `ConstRef(iri)` (`eigentt_type_mirror.rs:139`). `WitnessKey` carries the
grounded resource's IRI. **No layer id is an input anywhere in that chain**, so the hash cannot
distinguish "C as defined in L1" from "C as defined in L2".

`hash_stored_proposition` (`witness_index.rs:120`) decodes against the layer first, so *definition*
bodies do enter the hash (D66 §4). Classes and axioms do not unfold, so they do not.

Two tests in `kernel/src/layer/witness_index.rs`, both passing against current behaviour:

- `redefining_a_class_does_not_change_the_hash_of_a_proposition_over_it` — `Π(x : Dog). Prop` hashes
  identically against a layer defining `Dog` and a child redefining it, with an `assert_ne!` on the
  two resolutions first so the redefinition is known to be real.
- `witness_credit_survives_redefinition_of_a_class_the_proposition_quantifies_over` — a `Declared`
  witness admitted under one `Dog` is still found by `lookup_chain_witness` from a layer where `Dog`
  is **wider** (a required property dropped, so strictly more things are Dogs). `Π(x : Dog). P` is a
  stronger claim there than the one that earned the credit.

The direction is load-bearing. Narrowing the class shrinks the domain and leaves stale credit sound
by accident; only widening exhibits the unsoundness.

**The failing soundness argument is written down in the kernel** (`witness_index.rs:30-31`):

> "First-hit-wins is sound because Layer immutability means a once-admitted witness stays admitted in
> all descendants."

Immutability makes the **record** stable. It does not make the **meaning** of what was recorded
stable, because a descendant can rebind a name the proposition mentions.

### 3.5 Merge: the checking path and the resolution path are different paths

In a linear chain, "rebound between `Γ_k` and `Γ_n`" is well-defined because there is one path. A
merge layer has two.

LCA defines class `C`; branch A redefines it; branch B adds resource `R` whose proposition mentions
`C`. `R` was checked in `Γ_B` against `C_LCA`. In the merge `M = [A, B]`, `C` resolves to `C_A`. `R`
was never below A, so along *its* path `C` was never rebound.

Three independent failures stack:

1. **Nothing detects it.** `MergeSpan::shared_iris` (`merge/conflict.rs:222-231`) is the set
   **intersection** of the two sides' contributed IRIs, and `classify_conflicts` (`:689`) runs the
   per-IRI classifier over exactly that set. Here `sources_a = {C}`, `sources_b = {R}` — intersection
   empty, zero conflicts. The hazard is a *reference* meeting a *redefinition*, which an intersection
   cannot see.
2. **Nothing validates it.** `commit_resolutions_as_merge_layer` copies both sides' bodies verbatim
   (`merge/resolve.rs:930-962`) and ends `builder.build(storage)` → `backend.store_layer(&layer)`
   (`:1360-1361`) — the `store_layer`-only adapter (`commit/backend_persister.rs:26`). No validation
   pass runs over the merged layer.
3. **The backstop is unimplemented.** Cascade analysis is resolution-triggered (`merge/cascade.rs:15-27`),
   so with no conflict it never runs; and D20's type-checker-driven kind is declared unbuilt —
   *"`InvalidatedSignature` (type-checker driven) and `InvalidatedTrace` (trace-store driven) require
   integration surfaces not yet stood up; they stay in the enum for forward compat."*

Reachable as the `MergeBranches` RPC (`proto/eigenius.proto:120`, `server/branches.rs:427`).

**Not yet witnessed** — this section is read from the code, not reproduced. See §7.

### 3.6 The institution boundary types resource shapes, not propositions

An institution's declared contract is an **input class**: `marshal.rs:105-115` resolves it on the
layer and checks `required_typed_properties` — arity and property shape. EigenTT well-typedness is
not part of the contract.

On the way out, `build_verdict_resource` (`dispatch.rs:374-388`) copies every non-protected property
the institution returned onto the chain-committed Verdict verbatim. The comment names the passthrough
set: *"e.g. statistics-institution's `canonical_proposition`, computed_statistic, computed_p_value"*.
`canonical_proposition` is a proposition crossing the boundary with no kernel check.

Rule 16 (`validation/rules/eigentt_value.rs`) does the real work — decode, `check_infer`, require
`Sort(0)` — but it keys off the property's **declared range** (`class_types ∋ eigentt:TypeExpr`) and
runs at layer-validation time, not at the boundary. The coverage is real but incidental: it holds
where the ontology happens to declare that range. A declared property with a different range can
carry a proposition that no type-level check ever sees.

**Not yet witnessed.**

### Seam B — a resource is a record

### 3.7 `resolve_class_type` builds a Σ-chain where the theory wants a record type

A11.2 clauses 7–8 build record types by **set union**, witnessed by field *membership*:

```
7. if R ∈ RType, ℓ does not occur in R, T ∈ Type,  then  R ∪ {⟨ℓ, T⟩} ∈ RType
8. r : R ∪ {⟨ℓ, T⟩}   iff   r : R,  ⟨ℓ, a⟩ ∈ r,  and  a : T
```

`resolve_class_type` instead walks `requires` + `recommends` into a right-nested `Val::Sigma` chain.
Cooper names this exact substitution (2021 paper §2.3):

> "as TTR record types are **sets of fields** there are **several Σ-types which intuitively
> correspond to a single record type** … this equivalence is **not directly derivable** … **These
> Σ-types do not have a witness in common.**"

Two consequences:

- **The encoding is well-defined only by accident.** Properties live in a `BTreeMap` keyed by IRI, so
  the Σ-chain comes out in canonical order. A data-structure choice stands in for a theory; nothing
  states the invariant and nothing would catch its loss.
- **Nominal `subclass_of` is forced, not chosen.** `collect_properties` walks `subclass_of`
  transitively, so a subclass's chain has a different *shape* from its parent's and shares no witness
  with it. Structural subtyping is unavailable to a Σ-chain and free for a record type.

See the deferred subtyping note for what changes if the relation is taken structurally rather than
nominally — that is a separate decision, and this section does not depend on it.

### 3.8 Open-world validation admits properties the type cannot mention — **witnessed**

Eigenius is open-world: a resource may carry properties its classes neither require nor recommend.
Rule 22 §c constrains only the *vocabulary* — the key must resolve to a declared `core:Property`.

The value keeps such a property: a resource marshals to `Val::ResourceVal(Box<Resource>)`
(`nbe/eval/marshal.rs:58`), carrying the whole resource.

The type does not. `resolve_class_type(class_iri: &Iri, layer: &Layer)` (`program/ground.rs:37`)
takes a **class**, not a resource, and builds its Σ-chain from `requires` + `recommends`
(`:63-81`). A resource's type is its *classes'* type, never its own.

So the two halves disagree, and the disagreement is reachable from the surface syntax:

- `Exp::PropAccess(e, prop)` resolves through `find_sigma_field` and returns
  `CheckError::IllFormed("property '…' not found in type …")` on a miss (`check/mod.rs:766-778`).
- `Exp::Construct(class_iri, fields)` likewise rejects a field the class does not declare
  (`:793-796`) — so an undeclared property cannot be *written* through a typed construct either.

**A resource that validates therefore has fields that cannot be projected.** Witnessed by
`an_undeclared_property_is_admitted_by_validation_but_cannot_be_projected` (`check/mod.rs`): with
`example:nickname` declared as a `core:Property` on the chain, `find_sigma_field` finds `name` on
`Dog` and does not find `nickname`.

**This is where record types help most.** A11.2 clause 8 —

```
8. r : R ∪ {⟨ℓ, T⟩}   iff   r : R,  ⟨ℓ, a⟩ ∈ r,  and  a : T
```

— witnesses by **membership**, not shape. Two consequences:

- **Open-world validation becomes derivable rather than policy.** A record carrying extra fields
  witnesses the smaller type *by clause 8*, so "a resource may carry more than its class declares" is
  a theorem about record types instead of a validation stance bolted beside a closed Σ-chain. This is
  the same shape as §4's derivation of Rule 22's retroactive scan, and is a second piece of evidence
  that the record model is the right one.
- **The extra fields become typed.** A resource's own record type is the union of its actual fields,
  so an undeclared property is *in* the type: projectable, quantifiable over, and mentionable in a
  proposition the kernel checks. Today it is data the type system cannot talk about.

### 3.9 `is_a` is checked — by a second implementation the kernel never sees

**Corrected.** An earlier draft of this section said the graph's membership and the theory's typing
are "disjoint", implying membership goes unchecked. It does not. Constraint satisfaction is verified
at commit:

- **Rules 1+2** (`validation/mod.rs:240-250`) require every `requires` property of every `is_a`
  class — **including inherited** via the `subclass_of` walk and **conditional** requirements — to be
  present on the resource.
- **Rules 3–10** then validate each property value against its declaration: `data_type`,
  `class_types`, `allows_only`, `domain`, format, pattern, range, length.

Together that is a complete constraint checker, and it runs over all 9.4M chain resources.

**The defect is duplication, not absence.** The check is performed by the *validator's* constraint
implementation and never by the kernel's type system:

| | checks membership | how |
|---|---|---|
| validator | yes | field presence + per-property declaration checks |
| kernel | no | `resolve_class_type` builds a Σ-chain that no membership decision consults |

Exactly two rules invoke the type checker — Rule 16 (decoded `eigentt:TypeExpr` values) and Rule 23
(inductive declarations). `is_a.rs` calls neither `check_infer` nor `resolve_class_type`. So the
Σ-chain exists for `Construct`, `PropAccess`, and D18 ontology-as-types *inside EigenTT terms*, while
every membership decision on the chain is made by the other implementation.

**And `Exp::EigonClass` is pinned, not stratified.** `check_type` accepts it unconditionally
(`check/mod.rs:253`) and `check` admits it against any `Sort(m)` with `1 ≤ m` (`:564-568`) — every
class inhabits `Set` and above, regardless of the class. So `core:Class is_a core:Class` (the core
ontology is self-typing; 21 of its 120 resources are instances of `core:Class`) is admitted because
the kernel never asks, not because stratification answers.

**This is the strongest Seam B item**, and the corrected form is the stronger one: the constraint
reading of §6.0 is not a proposal, it is **the shipped semantics of the validator**. §3.7 and §3.8
are consequences of the kernel implementing a different reading of the same thing.

**Do not read the kernel's behaviour as a design constraint.** On this axis it declines to ask the
question, so its behaviour is evidence of the duplication, not of a settled semantics.

## 4. Closing Seam A: the chain and its ancestors are Γ_env

`Layer::resolve_uncached` (`kernel/src/layer/mod.rs:713`) walks `parents.first()` innermost-first,
first hit wins, with `tombstoned_iris` for removal. `resolve_all` (`:780`) returns the whole
shadowing stack. The correspondence needs no construction:

| ML / Rust scope | Layer chain |
|---|---|
| binding set of a scope | `defined_iris` |
| enclosing scope | `parents.first()` |
| name lookup, innermost first | `resolve` |
| shadowing | a lower layer's IRI redefined higher |
| all bindings of a name | `resolve_all` |
| `Γ_env; Γ ⊢ e : T` | (layer, `Rho`) ⊢ `Exp` : `Val` |

**The purity side condition is already asserted in the code.** The resolve-memo comment
(`mod.rs:678-680`) states that the chain is immutable within a pass, *so `resolve(self, iri)` is a
pure function of `(self.id, iri)`* — which is exactly the condition under which environment lookup
belongs inside a pure judgment rather than behind a capability.

### The derived obligation

Shadowing in ML is benign because old bindings become unreachable. Here, resources in lower layers
still **reference** the shadowed IRI and were checked against the old binding. So:

> A term checked in `Γ_k` must be rechecked in `Γ_n` exactly when a name it mentions was rebound
> between them.

Rule 22's retroactive revalidation, scoped to redefinitions, is currently an operational rule. Under
this model it is **derivable** — which is evidence the model is right, and which is what §3.5's merge
path fails to implement and §3.4's witness index fails to notice.

## 5. What closing the seams forces

### Seam A

Reclassify `layer` from effect capability to judgment component, and the following stop being
choices:

- **Consolidation.** `Exp::Const(iri, levels)` resolves during normalisation → no inlined
  declaration → no stub → no `PartialEq`-by-IRI → levels distinguish instantiations → five variants
  collapse to one. #188's residual becomes levels on one variant.
- **Merge is a pushout of environments**, not a set union of resources. "Recheck what the pushout
  rebound" is part of taking the pushout. `InvalidatedSignature` stops being a forward-compat enum
  variant and becomes what merge *is*.
- **An institution's signature is a type in `Γ_env`**, so a proposition crossing the boundary is
  checked because crossing is an application — not because someone declared the right range on a
  property.

### Seam B

Read a resource as a record type over its actual fields, and:

- **A resource's type is its own**, not its classes'. `resolve_class_type(&Iri, &Layer)` becomes a
  function of a resource; the class type is the *declared minimum* the record must satisfy, not the
  whole of what it is.
- **Open-world admission becomes a theorem**, by A11.2 clause 8, instead of a validation stance
  standing beside a closed Σ-chain (§3.8).
- **Undeclared properties become projectable**, so `PropAccess` and `Construct` stop rejecting fields
  the graph already carries.
- **Field order stops mattering.** Union is order-free, so the `BTreeMap`-ordering accident that
  currently makes the Σ-chain well-defined (§3.7) is no longer load-bearing.

### What it costs

- **Conversion becomes environment-relative.** `eq_nf` gains `Γ_env` and needs a **δ-policy**: which
  declarations unfold, in what order, with what transparency.
- ~~**`Val` captures the environment.**~~ Withdrawn — see §8 Q1. `eval` already produces opaque
  values for chain references, so the neutral carries only the IRI and the environment lives in the
  conversion context, exactly as nanoda's `Tc` holds `Env` while `Const` holds name + levels.
- **Which layer is `Γ_env` mid-check is a real decision.** The layer under construction sees its own
  partial contents; nanoda extends `Env` declaration-by-declaration as each is checked. Forward
  references and intra-layer self-reference both turn on this.
- **Seam B needs a new `Val` former.** A record type is not a Σ-chain; `Val::Record` (or equivalent)
  is a kernel type addition with its own conversion, readback, and D47 codec arms. The Σ-chain path
  cannot be reinterpreted in place.
- **`subclass_of` stays nominal unless separately decided.** Record types make structural subtyping
  *available*; they do not make it *chosen*. See the deferred note.

## 6. Closing Seam B: a class is a constraint

### 6.0 The diagnosis

**A class is a constraint declaration, not an object sitting in a universe.** That single reading
dissolves more of §3 than any other move in this document.

Eigenius already implements it — **three times**, in three places that never meet:

| reading | where | what it does |
|---|---|---|
| constraint, **intensional** ("does `r` satisfy `C`?") | the validator | Rules 1+2 require every `requires` property of every `is_a` class, inherited and conditional included; Rules 3–10 check each value against its declaration |
| constraint, **extensional** ("which `r` satisfy `C`?") | the query engine | `MATCH ?x : C` compiles to `class_with_subclass_closure(C)` — a transitive `subclass_of` walk over the index — then `scan_chain(layer, is_a, concrete)` per class in the closure (`query/evaluate/pattern.rs:160-175`, `:346-362`) |
| class as **Σ-type** | the kernel | `resolve_class_type` folds `requires` + `recommends` into a right-nested `Val::Sigma` |

The first two are one relation seen from two sides — check it, or enumerate it — which is exactly
A11.2 clause 8 read intensionally and extensionally. Both run over all 9.4M chain resources. **The
kernel's Σ-type is the outlier**, used only inside EigenTT terms.

**The query engine already relies on entailment that nothing checks.** `class_with_subclass_closure`
returns instances declared at a *subclass* as answers to a query for the parent — sound only if the
subclass's constraint entails the parent's. Today that holds, but by an implementation coincidence:
`collect_properties` and the validator's Rules 1+2 walk `subclass_of` transitively, so an instance of
`Pup` was in fact checked against `Dog`'s requirements. Under Seam B, where a class carries an
explicit field set rather than inheriting one by collection, that coincidence disappears and the
query optimizer's closure becomes unsound. **This is an independent argument for 10d**: entailment is
already assumed by the query layer, and Seam B is what turns the assumption into an obligation.

### 6.1 Why the constraint reading makes classes level-generic

A constraint says: *for any r, r satisfies C iff for each ⟨ℓ, T⟩ in C, ⟨ℓ, a⟩ ∈ r and a : T.* The
orders come from the **fields' types**, not from C. C is a schema over levels — it does not sit at
one.

That is the same form as A10, which defines the hierarchy by metatheoretic schemas (`for each n`)
rather than object-level quantification. §6.3 records that an implementation "cannot defer the
indices" and must pick a concrete order per site or level variables. **For a constraint that
dichotomy does not bite**: a constraint is *checked* against resources whose field types have
concrete orders, so the order is determined per check and never has to appear in the class. Option
(1)'s restatement cost — a record type usable at two orders must be restated once per order —
evaporates, because a constraint does not need to *exist* at an order.

So `Exp::EigonClass` being pinned to `Sort(1)` and above (§3.9) is an artifact of the Σ-type reading.
Under the constraint reading it is not under-specified, it is **mis-specified**: a class was never a
thing at a level.

And `core:Class is_a core:Class` stops being the `Type : Type` hazard it looks like. It is a resource
satisfying a constraint that it itself declares — predicate satisfaction, not universe membership. No
stratification is needed to admit it.

### 6.2 Where this departs from TTR, and where it does not

Cooper does **not** separate "constraint" from "record type" — A11.2 clause 8 makes a record type
*be* its membership condition:

```
8. r : R ∪ {⟨ℓ, T⟩}   iff   r : R,  ⟨ℓ, a⟩ ∈ r,  and  a : T
```

Read as a definition, "the type" and "the constraint" are one thing. What is un-TTR-like is not the
constraint reading — it is Eigenius's **Σ-chain**, a type that is *not* its membership condition, and
that is why the two implementations were able to diverge without either being obviously wrong.

So the move is not "adopt constraints instead of TTR types". It is: adopt clause 8, at which point
the validator's constraint checker and the kernel's type former become the same thing, and the thing
they become is level-generic.

### 6.3 The synthesis

**A resource is a record, inhabiting a specific universe level, that satisfies 0 or more class
constraints.**

Three clauses, each doing work:

- **a record** — its type is derived from its *actual* fields, not from its classes (§3.8)
- **at a specific level** — computed from its field types, not declared (§6.1: a constraint is
  level-generic precisely because the levels live in the fields)
- **satisfying 0 or more constraints** — `is_a` is a checkable claim of satisfaction, and the record
  exists independently of what it satisfies

This closes three of the five questions §6.0 opened.

**Q8 — a constraint is a value, because a class is a resource.** This follows from the project's
founding principle: *"Everything is a `Resource` — no separate Class/Property/DataType Rust types."*
The value language violates it today —

```rust
Val::EigonClass(Iri),          // val.rs:62 — a class
Val::ResourceVal(Box<Resource>), // val.rs:66 — a resource
```

— two variants for a thing the architecture says is one. No new machinery is needed for Q8; the
existing machinery has to stop making the distinction.

**Q6 — `check_infer` of a class reference is the record type of the class resource, at its level.**
The hole in option C was that a constraint is not in `Sort(1)` or any sort. The synthesis dissolves
it with a use/mention split:

| | what it is | what classifies it |
|---|---|---|
| `C` **the resource** | a record with fields `requires`, `short_name`, `subclass_of`, … | its record type, at its level |
| `C` **the constraint** | the predicate that resource *denotes* | a predicate on records |

The reference carries the first; the second is reached by **interpreting** the resource, which is
what `resolve_class_type` already does. A class reference therefore needs no exotic judgment — it is
classified like any other resource reference.

**And this moves Q3 again — back to one variant, for a better reason.** If a class is a resource and
a reference to a resource is `Const(iri)`, then `EigonClass` is not a distinct kind of reference at
all. It disappears not by merging into a type former (option A's argument, which Q2 refuted) and not
by being categorically separate (option C's argument), but because **it was never a distinct kind of
reference**. What is special about a class is not how you refer to it but what you do with it.

So `r is_a C` is an **application** — the constraint denoted by `C`, applied to `r` — which is
precisely §3.9's missing judgment, supplied rather than bolted on.

| | variants | why |
|---|---|---|
| §3.1 as written | 5 → 1 | merge duplicates, de-fuse application |
| option C | 5 → 2 | a constraint is categorically not a type |
| synthesis | 5 → **1** | a class reference is a resource reference |

**Two consequences to flag, not resolve.**

*Levels are computed, not declared.* A record's level follows from its field types. That is consistent
with §6.1 — classes need no `uparams` — and it means `Const(iri, levels)` carries levels only for
declarations that are genuinely polymorphic (inductives, definitions), not for resource references.

*Rule 0 is a policy, not a necessity.* "Every resource must declare at least one `is_a` class"
(`is_a.rs:15`) contradicts "0 or more". Under the synthesis a classless record is well-formed, and
Rule 0 becomes a curation rule the chain chooses to enforce. Whether to keep it is a decision, but it
should be made knowing it is no longer forced by the model.

### 6.4 What TTR contributes

Cooper's appendix is an input, not the frame. Four things it supplies:

- **A11.2** is the formal statement of §3.7 and the fix for §3.8 — record types by union, witnessed
  by **membership** (clause 8), which makes open-world admission a theorem and gives undeclared
  properties a place in the type.
- **A11.4 fixed-point types** remove the self-reference stub of §3.2 from the other direction:
  self-referential record types are constructed as fixed points `ℱ(𝒯)` of a dependent record type
  `𝒯 = λr : T₁ . T₂((r))path`, rather than by threading a name-compared placeholder. This converges
  with §5's route; §5's is more conservative, introducing no new construction.
- **A11.5's unique-identifier notation** is the device Eigenius already uses for binder hygiene
  (`TC#`, `IDX#`, `IH#`): reference by an unforgeable identifier rather than a spellable name.
- **A11.6 dependency families** supply a discipline Eigenius lacks. Generalizing a record type to a
  path must carry `pathsπ(T)`, closed under both directions — *"If we remove a field on which another
  field depends we have to remove the dependent field as well."* Eigenius has dependent properties (a
  `class_types` referencing an earlier field) and **no check that a `subclass_of` respects them**.
- **A6 singleton types are manifest fields.** `T_a` with `b : T_a iff b : T and a = b`. Eigenius has
  this in disguise: `core:has_value` on a `core:ConditionalRequirement`, and `allows_only`
  enumerations — singleton or join types encoded ad hoc.

**On universes**, the finding is negative and worth recording. A10 defines the stratification as
**metatheoretic schemas** (`for each n`), not object-level quantification: no level variable in a
term, no declaration carrying universe parameters. EigenTT already implements A10 — `Exp::Sort(Level)`,
cumulativity via `Level::leq`, stratification as `Sort(l) : Sort(Succ(l))`.

But Cooper identifies `Typeⁿ` with Martin-Löf universes outright (Ch. 1 fn. 2), and his working
notation is level-**implicit**, said twice in §1.4.3.3: *"ignore the fact that this is bringing us
into danger of introducing Russell's paradox"*, and *"assume that our type systems are stratified in
this way without mentioning it explicitly for the most part."*

So an implementation of his notation must choose: a concrete order per site (a chain-resident record
type usable at two orders must be **restated once per order**), or level variables and instantiation.
**N3 §5a's recorded trigger — "Cooper is using universe polymorphism" — is inaccurate.** The accurate
statement: TTR's working notation is level-implicit, and an implementation needs polymorphism or a
concrete order per site.

§6.1 is the escape from that dichotomy for classes specifically: a *constraint* is checked at
concrete orders per site and never instantiated at one, so it pays neither cost. The dichotomy still
binds for anything genuinely used as a type in a term — which is what the re-scoped M1 (§8 Q3) is
looking for.

## 7. Build order

The two seams are independent. Seam A is the deeper one and gates #188; Seam B gates nothing and is
the one with a working reference design.

**Seam A**

1. **Witness §3.5 and §3.6.** Both are read from code, not reproduced. §3.5 first — merge is a live
   RPC and the claim "nothing detects this" should be a failing scenario, not a reading. Also check
   the asymmetric **tombstone** case (B tombstones an IRI, A references it): `DeletionConflict` exists
   as a `ConflictKind` and may or may not cover it.
2. **File the symptoms** under #215 (the type-theory-soundness tracker) so they exist independently
   of this document.
3. **The `Γ_env` design decision** — §5's costs are the agenda: the environment's shape, the δ-policy,
   the layer-under-construction question, and whether `EvalCtx` keeps two arms once the layer leaves
   the effectful one.
4. **Consolidation** to one `Const`, which is what makes #188's residual affordable.
5. **#188's residual** — levels on one variant.
6. **Merge as pushout**, and the institution boundary as application.

Steps 4–6 are gated on 3. Step 1 gates nothing but decides what step 2 files.

**Seam B**

7. **Decide `is_a` vs `:`** (M3, §8 Q3) — whether the validator's constraint checker and the kernel's
   type former are one relation. §6.0 says they are two implementations of one idea; this step makes
   that a decision rather than an accident.
8. **`Val::Record` as a kernel former**, with conversion, readback, and D47 codec arms — clause 8 as
   the membership rule, which is what unifies the two implementations.
9. **`resolve_class_type` becomes a function of a resource**, with the class constraint as the
   declared minimum rather than the whole type.
10. **`PropAccess` / `Construct` over records**, which is what closes §3.8.

Seam B is gated on nothing in 1–6, and §3.8 is the strongest single argument for starting it: it is
the one place where the current model loses information a resource already carries, rather than
losing soundness. The nominal-vs-structural decision is **not** on this path — record types make
structural subtyping available without requiring it.

## 8. Open questions, and the order they resolve in

```
Q1 ANSWERED ──▶ Q2 ANSWERED ──▶ Q3 OPEN ──▶ Q5 falls out
 env in            δ per kind      one opaque
 check + conv      classes opaque  Const, or two?
 not eval                              ▲
                              Seam B ──┘  (what projection returns)

Q4 recursor elimination universe ── independent ──▶ (#188 level work)
```

### Q1 — ANSWERED: the environment belongs to `check` and `conv`, not to `eval`

Three findings settle it.

**Not a materialised projection.** A name→declaration map built per pass is the full-chain-scan
antipattern that has produced OOMs here twice (`build_axiom_env`'s `iter_all_resources`; the
institution-index rebuild). Over a 9.4M-resource chain it is not viable. The memo at `mod.rs:678` is
*not* a projection — it is a lazy cache keyed by `(LayerId, Iri)`, which is the right shape and
already handles shadowing.

**`eval` does not need it.** `eval` already produces opaque values for every chain reference:
`Exp::EigonClass(iri) → Val::EigonClass(iri)` (`eval/mod.rs:506`), `Exp::EigonAxiom(iri) →
Val::Nt(Neut::EigonAxiom(iri))` (`:510`). It builds neutrals and defers — which is exactly nanoda's
shape, where `Const{name, levels}` is inert until `def_eq` unfolds it. So the environment does not
have to be threaded through evaluation.

**The size difference is decisive.**

| surface | call sites | needs `Γ_env` |
|---|---|---|
| `eval` / `eval_ctx` | 195 | **no** — already builds neutrals |
| `check_infer` / `check` | 89 | already has one, ad hoc |
| `eq_nf` | 23 | **yes** — this is the gap |

**Answer.** One environment trait, `Layer` as its implementation, reached by `check` and `conv`.
`CheckHooks` (`check/hooks.rs:34`) is already that boundary and should absorb the role, replacing
`CheckCtx.layer: Option<Arc<Layer>>` + `type_cache` and `EvalCtx::Effectful`'s layer field. The
`Option` must go: "no layer access in pure check mode" is precisely what let the three surfaces
diverge. `Val` does **not** capture the environment (§5 cost withdrawn).

### Q2 — ANSWERED for three kinds of global; the fourth is Q3

δ is decided **per kind**, not per declaration — so Eigenius does not need nanoda/Lean's transparency
annotations at this stage.

| global | δ | why it is not a choice |
|---|---|---|
| **definitions** | **transparent** | `decode_type` already unfolds them (D66 §4) and §3.4's proposition hash is taken over the decoded term. Changing it forks proposition identity. |
| **axioms** | **opaque** | a postulate has nothing to unfold; `eval/mod.rs:510` is already correct |
| **classes** | **opaque** | see below — transparency would silently make class identity structural |
| **inductives** | open | inlined today, so the question does not arise; it arises under Q3 |

**Why classes must stay opaque, and why that inverts the obvious fix for §3.3.** Two classes with
identical field sets are nominally distinct and structurally identical:

```
eq_nf(EigonClass(Alpha), EigonClass(Beta))  → false    // folded: compares IRIs
eq_nf(Σ(Alpha),          Σ(Beta))           → true     // unfolded: identical fields
```

Witnessed by `unfolding_a_class_would_collapse_two_nominally_distinct_classes` (`check/mod.rs`).

So making classes transparent does not merely reconcile `check` with `eq_nf` — it makes **class
identity structural**, which is the nominal-vs-structural decision deferred to
`docs/notes/nominal-vs-structural-subtyping.md`, taken by the back door via a δ-policy choice.
`subclass_of` is nominal and load-bearing for Rule 22, `class_types` and institution dispatch.

**Therefore §3.3 reconciles by fixing `check`, not `eq_nf`.** The naive reading — "`eq_nf` should
unfold to match `check`" — is the wrong direction. `find_sigma_field`'s unfolding must become a
**projection rule** that consults the environment to type a field access *without* asserting that the
class equals its unfolding. Field access needs the class's fields; it does not need the class to *be*
its Σ-chain.

This answer is **stable under Seam B**: if a class resolves to a `Val::Record` instead of a Σ-chain,
two same-field records are still definitionally equal, so the argument for nominal opacity is
unchanged.

### Q3 — the first genuine design decision, and how to assess it

Q2 constrains it from one side: classes are δ-**opaque**, so `EigonClass` cannot become a transparent
`Const`. Two options remain:

- **A — one `Const`, opaque by kind.** Opacity is a property of the resolved resource's kind.
  Keeps §5's collapse to a single variant.
- **B — a distinct nominal former retained** beside `Const`. Never resolves in conversion; keeps the
  duplication §3.1 identified.
- **C — a class is a constraint reference, not a type former at all** (§6.0). Then the question is
  not which variant carries levels but whether a class belongs in the `Exp` type-former position in
  the first place. Under §6.1 a constraint is level-generic *without* level parameters, so classes
  would leave #188's scope entirely rather than contributing one variant or two.

**C is structurally B, with a different justification — and that matters.** A term still has to
*refer* to a class: `Construct(class_iri, fields)` names one, `PropAccess` types against one. So C
does not remove the variant; it reclassifies it. What changes is the argument:

- Under B, `EigonClass` staying separate is a **compromise** — §3.1's duplication accepted to avoid
  threading levels through a second variant.
- Under C it is **correct**. A constraint and a type are categorically different, so two variants is
  the right shape and §3.1's duplication objection does not apply to this pair.

**The split is not where §3.1 drew it.** §3.1 grouped `EigonClass` and `EigonAxiom` as the same shape
(an IRI resolved through the chain) and proposed merging both. Under C they part company: an axiom is
a **postulated term** — a declaration with a type, carrying uparams exactly as nanoda's axioms do —
while a class is a constraint. So:

| variant | under C | levels? |
|---|---|---|
| `EigonAxiom` | merges into `Const(iri, levels)` | yes |
| inductive trio | de-fuses to `Const(iri, levels)` + `App` | yes |
| `EigonClass` | stays, as a constraint reference | **no** |

**Result: two variants, not one and not five** — and #188's residual is levels on `Const` alone,
which is the win §5 wanted, reached by a different route.

**Option C changes what M1 is for.**Still cross-gated on Seam B for what projection *returns* (Σ-chain vs record).

### Questions the constraint reading opens

Adopting §6.0 is not free of new decisions. Five, none of them answered here:

**Q6 — what is `check_infer(EigonClass(iri))`? — ANSWERED in §6.3.** The record type of the class
resource, at its level. The hole in option C came from conflating the class-resource with the
constraint it denotes; the use/mention split removes it.

**Q7 — what does `Construct(C, fields)` return?** Today, `EigonClass(class_iri)`
(`check/mod.rs:784-796`). If `C` is a constraint rather than a type, the constructed thing's type is a
**record type** and `C` is a check on it. Construct's result type changes — a representation
decision, not a relabelling.

**Q8 — is a constraint a first-class value? — ANSWERED in §6.3.** Yes, because a class is a resource
and "everything is a Resource". `Val::EigonClass` beside `Val::ResourceVal` is the architectural
violation, not the answer.

**Q9 — does institution dispatch key on declared `is_a` or on satisfied constraints?** Dispatch reads
`is_a` today: a declaration, decidable from one resource. Under the constraint reading, "is an
instance of C" is a *predicate to check*. Dispatching on satisfaction rather than declaration changes
both semantics and cost.

**Q10 — is `subclass_of` declared or derived?** See below; the deferred nominal-vs-structural
question re-enters through the constraint door, and C does not settle it. Q2's answer (classes
δ-opaque, identity nominal) is stable under C for the same reason it was stable under Seam B.

### The three remaining options, and how they couple

**Q7 — what does `Construct(C, fields)` return?**

| | option | consequence |
|---|---|---|
| 7a | the **record type of the given fields** — `C` checked as a side condition, absent from the type | correct for §3.8 (a resource's type is its own), but the nominal claim leaves the type, which is Q9's problem |
| 7b | **CHOSEN** — a **refinement**, `{r : Record{…} \| r satisfies C}` | carries structure *and* nominal claim; a new former, but see below on cost |
| 7c | **the class, as today** (`EigonClass(class_iri)`, renamed) | smallest change and dispatch keeps working, but it re-imposes the class's type on the resource — **reintroducing §3.8**, the defect the synthesis closes |
| 7d | 7a **plus a side-judgment channel** recording satisfied constraints outside the type | keeps both without a new type former; needs somewhere for the channel to live |

7c is a regression, not an option on the merits. The live choice is 7a/7d versus 7b, and it turns on
whether the nominal claim must be *in the type* or may live beside it.

**Q9 — does dispatch key on declared `is_a` or on satisfied constraints?**

| | option | consequence |
|---|---|---|
| 9a | **declared only** (status quo) | O(1) from one resource; but under "0 or more" declaration and satisfaction come apart, so a record that satisfies `C` without declaring it will not dispatch |
| 9b | **satisfied, computed** | structural, and cross-ontology alignment improves; the objection is *not* cost in general — the query engine already enumerates class membership by index — but that index is keyed on the **declaration** (`is_a` + `subclass_of` closure), not on field satisfaction. A satisfaction index would be a different index over field sets, and multiple satisfied constraints raise an ambiguity the dispatcher has no rule for |
| 9c | **CHOSEN — and already implemented** — declared, satisfaction checked at commit | Rules 1+2 and 3–10 already perform exactly this check (§3.9); dispatch on a declaration is therefore dispatch on a *verified* declaration |
| 9d | **declared for dispatch, satisfaction as a separate queryable relation** | both exist for different purposes; two relations to keep coherent |

**Q9c is the same decision as M3 — and it is already the shipped behaviour.** Satisfaction is
verified at commit by Rules 1+2 and 3–10 (§3.9), so dispatching on a declared `is_a` is dispatching
on a *verified* declaration, not on an unchecked assertion. 9a's objection — that declaration and
satisfaction come apart under "0 or more" — does not arise: they are kept together by the commit
gate.

**This re-scopes M3 and lowers its cost estimate.** M3 was framed as "if `is_a` becomes a typing
judgment, comparison volume rises by orders of magnitude". That was wrong: the satisfaction check is
already being paid, in the validator. The question is not whether to *add* a check but whether to
*unify* two implementations of one, and the cost delta is the difference between the validator's
field-wise check and the kernel's type-wise check — not the cost of a new pass.

**Q10 — is `subclass_of` declared or derived?**

| | option | consequence |
|---|---|---|
| 10a | **declared only** (status quo, nominal) | two identical constraints stay unrelated; `subclass_of` remains load-bearing for Rule 22, `class_types`, dispatch |
| 10b | **derived only** (structural) | `C ⊑ D` iff C's fields ⊇ D's; `subclass_of` becomes informational, and what "is a" means changes chain-wide |
| 10c | **declared authoritative, derived exposed as a view** | structural inclusion computed for alignment suggestions without being authoritative — the "additional, not replacing" option from the deferred note |
| 10d | **CHOSEN** — declared, with entailment checked | declaring `Pup : Dog` requires Pup's constraint to actually entail Dog's |

**10d is not currently checked, and today nothing needs it.** `collect_properties` walks
`subclass_of` transitively, so a subclass *inherits* its parent's requirements and entailment holds by
construction. Under explicit field sets that stops being automatic: nothing compares a subclass's
property declarations against its parent's, so a subclass could redeclare a property at an
incompatible type. Verified — no override, conflict, or variance check exists in `is_a.rs`,
`conditional.rs`, or `ground.rs`.

**The coupling to decide first.** 9c and 10d are the same stance — *declarations are authoritative
and checked* — applied to instances and to classes respectively. Choosing it once settles both, and
it is the stance that closes §3.9.

### Decisions: 10d and 7b

**10d — `subclass_of` declared, entailment checked.** Three arguments converge on it: it closes the
unchecked-redeclaration hole; it supplies the side condition 7b's subtyping rule needs; and it makes
the query engine's subclass closure sound rather than accidentally sound (§6.0). Entailment for field
constraints is decidable:
`C ⊨ D` iff C's field set includes D's, and for each shared field C's type is a subtype of D's. The
per-field half already exists as `subtype_of_inner` (`check/conv.rs`); the field-set half is set
inclusion. This is structural inclusion used to **validate** a nominal declaration rather than to
replace it — 10c's machinery with 10a's semantics — and it closes the hole recorded above, where a
subclass may today redeclare a property at an incompatible type with nothing to catch it. Needs a new
validation rule.

**7b — `Construct` returns a refinement.** Correcting the claim above that 9c makes 7a sufficient: it
does not, and the reason is **Q2**. Under 7a,

```
Construct(Alpha, {name = "x"})  :  Record{name : string}
Construct(Beta,  {name = "x"})  :  Record{name : string}      // identical
```

so the nominal distinction between `Alpha` and `Beta` — which Q2 established must be preserved, on
pain of class identity silently becoming structural — is erased at **every construction site**. 9c
checks the claim at commit, but the *type* still forgets it, so anything reasoning in the type
language loses it. 7b is therefore required for consistency with Q2, not merely tidier than 7c.

**10d and 7b supply each other.** A refinement type needs a subtyping rule:

```
{r : R | r sat C}  <:  {r : R′ | r sat D}     iff     R <: R′   and   C ⊨ D
```

`C ⊨ D` is exactly 10d's entailment judgment. 10d is the relation; 7b is the type that consumes it.
Neither is well-formed without the other, which is why they were separate questions with one answer.

**The cost of 7b is lower than first stated.** There is no refinement type today, but the kernel
already has *check-produces-evidence*: `NativeDecide(Constraint, Box<Exp>)` (`term.rs:86`) reduces a
decided constraint to `Refl`. So 7b introduces a new former, not a new idea — the inhabitation path
it needs is the one `NativeDecide` already walks.

**Q9 — 9c, and it is already implemented.** Satisfaction is verified at commit (§3.9), so dispatch
on a declared `is_a` is dispatch on a verified declaration. The three decisions are consistent: 9c
and 10d are the same "declarations are authoritative and checked" stance applied to instances and to
classes, and 7b is the type that carries the resulting claim into the term language.

What remains is not a decision but the unification: the validator checks satisfaction field-wise, the
kernel would check it type-wise, and Seam B's work is making those one thing.

### Q4 — the soundness half is done; the representation half is #188's trigger again

**What is already built.** The Prop restriction on large elimination is implemented and ported
against nanoda's `large_elim_test_aux`: `large_elim_admitted` (`check/inductive.rs:36-125`) is D46
§7 singleton-elim, Case A (zero constructors — `False`, `Asserts(iri)`) and Case B (one constructor
whose every non-parameter argument is propositional *or* is one of the conclusion's index
expressions, membership not mere mention). The soundness question — may a `Prop` inductive eliminate
into `Type` — is answered.

**What is not.** The motive's codomain is a constant:

```rust
let codomain_sort =
    if <decl is Prop> && !large_elim_admitted(decl) { Exp::sort(0) } else { Exp::sort(2) };
```

and its doc comment claims `Sort(2)` admits "Set, Type(n) all … via cumulativity". **That is false
for `Type(n)` with n ≥ 1.** A motive returning `Sort(k)` has type `I → Sort(k+1)`, and the recursor
demands `I → Sort(2)`, so only `k ∈ {0,1}` pass:

| motive returns | its type | admitted? |
|---|---|---|
| `Sort(0)` = Prop | `Sort(1)` | yes |
| `Sort(1)` = Set | `Sort(2)` | yes |
| `Sort(2)` = Type 1 | `Sort(3)` | **no** |
| `Sort(3)` | `Sort(4)` | **no** |

Pinned by `large_elimination_is_capped_at_set_not_type_n` (`nbe/level.rs`). **The effective ceiling
is Set.** No recursor in the system can eliminate into `Type 1` or above.

**This is #188's original trigger in a second location** — a level fixed at a concrete constant that
does not generalise, where raising it is an edit rather than an instantiation. `reasoning:spec_poly`
was the first.

**Options.**

| | option | consequence |
|---|---|---|
| 4a | leave it pinned at `Sort(2)` | ceiling stays at Set; no recursor eliminates into `Type 1`+ |
| 4b | bump the constant | moves the ceiling; this is precisely the ladder #188 exists to escape — "one bump per level, each a bootstrap ontology edit and a reseed" |
| 4c | **motive codomain becomes a level parameter** — `I.rec.{u}`, motive `I(params) → Sort u` | Lean/nanoda's shape; needs `Const(iri, levels)` (Q3) and declaration-level uparams (#188's residual) |
| 4d | infer the motive's level per elimination site, no parameter on the recursor | avoids uparams for recursors specifically |

**The framework discriminates 4c from 4d.** §6.1 argued a constraint is level-generic *without*
parameters because it is only ever **checked** at concrete orders, never instantiated. A recursor is
not like that: it is a term, it is applied, it appears inside other terms, and it must have a type
independently of any use site. It must therefore be *instantiated*, which requires a parameter.
**4c.** That is a derivation from §6.1's own criterion, not an appeal to the reference
implementation.

**4c preserves the existing gate unchanged.** The current two-way choice between `sort(0)` and
`sort(2)` becomes a choice between *`u` pinned to 0* and *`u` free* — `large_elim_admitted` keeps its
exact meaning and its exact call site. The soundness work does not have to be revisited; only the
representation changes.

**Consistency with Q5.** Inductives are one of the two kinds the framework says will emit a
`universe` declaration. A recursor carrying `{u}` is exactly that case, so 4c is what makes Q5's
surviving syntax load-bearing rather than vestigial.

**Sequencing.** 4c is gated on Q3 and #188's residual, so Q4 is no longer independent — it was
independent only while the answer was "pick a constant".

### Q5 — ANSWERED: it survives, it is already built, and the framework narrows its scope

`universe u v;` is implemented end to end and tagged `eigenius#188`: `TokenKind::Universe`
(`esl/lexer.rs:112,645`), `parse_universe` → `UniverseDecl` (`esl/parser.rs:315-318`), and emission
via `note_universe` (`esl/print.rs:80,148-157`).

**It is forced by round-trip, not chosen.** The printer's own comment states the reason:

> "a `universe` declaration, printed source that mentions one does not recompile without it."

A term carrying a level parameter must print source that reparses. So the syntax exists because
`every_shipped_ontology_document_round_trips` requires it, and the question was never really "should
ESL have this" — it was "will anything emit one".

**Under this framework, three of four kinds will not:**

| declaration | emits a level parameter? | why |
|---|---|---|
| classes | **no** | §6.1 — a constraint is level-generic *without* parameters, checked at concrete orders per site |
| resources generally | **no** | §6.3 — a record's level is computed from its field types |
| **axioms** | **yes** | a postulated term whose type quantifies over universes |
| **inductives** | **yes** | genuinely polymorphic — `List.{u}` |

So the framework does not retire the syntax; it **shrinks the surface that needs it** from "every
chain-resident type former" to "declarations whose types quantify over universes". #188's original
trigger — `reasoning:spec_poly`'s domain binder raised from `T : Set` to `T : Type 1`, fixed at
level 1 by hand — is exactly one of those, which is why it motivated the issue in the first place.

**What to do with Q5: nothing.** It needs no decision and no work. The residual is that no ontology
currently *writes* a `universe` declaration, because nothing can be level-polymorphic until
`Const(iri, levels)` exists (Q3). Q5 is downstream of Q3 in implementation order, not in design.

**Status.** Q1 and Q2 are answered above from evidence in the tree. Q3 is the open design decision,
now constrained on one side by Q2. Q4 is parallelisable. Q5 falls out of Q3.

## 8a. Status roll-up

**Settled on paper. Nothing here is implemented**, and the tests written during this analysis pin
*current* behaviour — the defects — not the design.

| | question | status |
|---|---|---|
| Q1 | environment interface | **answered** — `check` + `conv`, not `eval` |
| Q2 | δ-policy | **answered** per kind; inductives deferred into Q3 |
| Q3 | `EigonClass` as `Const`? | **answered conditionally** — 5→1, but the chain Q8→Q6→Q3 assumes Seam B lands. If the record model does not land, Q3 reverts to the A/B/C analysis. |
| Q4 | recursor elimination universe | **soundness done** (D46 §7 singleton-elim); representation open — motive pinned at `Sort(2)`, ceiling is Set. Options enumerated; 4c indicated. |
| Q5 | `universe u v;` in ESL | **answered** — already implemented end to end; forced by round-trip. The framework shrinks who emits one to axioms and inductives. No work. |
| Q6 | `check_infer` of a class reference | **answered** (§6.3) |
| Q7 | what `Construct` returns | **decided** — 7b |
| Q8 | is a constraint a value | **answered** — yes, it is a resource |
| Q9 | dispatch key | **decided** — 9c, and already implemented |
| Q10 | `subclass_of` declared or derived | **decided** — 10d |

**Still open beyond the numbered questions:**

- **Nominal vs structural subtyping.** Deliberately deferred
  (`docs/notes/nominal-vs-structural-subtyping.md`). 10d *uses* structural inclusion as a check while
  keeping nominal semantics, so it does not settle whether structural subtyping should also be
  available as a relation.
- **Rule 0 — policy or necessity.** "Every resource must declare at least one `is_a` class"
  contradicts the synthesis's "0 or more". Flagged in §6.3, not decided.
- **How a record's level is computed.** §6.3 says levels are computed from field types rather than
  declared. The rule itself (presumably `max` over field levels, with the successor at the type
  level) is not worked out, nor is its interaction with the currently-pinned `Sort(1)` for classes.
- **§3.5 (merge) and §3.6 (institution boundary) are argued, not witnessed.** Both are read from
  code. §3.4 and §3.8 have tests; these do not.
- **The asymmetric tombstone case at merge** — B tombstones an IRI, A references it. `DeletionConflict`
  exists as a `ConflictKind`; whether it covers this was never checked.

**Measurements:**

- **M1** (is any class used as a type in a term the constraint reading cannot serve?) — **not run**,
  and still worth running: it is the check that the synthesis does not break D18 ontology-as-types.
- **M2** (conversion resolve traffic) — **obsolete**. It existed to choose between Q3's options A and
  B; the synthesis answered Q3 on different grounds.
- **M3** — **resolved into Q9**, which turned out to be already-shipped behaviour.

**The load-bearing caveat.** Q3, Q6, Q7 and Q8 all rest on the record model. They are answers *given
Seam B*, not independent of it. Seam B itself has no implementation, no `Val::Record`, and no
migration of the three existing constraint implementations onto one. The design is settled; the
question of whether it survives contact with 9.4M resources is not.

## 9. References

- `references/publications/Cooper-2023-TTR-chaper-1.pdf` — Ch. 1, the working notation (§1.4.3.3,
  §1.4.3.5, fn. 2)
- `references/publications/Cooper-2023-TTR-appendix-1.pdf` — Appendices A1–A11, the formal system
- Cooper, *"So what's all this structure good for?"*, CSTFRS 2021 — §2.3 on Σ-types vs record types
- nanoda_lib at `6ae1f0c` — `env.rs:37 DeclarInfo`, `expr.rs:54 Const{name,levels}`
- D20 (merge), D49 §6 (witness admission), D62 §3 (class-as-record-signature), D66 §4 (definitions
  and the decode side)
- #188 (universe polymorphism), #215 (type-theory soundness tracker)
- `docs/notes/nominal-vs-structural-subtyping.md` — the deferred axis
