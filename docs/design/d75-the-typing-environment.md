# D75 — The typing environment: the layer chain as Γ_env

Status: draft. Written `2026-08-24` on `p2-residue`.

Supersedes `docs/notes/ttr-as-the-class-model.md` (deleted; the investigation is preserved in the
nine commits `fa03128..276fa5c`, which reached these findings in discovery order rather than causal
order). The subtyping question that note raised is genuinely independent of this thesis and moves to
`docs/notes/nominal-vs-structural-subtyping.md`.

## 0. The decision

**The kernel's type theory has no global environment.** Every global a term could need is inlined
into the term. This document argues that the layer chain, with its parent pointers and IRI
shadowing, already *is* the environment `Γ_env` of the judgment `Γ_env; Γ ⊢ e : T`, and that
reclassifying it from an effect capability to a component of the judgment is the root fix for seven
separately-filed symptoms — of which #188's residual is one.

Nothing here is implemented. §3.4 is the only symptom currently witnessed by tests.

## 1. Thesis

A typing judgment names two contexts: the local binders `Γ` and the global environment `Γ_env`.
EigenTT has the first (`Rho`) and not the second.

Without `Γ_env`, "well-typed" is a fact recorded without recording *checked against what*. Every
operation that changes the environment while leaving the term alone therefore preserves the record
and destroys what the record meant. That single defect is the generator of §3.

## 2. Evidence: there is no Γ_env

`EvalCtx` (`kernel/src/nbe/eval/mod.rs:112-123`):

```rust
pub enum EvalCtx {
    Pure,
    Effectful { layer: Option<Arc<Layer>>, hooks: Arc<dyn EffectHooks> },
}
```

The layer sits in the **effectful** arm, beside IO and institution dispatch — filing "read a global
declaration" as a capability. nanoda resolves `Const{name, levels}` against `Env`, purely; the
capability framing has no precedent in the references.

The measurement is stronger than "`Pure` has no layer":

- `kernel/src/nbe/` contains **zero** calls to `.resolve(` and **zero** reads of `ctx.layer()`.
- The `layer` field has exactly one consumer chain-wide, `institution/eval_hooks.rs:1100`.

So it is not that one arm lacks an environment. **Neither arm supplies one to the kernel.** Terms
carry their own resolved payloads because there is nothing to look anything up in — and that is why
there are five ways to reference a global instead of one.

## 3. The symptoms

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

### 3.3 Conversion is environment-free, so nothing can δ-reduce

`eq_nf(level: usize, v1: &Val, v2: &Val)` (`kernel/src/nbe/check/conv.rs:30`) takes no environment.
That is sound *only* because nothing unfolds: `Exp::EigonAxiom` evaluates to
`Val::Nt(Neut::EigonAxiom(iri))` (`eval/mod.rs:510`) — opaque — and inductives carry their
declaration inline.

The absence of `Γ_env` and the absence of δ-reduction are the same fact stated twice.

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

This is the one symptom that is not caused by the missing `Γ_env`. It is included because it
constrains the same code and because the fix interacts: see the deferred subtyping note.

## 4. The chain and its ancestors are Γ_env

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

## 5. What the reframe forces

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

### What it costs

- **Conversion becomes environment-relative.** `eq_nf` gains `Γ_env` and needs a **δ-policy**: which
  declarations unfold, in what order, with what transparency.
- **`Val` captures the environment.** A neutral `Const` awaiting unfolding holds an `Arc<Layer>` —
  immutable and refcounted, but it changes `Val`'s lifetime story.
- **Which layer is `Γ_env` mid-check is a real decision.** The layer under construction sees its own
  partial contents; nanoda extends `Env` declaration-by-declaration as each is checked. Forward
  references and intra-layer self-reference both turn on this.

## 6. What TTR contributes

Cooper's appendix is an input, not the frame. Four things it supplies:

- **A11.2** is the formal statement of §3.7 — record types by union, witnessed by membership.
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

## 8. Open questions

- Does normalisation consult the layer directly, or is `Γ_env` a projection of it (a name→declaration
  map built once per pass)? The memo at `mod.rs:678` suggests the latter is already half-built.
- What is the δ-policy? Eigenius has no transparency annotations today; nanoda/Lean have several.
- Does `EigonClass` survive consolidation as a distinct variant, or become a `Const` whose resolved
  resource happens to be a class?
- Recursor elimination universe — deferred from #188 and untouched here.
- Whether `universe u v;` survives as ESL surface syntax.

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
