# TTR as the model for classes — and what it says about universe polymorphism

Sources, all vendored: `references/publications/Cooper-2023-TTR-chaper-1.pdf` (Chapter 1, the
usage) and `Cooper-2023-TTR-appendix-1.pdf` (Appendices A1–A11, the formalisation), plus Cooper's
CSTFRS 2021 workshop paper *"So what's all this structure good for?"*. Written `2026-08-23`.

Chapter 1 and the appendix answer different questions and neither is sufficient alone: the appendix
gives the formal system, the chapter gives the notation actually used, and §1 below turns on the gap
between them.

## 1. Universe polymorphism: the formal system does not have it, the working notation assumes it

A10 ("The type `Type` and stratification") defines an intensional system of complex types as a
family of quadruples **indexed by the natural numbers**, with:

```
2. for each n,  Typeⁿ ⊆ Typeⁿ⁺¹                    — cumulativity
4. for each n>0, Typeⁿ ∈ Typeⁿ⁺¹                   — stratification
5. for each n>0, T :ⁿ Typeⁿ iff T ∈ Typeⁿ⁻¹
```

The `for each n` / `for any n` clauses are **metatheoretic schemas** defining the family. They are
not object-level quantification over levels: there is no level variable in a term, no declaration
carrying universe parameters, and no instantiation of a declaration at a level.

The two things TTR *calls* polymorphism are different mechanisms:

- **A4, partial function types.** `f : (T₁ ⇀ T₂)` iff there is some `T′` with `f : (T′→T₂)` and
  every `a : T′` satisfies `a : T₁`. That is polymorphism in the **domain**, driven by subtyping.
- **A3.1, polymorphic predicate signatures.** `Arity(P)` is a *set* of argument-type tuples —
  arity overloading.

Neither requires `uparams` or level arguments at reference sites.

**EigenTT already implements A10.** #188's first half gave `Exp::Sort(Level)` with `Zero`/`Succ`,
cumulativity through `Level::leq` in `subtype_of_inner`, and stratification as
`Sort(l) : Sort(Succ(l))` in `check_infer`. Measured against A10 clauses 2, 4 and 5, that is the
whole of what TTR's universe treatment needs.

**Consequence for #188.** The first half was worth doing on its own evidence — it removed the
`Level` `Ord` derive (a structural comparison masquerading as the universe order), `check_type`'s
`check(a, Sort(1))` fallback, and `result_sort`'s string grammar. None of that depended on Cooper.
But the **second half** — declaration-level `uparams` and level arguments at ~583 reference sites —
has now lost its stated justification. The TTR trigger is already satisfied by what shipped.

### The working notation

**Cooper identifies `Typeⁿ` with Martin-Löf universes outright** (Ch. 1, fn. 2):

> "In Martin-Löf type theory, types of types are called **universes**. This is, however, potentially
> a confusing terminology for a theory relating to the kind of model theory which has been used in
> linguistics where 'universe' has a different meaning."

So the hierarchy is a universe hierarchy; only the word is avoided.

**The working notation is level-IMPLICIT, and he says so twice** (§1.4.3.3):

> "For everyday working purposes we will assume that this is the system we have and **ignore** the
> fact that this is bringing us into danger of introducing Russell's paradox."

> "We will in future assume that our type systems are stratified in this way **without mentioning it
> explicitly for the most part**."

That is the crux. In the text, `Type` is written unindexed and treated as though `Type : Type`; the
stratification is the background story that makes it safe. **Writing `Type` unindexed and letting
the order be inferred is exactly the ergonomic universe polymorphism buys**, achieved in prose by
deferring the indices.

**An implementation cannot defer them.** When `Type` appears in a source file, something must
resolve it to an order, and there are two options:

1. **A concrete order per site** — a literal reading of A10. Then a record-type definition cannot be
   reused at another order; it must be restated, once per order it is needed at.
2. **Level variables and instantiation** — object-level polymorphism, which is what #188's remaining
   half proposes.

Cooper's construction is (1) in the metatheory and reads like (2) on the page. An implementation
that wants his notation needs (2).

### What this means for #188

N3 §5a records the trigger as "Cooper is using universe polymorphism". The accurate statement is
narrower and stronger: **TTR's working notation is level-implicit, and an implementation of it needs
either polymorphism or a concrete order per site.** Option (1) means a chain-resident record type
usable at two orders must be restated once per order in the ontology; that is the cost polymorphism
removes.

EigenTT today is stricter than Cooper's *working* system and matches his *formal* one: it has
`Sort(l) : Sort(Succ(l))`, never `Type : Type`. Nothing in the current chain writes a level-generic
definition, because nothing can. The question the TTR work will actually pose is whether a
chain-resident record type needs to be usable at more than one order — and if it does, option (1)
means restating it per order in the ontology, which is the cost polymorphism removes.

## 2. A11.2 is the formal statement of a defect in `resolve_class_type`

D18 §494 and D62 §3 already say a class *is* a record type — D62 says "**exactly** the
Class-as-record-signature Eigenius uses". The appendix makes precise what the current encoding
loses.

**Record types are built by set union, and witnessed by field membership** (A11.2, clauses 7–8):

```
7. if R ∈ RType, ℓ does not occur in R, T ∈ Type,  then  R ∪ {⟨ℓ, T⟩} ∈ RType
8. r : R ∪ {⟨ℓ, T⟩}   iff   r : R,  ⟨ℓ, a⟩ ∈ r,  and  a : T
```

Clause 8 gives subtyping directly: a record carrying *extra* fields still witnesses, because
witnessing tests membership rather than shape. And union is order-free, so a record type has no
field order to get wrong.

`resolve_class_type` instead walks `requires` + `recommends` and builds a right-nested
**`Val::Sigma` chain**. Cooper names this exact substitution in the 2021 paper §2.3:

> "as TTR record types are **sets of fields** there are **several Σ-types which intuitively
> correspond to a single record type** … this equivalence is **not directly derivable** from the
> characterization of Σ-types. **These Σ-types do not have a witness in common.** In TTR we use
> record types in place of Σ-types."

Two consequences for Eigenius, neither previously written down:

- **The encoding is well-defined only by accident.** Properties live in a `BTreeMap` keyed by IRI,
  so the Σ-chain comes out in a canonical order. A data-structure choice is standing in for a
  theory. Nothing states the invariant, and nothing would catch its loss.
- **Nominal `subclass_of` is forced, not chosen.** `collect_properties` walks `subclass_of`
  transitively, so a subclass's sorted chain has a different *shape* from its parent's, and by
  Cooper's observation the two share no witness. Structural subtyping is unavailable to a Σ-chain;
  it is free for a record type.

## 2a. Subtyping: nominal-generative vs structural-derived

Chapter 1 §1.4.3.5 defines it and gives the syntactic criterion:

> "T₁ is a subtype of T₂ (in symbols T₁ ⊑ T₂) just in case for any a, a : T₁ implies a : T₂, **no
> matter what is assigned to the basic types and ptypes**."
>
> "We can tell that (53b) is a subtype of (53a) simply by the fact that **the set of fields of (53a)
> is a subset of the set of fields of (53b)**."

More fields ⇒ subtype. And the relation is **modal** — it must hold in every possibility of a modal
system (§1.4.3.5, A9), so it is necessary rather than contingent on one model.

In Eigenius field inclusion holds **by construction**: `collect_properties` walks `subclass_of`
transitively, so a declared subclass inherits its parent's `requires` and its resolved type contains
the parent's fields. `class ex:Pup : ex:Dog { }` with `Dog` requiring two properties and `Pup`
declaring none resolves to the *same* Σ-type as `Dog`. No rule compares property sets because none
needs to.

The two systems get the relation from opposite directions:

| | Eigenius | TTR |
|---|---|---|
| direction | **nominal-generative** — declare `subclass_of`, inheritance follows, inclusion becomes true | **structural-derived** — have the fields, the relation follows |
| two classes with identical fields, no declaration | **unrelated** | mutual subtypes |
| "any record with a `name` field" | inexpressible — a class must be named | a record type |
| same structure in two ontologies | unrelated until a comorphism bridges them | already related |

The third and fourth rows are the ones that cost. Cross-vocabulary alignment is a standing problem
here — the institution/comorphism apparatus carries part of that load — and under a structural
reading two ontologies that describe the same shape are related without anyone declaring it.

That is the actual trade, and it is a genuine trade rather than a defect: nominal subtyping is
*intentional*, and `subclass_of` being a declaration is what makes institution dispatch and
`allows_only` decidable by looking at one resource. Moving to structural subtyping is not a
representation change, it changes what "is a" means chain-wide.

## 3. Two further mappings the appendix supplies

- **A6 singleton types are manifest fields.** `T_a`, with `b : T_a iff b : T and a = b` — a field
  pinned to a value. Eigenius has this in disguise: `core:has_value` on a
  `core:ConditionalRequirement`, and `allows_only` enumerations. Both are singleton or join types
  encoded ad hoc.
- **A11.6 supplies the dependency discipline Eigenius lacks.** Generalizing a record type to a path
  must carry its **dependency family** `pathsπ(T)` — closed under both "depends on" directions.
  The 2021 paper puts it plainly: "If we remove a field on which another field depends we have to
  remove the dependent field as well." Eigenius has dependent properties (a `class_types` that
  references an earlier field) and **no check that a `subclass_of` respects them**.

## 4. What this changes

### Why consolidation is the cheap route to #188's residual

The residual is expensive for one reason: levels have to go on **five** `Exp` variants, ~583
construction sites. The five exist for two separable reasons, and TTR bears on both.

**Two of the five are pure duplication.** `EigonClass(Iri)` and `EigonAxiom(Iri)` are the same
shape — an IRI resolved through the chain — differing only in what the resource turns out to be.
`EigonAxiom`'s own doc says it "round-trips as `ConstRef(iri)` exactly like `EigonClass`" and that
eval and readback are identity for both. Merging them is unconditional and owes nothing to TTR:
138 sites become one variant that carries levels once.

**The other three fuse the former with its application**, which neither reference does. nanoda has
no applied-inductive node; TTR builds record types by set union (A11.2 cl. 7) and applies functions
separately. De-fusing to `Const` + `App` would leave levels on **one** variant instead of five.

**What blocks de-fusing is a chain of two constraints, and TTR breaks the second.**

1. `EvalCtx::Pure` has no layer, so a `Const` cannot be resolved during normalisation. That is why
   the declaration is carried inline as `Arc<InductiveDecl>`.
2. Inlining forces `PartialEq` on `InductiveDecl` to be **by IRI** — because a constructor's own
   type contains a *stub* decl (empty params and ctors) that must compare equal to the full one.

Constraint (2) is what makes levels on an inlined decl unsound: `List.{0}` and `List.{1}` carry the
same IRI and would compare **equal**, identifying exactly the two types polymorphism exists to
separate.

**A11.4 removes the stub.** Cooper constructs self-referential record types as **fixed points** of a
dependent record type — `ℱ(𝒯)` for `𝒯 = λr : T₁ . T₂((r))path` — rather than by threading a
name-compared placeholder. A11.5's unique-identifier notation is the same device Eigenius already
uses for binder hygiene (`TC#`, `IDX#`, `IH#`): reference by an unforgeable identifier rather than by
a name that something else could also spell.

With self-reference handled by a fixed point, `PartialEq` no longer has to be by IRI, identity can be
structural, and levels distinguish instantiations correctly — **whether or not `EvalCtx::Pure` ever
gets a layer.** That is the load-bearing contribution: TTR does not merely supply a nicer class
model, it removes the reason for the design that makes #188's residual expensive.

**What remains open.** Constraint (1) — whether normalisation may consult a layer. Cooper says
nothing about it. §5 answers it from a different direction.

**What is NOT settled here.** Adopting record types is not a representation swap: `subclass_of` is
nominal and load-bearing for Rule 22, `class_types` and institution dispatch. Moving to
field-inclusion subtyping changes what "is a" means chain-wide. That is the question a follow-on
note has to open with.

---

## 5. Constraint (1): the chain and its ancestors *are* the environment

### The diagnosis

`EvalCtx` (`kernel/src/nbe/eval/mod.rs:112-123`) is:

```rust
pub enum EvalCtx {
    Pure,
    Effectful { layer: Option<Arc<Layer>>, hooks: Arc<dyn EffectHooks> },
}
```

The layer sits inside the *effectful* arm, beside IO and institution dispatch. That places "read a
global declaration" in the same category as "invoke a component" — but in every standard
presentation the judgment is `Γ_env; Γ ⊢ e : T`, and `Γ_env` is a **component of the judgment**, not
a capability. nanoda: `Const{name, levels}` resolves against `Env`, and that is pure.

The measurement is stronger than "`Pure` has no layer":

- `kernel/src/nbe/` contains **zero** calls to `.resolve(` and **zero** reads of `ctx.layer()`.
- The `layer` field has exactly one consumer chain-wide, `institution/eval_hooks.rs:1100`.

So it is not that one arm lacks an environment. **The NbE core has no notion of a global environment
at all.** Every global a term could need is inlined into the term. That is the generator of the five
reference variants: with no `Γ_env` to look anything up in, each variant carries its own resolved
payload.

### The chain already implements scoped lookup

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

The purity side condition is already asserted in the code. The memo comment at `mod.rs:678-680`
states that *the chain is immutable within a pass, so `resolve(self, iri)` is a pure function of
`(self.id, iri)`* — which is exactly the condition under which environment lookup belongs in a pure
judgment rather than behind a capability.

### What this dissolves

Reclassify `layer` from effect capability to judgment component and constraint (1) is gone:
`Exp::Const(iri, levels)` becomes resolvable during normalisation. Then

- no need to inline `Arc<InductiveDecl>`,
- so no self-reference stub — a constructor's type names its inductive by `Const`, resolved in
  `Γ_env`, which is what nanoda does,
- so no `PartialEq`-by-IRI, so levels distinguish `List.{0}` from `List.{1}`,
- so five reference variants collapse to one `Const`.

**The two routes converge.** §4 kills the stub by changing how self-reference is *expressed*
(A11.4 fixed points). This kills it by making `Const` *resolvable*. The second is the more
conservative of the two — it introduces no new type-theoretic construction, only moves an existing
lookup from outside the judgment to inside it — and it is the route nanoda already took.

### What it costs

**Conversion becomes environment-relative.** `eq_nf(level: usize, v1: &Val, v2: &Val)`
(`kernel/src/nbe/check/conv.rs:30`) takes no environment. That is sound only because nothing
δ-reduces: `Exp::EigonAxiom` evaluates to `Val::Nt(Neut::EigonAxiom(iri))` (`eval/mod.rs:510`), i.e.
opaque, and inductives carry their decl inline. Once names resolve, conversion needs `Γ_env` and a
δ-policy (which declarations unfold, in what order, with what transparency).

**`Val` captures the environment.** A neutral `Const` awaiting unfolding holds an `Arc<Layer>`.
Immutable and refcounted, but it changes `Val`'s lifetime story.

**Which layer is `Γ_env` mid-check is a real decision.** The layer under construction sees its own
partial contents; nanoda extends `Env` declaration-by-declaration as each is checked. Forward
references and intra-layer self-reference both turn on this.

### What it explains

Rule 22's retroactive revalidation scoped to redefinitions is currently an operational rule. Under
the environment reading it is **derivable**: ML shadowing is benign because old bindings become
unreachable, whereas here resources in lower layers still *reference* the shadowed IRI and were
checked against the old binding. A term checked in `Γ_k` must be rechecked in `Γ_n` exactly when a
name it mentions was rebound between them — which is the rule as implemented. An operational rule
falling out of the model is evidence the model is the right one.

### Consequence for #188

This, not the class model, is the first thing to settle. It decides whether levels land on one
variant or five, and it is prerequisite to both. It needs its own design note: the `Γ_env` shape,
the δ-policy for conversion, the layer-under-construction question, and whether `EvalCtx` keeps two
arms at all once the layer moves out of the effectful one.

---

## 6. Where the invariant is already violated

§5 derives the obligation: *a term checked in `Γ_k` must be rechecked in `Γ_n` exactly when a name it
mentions was rebound between them.* The linear-commit path implements it operationally (Rule 22's
retroactive scan, scoped to redefinitions). Two other paths do not.

### 6.1 Merge: the checking path and the resolution path are different paths

In a linear chain "rebound between `Γ_k` and `Γ_n`" is well-defined because there is one path. A
merge layer has two, and a term's checking path need not be its resolution path.

Take LCA defining class `C`; branch A redefines `C`; branch B adds resource `R` whose proposition
mentions `C`. In `Γ_B`, `R` was checked against `C_LCA`. In the merge `M = [A, B]`, `C` resolves to
`C_A`. `R` was never below A, so along *its* path `C` was never rebound — and no scan fires.

**Nothing detects this.** `MergeSpan::shared_iris` (`kernel/src/layer/merge/conflict.rs:222-231`) is
the set **intersection** of the two sides' contributed IRIs, and `classify_conflicts` (`:689`) runs
the per-IRI classifier over exactly that set. Conflict detection is definition-vs-definition. Here
`sources_a = {C}`, `sources_b = {R}`, intersection empty, **zero conflicts**. The hazard is a
*reference* meeting a *redefinition*, which the intersection cannot see.

**Nothing validates it either.** `commit_resolutions_as_merge_layer` copies both sides' bodies
verbatim (`try_load_resource` → `add_resource`, `resolve.rs:930-962`) and ends
`builder.build(storage)` → `backend.store_layer(&layer)` (`:1360-1361`) — the `store_layer`-only
adapter, described at `commit/backend_persister.rs:26` as the one with "no cache, no CAS", used by
callers without branch semantics. No validation pass runs over the merged layer.

**And the backstop is unimplemented.** Cascade analysis walks "each resolution's drop / rename
targets" (`cascade.rs:15-27`) — it is resolution-triggered, so with no conflict there is no
resolution and no cascade. Even with one, D20's type-checker-driven cascade kind is declared and not
built: *"`InvalidatedSignature` (type-checker driven) and `InvalidatedTrace` (trace-store driven)
require integration surfaces not yet stood up; they stay in the enum for forward compat."*

Reachable as the `MergeBranches` RPC (`proto/eigenius.proto:120`,
`server/branches.rs:427`).

### 6.2 Witness credit survives the rebinding

The sharpest form is not "an ill-typed term commits" — it is **a discharged proof obligation stays
discharged after its meaning changed.**

`hash_stored_proposition` (`kernel/src/layer/witness_index.rs:120`) decodes against the layer before
hashing, so *definitions* unfold (D66 §4) and a changed definition body changes the hash. Classes and
axioms do not unfold: `Exp::EigonAxiom` evaluates to `Val::Nt(Neut::EigonAxiom(iri))`
(`eval/mod.rs:510`), opaque, the IRI being the whole term. A proposition mentioning class `C`
therefore hashes **identically** before and after `C`'s structure changes. A proof witnessed under
`Γ_B` is credited under `Γ_M` against a different `C`.

The opacity that §4 identified as a symptom — nothing δ-reduces, so `eq_nf` needs no environment —
is the same property that makes proposition identity environment-blind here.

**Witnessed, not argued.** Two tests in `kernel/src/layer/witness_index.rs`:

- `redefining_a_class_does_not_change_the_hash_of_a_proposition_over_it` — `Π(x : Dog). Prop`
  hashes identically against a layer defining `Dog` and a child redefining it, with an `assert_ne!`
  on the two resolutions first so the redefinition is known to be real.
- `witness_credit_survives_redefinition_of_a_class_the_proposition_quantifies_over` — a `Declared`
  witness admitted under one `Dog` is still found by `lookup_chain_witness` from a layer where
  `Dog` is **wider** (a required property dropped, so strictly more things are Dogs). `Π(x : Dog). P`
  is a stronger claim there than the one that earned the credit. Narrowing would shrink the domain
  and leave stale credit sound by accident, which is why the test widens.

Both assert *current* behaviour and name §6.2, so they fail loudly if proposition identity ever
becomes environment-relative.

The failing soundness argument is written down in the module doc (`witness_index.rs:30-31`):
*"First-hit-wins is sound because Layer immutability means a once-admitted witness stays admitted in
all descendants."* Immutability makes the **record** stable. It does not make the **meaning** of what
was recorded stable, because a descendant can rebind a name the proposition mentions.

### 6.3 The institution protocol types resource shapes, not propositions

An institution's declared contract is an **input class**: `marshal.rs:105-115` resolves it on the
layer and checks `required_typed_properties` — arity and property shape. EigenTT well-typedness is
not part of the boundary contract.

On the way out, `build_verdict_resource` (`dispatch.rs:374-388`) copies every non-protected property
the institution returned onto the chain-committed Verdict verbatim, `r.set(prop_iri.clone(),
value.clone())`. The comment names the passthrough set explicitly — *"e.g. statistics-institution's
`canonical_proposition`, computed_statistic, computed_p_value"*. `canonical_proposition` is a
proposition, crossing the boundary with no kernel check that it is well-typed where it lands.

Rule 16 (`validation/rules/eigentt_value.rs`) does the real work — decode, `check_infer`, require
`Sort(0)` — but it keys off the property's **declared range** (`class_types ∋ eigentt:TypeExpr`), so
the obligation exists only where the ontology declares that range, and it runs at layer-validation
time, not at the protocol boundary. The protocol carries no obligation forcing either. An institution
that invents a declared property with a non-TypeExpr range can put a proposition in it and no
type-level check ever applies.

### 6.4 One defect, three symptoms

All three are the environment not being part of the judgment. When `Γ_env` is implicit — inlined into
terms (§4), carried as an ambient optional "layer" (§5), or replaced by a resource-shape contract
(6.3) — then "checked" is a fact recorded without recording *checked in what*. Every operation that
changes the environment without changing the term preserves the record while destroying what it
recorded.

Under §5's reframe the shape of the fix is forced rather than chosen. "`R` is well-typed" stops being
a property of `R` and becomes a property of `Γ ⊢ R`. Merge stops being a set union of resources and
becomes a **pushout of environments**, whose proof obligation — recheck what the pushout rebound — is
part of taking the pushout, not an optional cascade the user acknowledges. `InvalidatedSignature`
stops being a forward-compat enum variant and becomes the thing merge *is*. And an institution's
signature becomes a type in `Γ_env`, so a proposition crossing the boundary is checked because
crossing is an application, not because someone declared the right range on a property.

### 6.5 Status

Latent, not actively corrupting: the reseed, demo, and parse paths are linear commits, where Rule 22
covers the case. The exposure is the `MergeBranches` RPC and any institution emitting propositions
into slots whose declared range is not `eigentt:TypeExpr`. Sequenced behind §5 — a merge-side patch
before the environment is part of the judgment would be a guard against the symptom, and the guard
would have to be rewritten once the pushout obligation is what merge computes.
