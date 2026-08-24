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
| **Seam B — records** | resources: sets of IRI-keyed fields, open-world | derive a right-nested `Val::Sigma` from what a *class* declares |

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

## 6. Closing Seam B: what TTR contributes

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
concrete order per site. That is a real cost, but it is a different argument from the one on file.

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

7. **`Val::Record` as a kernel former**, with conversion, readback, and D47 codec arms.
8. **`resolve_class_type` becomes a function of a resource**, with the class type as the declared
   minimum rather than the whole type.
9. **`PropAccess` / `Construct` over records**, which is what closes §3.8.

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

### Q3 — the first genuine design decision

Q2 has now constrained it from one side: classes are δ-**opaque**, so `EigonClass` cannot become a
transparent `Const` that unfolds. The live options are narrower:

- a `Const` that is **opaque by kind** — one variant, resolved for projection, never unfolded in
  conversion; or
- a distinct nominal former retained alongside `Const`.

The first keeps §5's collapse-to-one-variant and is the reason to prefer it. Still cross-gated on
Seam B for what projection *returns* (Σ-chain vs record).

**Q4 — recursor elimination universe.** Independent of Q1–Q3 and of both seams. It concerns which
universe a recursor may eliminate into (large elimination, `Prop` vs `Type`), and depends only on
#188's level machinery. It can be taken up on its own schedule.

**Q5 — does `universe u v;` survive as ESL surface syntax?** The most downstream of the five. It is a
consequence of Q3: how many `Exp` variants carry levels determines whether declarations need
user-visible universe parameters at all. Deciding it before Q3 would be deciding surface syntax for a
representation that has not been chosen.

**Status.** Q1 and Q2 are answered above from evidence in the tree. Q3 is the open design decision,
now constrained on one side by Q2. Q4 is parallelisable. Q5 falls out of Q3.

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
