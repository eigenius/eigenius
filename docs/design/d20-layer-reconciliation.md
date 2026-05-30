# D20 — Layer Reconciliation

**Status:** Implemented (Phase 15; Witness / Rename / SchemaQuotient / Restructure strategies live with chain-resident MergeResolutionRecord)
**Phase:** 15
**Supersedes:** the `NeedsWitnessedMerge` resolution stub in D23 §5.4.3 — that doc shipped Phase 14 with conflict surfacing only and committed to deferring resolution machinery to this phase
**Companion docs:** D23 (out-of-core layer architecture; the trivial-merge primitive Phase 15 extends), D1 (Eigon serialization format; the typing rules every resolution must respect), the *Comorphism, Realized* paper (the institution-theoretic context Phase 15's witnesses plug into)

## 1. Summary

Phase 15 closes the gap left by D23: when two branches diverge with overlapping contributions, give the user a typed, kernel-checked menu of resolution strategies rather than the binary "save as sibling branch / give up" that D23 currently surfaces.

The design rests on a single structural commitment: **a layer merge is the pushout of a span of layer extensions**, where the span's apex is the most recent common ancestor and the legs are the two branches. The pushout is computed in two stages — first in the category of ontology presentations (the schema layer), then pointwise in the category of Set-valued instances (the data layer) over the merged ontology — and a conflict is precisely the failure of the universal property at one of those stages.

Once the merge is named structurally, every resolution becomes the same operation under the hood: **transform the input span, then take the ordinary pushout.** The user's choice of resolution is just a choice of transformation, which keeps the kernel surface small (one pushout machine) while the policy surface scales to whatever the user actually needs.

Six resolution strategies suffice for the conflict shapes we currently see in dev workflows: `Witness` (supply the universal arrow as a coercive comorphism), `Rename` (apply a disambiguating isomorphism to one side before pushing out), `KeepBoth`, `KeepOne`, `KeepNeither` (three flavors of quotienting the span), and `Restructure` (augment the ancestor with new common structure before computing the span). Each carries the minimum input the kernel can't infer; each composes with cascade impact analysis that surfaces downstream consequences before commit.

This document specifies the conflict taxonomy, the resolution surface, the kernel-side type discipline that makes resolutions well-typed, the gRPC additions, and the sub-milestone sequencing for an implementation that lands incrementally on top of Phase 14.

## 2. Motivation

D23 §5.4 ships a write model in which divergent branches with disjoint IRI contributions merge automatically (trivial merge) and divergent branches with overlapping contributions return `NeedsWitnessedMerge`, leaving resolution to the caller. The caller's options pre-Phase-15:

1. Save the would-be-merged chain as an auto-named sibling branch and revisit later.
2. Discard the chain and re-derive against the current head.
3. Defer indefinitely.

Each is unsatisfying. (1) accumulates `auto-*` branches that no one wants and that can only be pruned, not reconciled. (2) forces re-execution of work that may have taken hours of compute or wet-lab time. (3) is just (1) with the user's intent kept offline.

Real workflows produce overlapping contributions routinely: two notebooks editing different parts of an ontology that share a class definition; a long-running task evolving an ontology while the CLI loads fresh data; a multi-author refactor of cross-cutting types. The system needs a path from "we have a real conflict" to "the conflict is resolved, the merge committed" that doesn't require throwing away work.

D23 §5.4.3's trivial merge handles roughly 80% of dev-workflow divergence. Phase 15 is the remaining 20% — and that 20% includes most of what regulated R&D workflows actually look like, where multiple humans evolve the same ontology in parallel.

The categorical framing isn't gratuitous. We need a structural definition of "merge" precise enough that:

- Conflicts are defined by what they *are*, not by which heuristic detected them.
- Resolutions are well-typed: a malformed resolution is a kernel-level error, not a runtime hazard.
- The framework composes — a resolution applied today doesn't make tomorrow's merge any worse-behaved.
- The implementation can incrementally cover more conflict types without redesign.

Pushouts give us all four. The price is that the doc reads like category theory in places; we accept the price.

## 3. Goals and non-goals

**Goals:**

- A typed `MergeResolution` surface covering the six strategies named above.
- Kernel-checked well-typedness of each strategy: a malformed resolution fails commit-time validation rather than at runtime or in production.
- Cascade impact analysis that computes downstream consequences of each resolution before commit, with explicit user acknowledgment as a gate.
- Reuse of the existing `AutoOnLoad` institutional validation surface for post-merge gates.
- Coexistence with the trivial-merge fast path from Phase 14 — `update_branch` falls through to Phase 15's resolution machinery only when trivial merge fails.
- Migration of the existing `NeedsWitnessedMerge` outcome into the new typed resolution surface; `auto-*` branches created pre-Phase-15 remain reachable and can be merged through the new path.

**Non-goals:**

- Pre-declared branch merge policies (a `BranchMergePolicy` resource the kernel consults before surfacing conflicts). Useful for CI-style bot deployments; deferred until usage patterns make the right policy shape visible.
- Auto-resolution. The kernel never picks a strategy on the user's behalf, even when the conflict looks "obvious." (See §11.2 for why.)
- Synthesis of new ontology objects — generating `auto:CommonParent_xyz` for a `Restructure` resolution. Restructuring requires user-supplied IRIs; the kernel rejects synthetic-parent shortcuts.
- Conflict-resolution policy beyond the six strategies. Genuinely novel conflict shapes (e.g., conflicts in `conditional_requires`) become open questions in §11 rather than v1 deliverables.
- Cross-institution comorphism integration with merge witnesses. Phase 15's witness shape and the cross-institution comorphism shape from the *Comorphism, Realized* paper share a name and a type discipline, but their use cases don't overlap in v1; see §13.
- Distributed merge coordination — concurrent resolution attempts across multiple kernel instances on the same branch. Phase 14's single-`serve`-per-DB constraint applies.

## 4. Theoretical foundation

The framing borrows directly from Spivak's chapter on databases, categories, functors, and universal constructions (Fong & Spivak, *Seven Sketches in Compositionality*, ch. 3). We re-state the key correspondences here so the rest of the doc can refer to them without ambiguity.

**A layer is a category presentation.** An ontology layer L presents a category C_L: classes are objects, properties (with `data_type: resource`) are arrows, properties with literal `data_type` are arrows into external-reference objects (Spivak's "white nodes"), and constraints (`subclass_of`, `domain`, conditional requirements with non-conditional shape) generate path equations.

**Instances are Set-valued functors.** The resources at L form a functor I_L : C_L → Set, sending each class to the set of resources typed by that class and each property to the function carrying a resource to its property value. Multiple class membership (D1 §3.7) means an IRI may live in the union of class fibers; the functor still type-checks because each class's fiber contains the IRI separately.

**Layer extension is functorial.** A child layer L' that extends L gives a functor F : C_L → C_L' that is the identity on inherited content. The instance functor I_L' restricted along F yields I_L (the parent's data is the pullback of the child's data along F).

**A branch divergence is a span.** Two branches diverging from L_anc give:

```
        C_anc
       /     \
      F₁      F₂
     /         \
    C₁          C₂
```

with `(I_anc → Δ_F₁ I₁)` and `(I_anc → Δ_F₂ I₂)` natural transformations witnessing that the ancestor's data lives in both branches.

**The merge is the pushout.** The merged ontology C_merged is the pushout in **Cat** (or, more practically, in the category of finite category presentations) of C_anc → C₁ and C_anc → C₂. The merged instance is the pushout in `[C_merged, Set]` of:

```
   I_anc ─────→ Σ_G₁(I₁)
     │            │
     ▼            ▼
   Σ_G₂(I₂) ──→ I_merged
```

where G₁ : C₁ → C_merged and G₂ : C₂ → C_merged are the inclusion functors from the schema pushout, and Σ_G_i is Spivak's left pushforward along G_i (left adjoint to Δ). Σ pushes each branch's instance forward into the merged ontology, freely extending; the pushout in `[C_merged, Set]` then identifies what the ancestor demands be identified and nothing more.

That last clause is the universal property of a pushout, and it's the structural specification of "what a merge should do." A merge that identifies more than the ancestor demands has lost information from one of the branches; a merge that identifies less has spuriously duplicated ancestor content. The pushout is the unique combination satisfying both constraints.

**A conflict is a failure of the universal property** — at one of three stages:

1. **Schema pushout invalid.** The pushout in Cat exists abstractly but the resulting category violates Eigon's typing rules (e.g., a property has two distinct `data_type` arrows pointing into incompatible primitives).
2. **Schema pushout valid, equations contradict.** The merged category type-checks, but its path equations introduce contradictions that didn't hold in either branch (a forced inheritance cycle, a violated disjointness assertion).
3. **Instance pushout requires non-existent identification.** The schema is fine; the data isn't. The pushout in Set would need to identify two distinct values at a single IRI.

These three failure modes have different shapes and admit different resolutions. The taxonomy in §5 makes them concrete.

**Σ, Δ, Π for adjacent operations.** Spivak's data migration triad gives us a uniform vocabulary for operations that aren't merges but show up in the same workflow:

- **Δ_F (pullback)** is time-travel reads against an older schema.
- **Σ_F (left pushforward)** is the rebase primitive: data on C_old, schema moved to C_new, Σ-migrate the data forward.
- **Π_F (right pushforward)** is schema restriction: project a chain through a stricter schema.

Phase 15 uses Σ explicitly (in the merge pipeline above) and exposes Δ implicitly via D23's existing `--at-layer` time-travel reads. Π is not exposed in v1 but the surface is left open.

## 5. Conflict taxonomy

Each conflict the kernel surfaces carries a typed `ConflictKind` so the resolution UI can present the right options.

### 5.1 Schema-level conflicts

```rust
pub enum SchemaConflict {
    /// A Property has different `data_type` declarations on the two sides.
    PropertyDataType {
        property: Iri,
        branch_a_type: Iri,
        branch_b_type: Iri,
    },
    /// A Property's `class_types` differ.
    PropertyClassTypes {
        property: Iri,
        branch_a_classes: Vec<Iri>,
        branch_b_classes: Vec<Iri>,
    },
    /// Both sides added `subclass_of` arrows to incompatible parents.
    Subclass {
        class: Iri,
        branch_a_parents: Vec<Iri>,
        branch_b_parents: Vec<Iri>,
    },
    /// Both sides redefined the Class's `requires` differently.
    RequiredProperty {
        class: Iri,
        branch_a_requires: Vec<Iri>,
        branch_b_requires: Vec<Iri>,
    },
    /// Both sides added range/length constraints that admit different sets.
    ValueConstraint {
        property: Iri,
        branch_a: ConstraintSet,
        branch_b: ConstraintSet,
    },
}
```

These are stage-1 failures: the pushout in Cat itself doesn't fail (it freely admits both arrows), but the resulting category isn't a valid Eigon ontology.

### 5.2 Equation-level conflicts

```rust
pub enum EquationConflict {
    /// Merged inheritance forms a cycle that didn't exist in either branch.
    InheritanceCycle { cycle: Vec<Iri> },
    /// A class-disjointness assertion is violated by a resource the merged
    /// instance would need to type into both classes.
    DisjointnessViolation {
        class_a: Iri,
        class_b: Iri,
        offending_iris: Vec<Iri>,
    },
    /// A path equation derivable from one branch contradicts a path
    /// equation derivable from the other.
    PathEquationContradiction {
        equation_a: PathEquation,
        equation_b: PathEquation,
    },
}
```

Stage-2 failures: the schema combines, but the combined equational theory is inconsistent.

### 5.3 Instance-level conflicts

```rust
pub enum InstanceConflict {
    /// Same IRI, materially different resource bodies.
    IriCollision {
        iri: Iri,
        branch_a_body: ResourceBody,
        branch_b_body: ResourceBody,
        ancestor_body: Option<ResourceBody>,
    },
    /// IRI exists in one branch and was tombstoned in the other.
    DeletionConflict {
        iri: Iri,
        modified_body: ResourceBody,
        deleting_branch: Side,
    },
}
```

Stage-3 failures: the schema is consistent, but the data disagrees.

### 5.4 Composite conflicts

A single divergence often produces multiple conflicts spanning all three stages — branch A added a class with required properties, branch B added the same class with different required properties (schema-level), and the two branches each created instances of the new class with the same IRIs but different field values (instance-level). The resolution surface treats each conflict independently; users may pick different strategies for different conflicts within a single merge.

## 6. Resolution strategies

The unifying frame: **every resolution transforms the input span, then takes the ordinary pushout.** The strategies differ only in the transformation.

| Strategy       | Transformation applied                                                | Conflict types it addresses                                |
|----------------|-----------------------------------------------------------------------|------------------------------------------------------------|
| `Witness`      | Supply the universal arrow that makes the instance pushout exist     | `IriCollision`, `DeletionConflict`                         |
| `Rename`       | Apply an isomorphism functor renaming one IRI on one side             | `IriCollision` when accidental; some schema-level when an IRI was independently chosen |
| `KeepBoth`     | Identity (no transformation) — accept the freely-combined arrows      | `Subclass`, sometimes `PropertyClassTypes`                 |
| `KeepOne`      | Quotient out one side's contribution before pushing out               | All schema-level conflicts                                 |
| `KeepNeither`  | Quotient out both sides' contributions; result matches ancestor       | Schema-level conflicts where neither branch's evolution is accepted |
| `Restructure`  | Augment the ancestor with new objects/arrows before computing the span | `Subclass`, `PropertyClassTypes`, `InheritanceCycle`       |

### 6.1 Witness

A `Witness` resolution supplies a coercive comorphism: a typed transformation `(branch_a_value, branch_b_value, ancestor_value) → merged_value` that the kernel substitutes for the missing universal arrow.

```rust
pub struct WitnessResolution {
    pub conflict: ConflictId,
    /// IRI of the comorphism resource committed earlier in the chain.
    pub comorphism: Iri,
}
```

The comorphism is itself a regular Eigon resource — committed to the chain before the merge attempt, type-checked at commit time per the *Comorphism, Realized* paper §3.3 — that the kernel resolves and applies.

The transformation Component's signature must match `(A, A, Option<A>) → A` where A is the resource type at the conflicting IRI. The `Option<A>` ancestor lets witnesses handle the case where one branch added the IRI fresh. The kernel checks this at the point the resolution is submitted.

**Terminological note.** "Comorphism" is overloaded: the *Comorphism, Realized* paper uses the word for cross-institution forward translations (Δ_dock → Δ_assay via Arrhenius, etc.); this doc uses it for within-Eigon merge witnesses. The two share the triadic `(export, transformation, import)` shape and the same kernel-checked typing discipline, but address different operations. Internal docs and the SDK should distinguish them — `CrossInstitutionComorphism` and `MergeComorphism` — to keep the terminological footing clear. The shapes can stay unified at the Resource-class level.

### 6.2 Rename

```rust
pub struct RenameResolution {
    pub conflict: ConflictId,
    /// Which side's IRI to rename.
    pub side: Side,
    /// The current IRI being renamed.
    pub old_iri: Iri,
    /// The new IRI. Must not already exist in the merged chain.
    pub new_iri: Iri,
}
```

A rename is an isomorphism functor applied to one branch before the pushout. The kernel checks:

- `new_iri` doesn't collide with anything else in the chain (including the *other* branch — renames don't dodge real conflicts by introducing new ones).
- The renaming is consistent across the affected slice — every reference to `old_iri` in the renamed branch is updated. This is a closed reference walk over the branch's diff from the ancestor (cheap, computable in time linear in the size of the diff).
- No path equation breaks under the substitution.

Renames are particularly useful for **accidental IRI collisions**, where two teams in the same namespace independently chose the same local name for genuinely different concepts. They are *not* a tool for dodging genuine conflicts; the resolution UI (§7.3) surfaces this distinction.

### 6.3 KeepBoth, KeepOne, KeepNeither

These are the three ways to quotient the span at a schema-level conflict.

```rust
pub enum SchemaQuotient {
    KeepBoth,
    KeepOne { winner: Side },
    KeepNeither,
}

pub struct SchemaQuotientResolution {
    pub conflict: ConflictId,
    pub quotient: SchemaQuotient,
}
```

`KeepBoth` is the "no transformation" option — the pushout in Cat *already* admits both arrows, so this accepts the freely-combined result. It's only valid when the conflict type permits both sides coexisting (Eigon's multiple class membership makes `KeepBoth` legal for `Subclass`; it's never legal for `PropertyDataType` because a property can't have two primitive types).

`KeepOne` quotients out the loser's contribution: every arrow the loser added at this conflict point is dropped from the merge. The cascade analysis (§8) flags everything downstream that referenced the dropped contribution.

`KeepNeither` collapses both contributions back to the ancestor's state. The merged ontology, on this point, matches what the ancestor said.

The kernel rejects strategies that don't apply to the conflict type — e.g., `KeepBoth` on a `PropertyDataType` conflict returns a typed error rather than producing a merged ontology that won't load.

### 6.4 Restructure

```rust
pub struct RestructureResolution {
    pub conflict: ConflictId,
    /// Existing or new IRI for a parent class to introduce.
    pub new_parent: Iri,
    /// If `new_parent` is new, its full Class definition.
    pub new_parent_def: Option<ClassResource>,
    /// Existing classes that should now subclass `new_parent`.
    pub classes_under_new: Vec<Iri>,
    /// Whether the conflicting class itself goes under `new_parent`.
    pub affected_class_under_new: bool,
}
```

The motivating example: branch A added `Dog subclass_of Mammal`, branch B added `Dog subclass_of Reptile`. Restructure introduces a new `Animal` class, makes `Mammal subclass_of Animal` and `Reptile subclass_of Animal`, and changes `Dog`'s ancestor to `Animal` only — sidestepping the original conflict by raising the abstraction.

This is the most heavyweight resolution because it modifies the ancestor, not just the branches. The kernel's check: the resulting category (after augmenting the ancestor with the new structure and re-computing both spans against the augmented ancestor) must type-check, and any `subclass_of` arrows the restructure subsumes must be derivable from the new structure (transitivity through `Animal`) or else they're being silently dropped, which the user must explicitly acknowledge.

`Restructure` requires the user to supply the new IRIs explicitly; the kernel rejects synthesized parents like `urn:eigenius:auto:CommonParent_xyz`. Generated names produce unreadable schemas and undermine the structural intent of the resolution.

## 7. Public API

### 7.1 Conflict surface in `update_branch`

`UpdateBranch`'s `NeedsWitnessedMerge` outcome (D23 §5.4.1) is replaced with a richer `NeedsResolution` outcome:

```rust
pub enum UpdateOutcome {
    FastForward,
    TrivialMerge { merge_layer: LayerId },
    NeedsResolution {
        conflicts: Vec<TypedConflict>,
        candidate_chain: LayerId,  // the would-be-merged chain
    },
}

pub struct TypedConflict {
    pub id: ConflictId,
    pub kind: ConflictKind,
    pub applicable_strategies: Vec<StrategyKind>,
    pub cascade_preview: CascadePreview,
}
```

The `applicable_strategies` field is the kernel's typed answer to "what menu should the UI show for this conflict?"

### 7.2 Resolution submission

```rust
pub fn submit_resolution(
    branch: &str,
    candidate_chain: LayerId,
    resolutions: Vec<MergeResolution>,
    cascade_acknowledgments: Vec<CascadeAck>,
) -> Result<UpdateOutcome>;
```

The caller supplies one `MergeResolution` per `TypedConflict` from `NeedsResolution`. The kernel:

1. Type-checks each resolution against its conflict (rejects malformed or inapplicable strategies).
2. Computes the cascade impact of all resolutions in concert (§8).
3. Verifies the cascade acknowledgments cover the computed cascades; rejects with `IncompleteAcknowledgments` if any cascade is unacknowledged.
4. Applies the span transformations and computes the merge pushout.
5. Runs `AutoOnLoad` institutional validation against the merged resources (D23 §5.4.7); a `Fails` verdict aborts the resolution and returns a typed error.
6. On success, commits the merge layer atomically and CAS-updates the branch ref to it.

### 7.3 Cascade preview

```rust
pub fn preview_cascade(
    candidate_chain: LayerId,
    resolutions: Vec<MergeResolution>,
) -> Result<CascadePreview>;
```

A non-mutating endpoint the resolution UI calls between "user selected a strategy" and "user clicks commit." Returns the same `CascadePreview` shape the merge attempt would compute, letting the UI surface consequences before commit.

### 7.4 CLI

```
eigenius db merge resolve --branch <name>
    --candidate <layer-id>
    --conflict <conflict-id> --strategy <strategy> [--strategy-options]
    [--conflict <conflict-id> --strategy <strategy> ...]
    --acknowledge-cascade <cascade-id> [--acknowledge-cascade ...]
```

A single CLI invocation can resolve multiple conflicts. Cascade acknowledgments are required flags; the CLI prints the cascade preview and refuses to commit without explicit acks for every cascade item.

```
eigenius db merge preview --candidate <layer-id> [--strategy ...]
```

Non-mutating preview — prints the cascade impact for a tentative resolution.

## 8. Cascade impact analysis

A resolution can have consequences beyond the conflict itself. Dropping branch B's `Reptile` contribution invalidates every resource currently typed `is_a: [..., Reptile]` in branch B's chain, every property whose `class_types` referenced `Reptile`, every program signature mentioning `Reptile`. The user must see this *before* committing.

```rust
pub struct CascadePreview {
    pub items: Vec<CascadeItem>,
}

pub enum CascadeItem {
    /// A resource referenced something the resolution drops.
    OrphanedReference {
        resource: Iri,
        dropped_target: Iri,
        location: PropertyPath,
    },
    /// A program signature no longer type-checks.
    InvalidatedSignature {
        program: Iri,
        signature_problem: TypeError,
    },
    /// A class has resources whose `is_a` includes the dropped class.
    OrphanedTyping {
        class: Iri,
        affected_resources: Vec<Iri>,
        count: u64,
    },
    /// A trace references content that becomes inconsistent.
    InvalidatedTrace {
        trace: TraceId,
        reason: String,
    },
}

pub struct CascadeAck {
    pub item_id: CascadeItemId,
    pub user: Option<String>,
    pub acknowledged_at: Timestamp,
}
```

The cascade items are computed deterministically from the resolution and the branch chains. Computing them requires walking the closed reference graph rooted at the conflict point, bounded by the chains being merged. For typical chains (10–100 layers, low thousands of resources per layer), the walk is sub-second; the design assumes this and surfaces the preview synchronously rather than as a background job.

The acknowledgment requirement is enforced by the kernel, not the UI. A resolution submission lacking acks for any cascade item returns `IncompleteAcknowledgments` with the missing items. This keeps "I didn't know the merge would invalidate 47 program signatures" out of the failure-mode catalog.

## 9. Worked examples

### 9.1 Subclass conflict, three resolutions

Branch A: added `Dog subclass_of Mammal`. Branch B: added `Dog subclass_of Reptile`. Ancestor: `Dog` exists with no parent.

**KeepBoth.** Merged ontology has `Dog subclass_of Mammal, Reptile`. Cascade preview: any resource typed `is_a: [Dog]` is now also implicitly an instance of both `Mammal` and `Reptile`. If `Mammal` and `Reptile` carry a disjointness assertion, this surfaces as an `EquationConflict::DisjointnessViolation` and `KeepBoth` is rejected at submission time.

**KeepOne, winner=A.** Merged ontology has `Dog subclass_of Mammal`. Cascade preview: every resource in branch B's chain typed `is_a: [..., Reptile]` and naming `Dog` somewhere becomes inconsistent (either it's a Dog and not a Reptile, or it's a Reptile and not a Dog under the merged taxonomy). Acks required for each affected resource.

**Restructure.** User supplies `urn:project:Animal` as a new common parent. Ancestor is augmented with `Mammal subclass_of Animal`, `Reptile subclass_of Animal`. Both branches' `subclass_of` arrows on `Dog` are subsumed (both ancestors of `Dog` now route through `Animal`). Merged ontology has `Dog subclass_of Animal` only. Cascade preview: any program assuming `Dog subclass_of Mammal` directly will need its signature updated; the cascade lists the affected programs.

### 9.2 IRI collision via Witness

Branch A: `urn:project:patient_42` has `weight_kg: 75.0` (and 12 other fields). Branch B: same IRI, `weight_kg: 76.0` (lab re-measurement). Ancestor: same IRI, `weight_kg: 75.0`.

The user commits a `MergeComorphism` resource earlier in the chain whose transformation is "if both branches modified `weight_kg`, take the more recent measurement (B)." The witness's transformation Component has type `(Resource, Resource, Resource) → Resource` and is checked at commit time to be well-typed.

The resolution: `Witness { conflict: ..., comorphism: urn:project:weight_witness }`. Kernel applies the witness, produces the merged resource, runs `AutoOnLoad` validation (which checks the merged resource's structural validity and any institution-level invariants), and commits.

### 9.3 IRI collision via Rename

Branch A: `urn:project:Patient` is the medical-records class. Branch B: `urn:project:Patient` is the legal-billing class. Ancestor: no `Patient` class.

Both teams introduced the same local name for genuinely different concepts. Resolution: `Rename { side: B, old_iri: urn:project:Patient, new_iri: urn:project:billing:Patient }`. Kernel applies the rename across branch B's slice, re-computes the merge against the renamed branch, and commits. Cascade preview lists every resource in branch B's chain that referenced `urn:project:Patient` (now updated to the new IRI).

## 10. Test plan

| Milestone | Test surface                                                                                                  | Pass criterion                                                                                                                                        |
|-----------|---------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------|
| 15a       | Theoretical scaffolding: pushout machinery for finite category presentations; Σ migration along inclusions.   | Unit tests demonstrating pushout-of-trivial-span equals disjoint union; tests on hand-constructed spans with known conflicts.                          |
| 15b       | Instance-level `Witness` resolution; `MergeComorphism` resource shape and commit-time type check.             | Witnesses with mismatched signatures rejected at commit; valid witnesses produce expected merge layers; round-trip a witness through a worked example. |
| 15c       | `Rename` resolution; isomorphism-functor application; cascade walker.                                         | Renames update every reference in the affected slice; renames colliding with existing IRIs rejected; deep-cross-reference renames complete in expected time. |
| 15d       | `KeepBoth` / `KeepOne` / `KeepNeither` schema-level resolutions.                                              | Each strategy applied to canonical conflicts produces the structural expectations; inapplicable strategies rejected.                                   |
| 15e       | `Restructure` resolution.                                                                                     | Synthetic-parent IRIs rejected; user-supplied parents type-check; cascade preview correctly identifies subsumed subclass arrows and affected programs. |
| 15f       | Cascade impact analysis.                                                                                      | Cascade items computed deterministically; acknowledgment gate enforced; missing-ack errors typed correctly.                                            |
| 15g       | Resolution UI surface (gRPC + CLI).                                                                           | End-to-end test: notebook commits divergent layers, hits `NeedsResolution`, cascade preview returns expected items, resolution submitted with acks, merge layer committed, branch ref CAS-advanced. |

Cross-cutting tests:

- **Composite conflicts:** spans producing multiple conflicts of different types resolved in a single submission with mixed strategies.
- **Failure path:** `submit_resolution` rejected for malformed strategies, missing acks, and `AutoOnLoad` validation failures — each producing a distinct typed error.
- **Phase 14 compatibility:** trivial-merge fast path unchanged; `update_branch` with `AllowTrivial` policy still returns `TrivialMerge` for disjoint-IRI cases without entering Phase 15 machinery.

## 11. Open questions

### 11.1 Pre-declared branch merge policies

`BranchMergePolicy` resources committed alongside branch refs would let CI-style bots auto-resolve known-safe conflicts. The shape might be: per-conflict-kind default strategy, with explicit allowlists for which contributors' commits can use which strategies. Deferred to v2 because the right shape isn't clear without usage data.

### 11.2 Auto-resolution

Tempting in cases where the conflict looks "obvious" — e.g., one branch is a strict superset of the other. Declined for v1 because there's no obvious obvious case in regulated workflows: the user should always face the choice. Worth revisiting if patterns emerge.

### 11.3 Conditional requirements interaction

D1 §5.2's `conditional_requires` aren't path equations — they're predicate-conditional, which lives outside the category proper. A merge that combines two branches each satisfying its own conditional requirements may produce a merged dataset where the conjunction doesn't. Currently the cascade analysis catches this on the `AutoOnLoad` re-validation pass, but we don't have a clean way to surface it as a typed conflict in advance. Marked as a known imperfection of the framing; revisit if it bites real workflows.

### 11.4 Performance of cascade computation

The closed reference walk is linear in the size of the affected slice, but for very large databases (hundreds of layers, millions of resources per layer) the synchronous `preview_cascade` may exceed reasonable latency. Decision: ship synchronous v1 with a hard timeout and fall back to async if real workloads exceed it. A `cascade_preview_async` endpoint with a polling outcome is the natural shape.

### 11.5 Auto-named sibling branches from Phase 14

D23 §5.4.4 said `auto-*` branches accumulate from pre-Phase-15 `NeedsWitnessedMerge` outcomes, with no auto-expiration. With Phase 15 landing, those branches can finally be merged through the new resolution surface — but the workflow for "I have an `auto-*` branch from three months ago, what do I do with it?" needs UI support. The CLI `eigenius db divergence list` will gain a `resolve <branch>` action. Whether that should bias toward any particular strategy (e.g., default to suggesting `KeepOne` with the active branch as winner) is a usability decision deferred to implementation.

### 11.6 Restructuring across more than two branches

Phase 15 ships pairwise merges only — `update_branch` resolves a single span with one ancestor and two leaves. Three-way restructures (three teams independently extending the same class hierarchy) require sequential pairwise merges. Whether to surface a multi-way merge primitive is an open question; pairwise composition probably suffices for v1.

### 11.7 Witness composition

Two merges in sequence may each apply witnesses; the second merge sees a chain in which the first witness's translation has already been applied. Whether this matters for soundness depends on whether witness application is associative — i.e., whether `(merge with W1) then (merge with W2)` equals `merge with (W2 ∘ W1)`. Likely yes for the comorphism shape we use, but not formally established. Worth proving (or testing exhaustively against worked examples) before relying on it.

### 11.8 Schema-level conflicts on the same point of structure

When two branches both modify a single point of structure (e.g., both add `subclass_of` arrows to `Dog`), the conflict is well-localized. When branches modify *different* points that interact (branch A adds `Dog subclass_of Mammal`; branch B adds `Mammal subclass_of Reptile`, creating a transitive conflict), conflict detection has to walk the path-equation closure. Currently the detection is described as "compute the merged category, then check Eigon's typing rules." Whether this is performant on real-world ontology sizes (low thousands of classes, low tens of thousands of properties) is an open question; if not, an indexed equation-closure may be needed.

## 12. Sub-milestone sequencing

```
15a theoretical scaffolding   ──┐
                                │
                                ├─→ 15b Witness (instance-level)  ──┐
                                │                                   │
                                ├─→ 15c Rename                     ──┤
                                │                                   ├─→ 15g resolution UI
                                ├─→ 15d KeepBoth/One/Neither       ──┤
                                │                                   │
                                └─→ 15e Restructure                 ──┘
                                                                     │
                                                                     ├─→ 15f cascade impact
                                                                     │
                                                                     └─→ shipping
```

15a is the prerequisite for everything else: pushout computation on finite category presentations, Σ migration along inclusion functors. 15b–15e are independent and can be parallelised; 15f and 15g depend on the resolution surfaces but are themselves orthogonal. The natural shipping order interleaves 15b (the highest-value resolution; covers the bulk of dev-workflow conflicts) with 15c, then 15d, with 15e and 15g closing out.

## 13. Related work

**Spivak's chapter** is the direct theoretical backbone: schemas as categories, instances as Set-valued functors, data migration via Δ/Σ/Π. *Seven Sketches in Compositionality* (Fong & Spivak), Chapter 3, pp. 77–115. The pushout-as-merge framing follows directly from §3.5 (limits and colimits) generalized to colimits of instance functors. Spivak's adjoint triple (§3.4.3) gives us the migration vocabulary the merge pipeline uses for the Σ step.

The *Comorphism, Realized* paper notes that Spivak's adjoints are not directly used for cross-institution comorphisms (because cross-institution translations are forward-only domain math, not schema migrations). For within-Eigon layer merging, that constraint doesn't apply: branches share the same base institution (Eigon), the divergence is a proper schema-morphism span, and the Σ/Δ/Π triad applies directly. Phase 15 is where Spivak's tools come back into the picture, in a way they don't for cross-institution work.

**Goguen–Burstall institutions** and the Satisfaction Condition give the institutional context: signature morphisms (which include renamings) preserve truth, so a `Rename` resolution is sound by the same theoretical machinery the *Comorphism, Realized* paper invokes for cross-institution comorphisms. The two flavors of comorphism (cross-institution forward translation; within-Eigon merge witness) share the same triadic shape and the same kernel-checked typing discipline; we keep them under one Resource-class hierarchy in v1 with the naming-distinction noted in §6.1.

**Three-way merge in version control.** Git's three-way merge, Mercurial's, Dolt's, and TerminusDB's all approach the same problem from different points in the design space:

- **Git** treats files as opaque text blobs and merges line-by-line with no schema awareness; conflicts are whatever its diff algorithm flags. Resolutions are unstructured.
- **Dolt** merges typed tabular data with primary-key awareness (closer to our shape), and surfaces conflicts at the row level. Resolutions are still largely unstructured ("pick a side").
- **TerminusDB** merges typed graph data with schema versioning and offers a rebase model that resembles our Σ migration. It's the closest existing system to Eigenius's shape; their conflict UI is worth studying as a UX precedent.

Eigenius's structural commitment goes further than any of these: typed strategies, kernel-checked well-formedness, cascade impact gates. The categorical framing is what makes that commitment tractable to specify and implement.

**Adhesive categories and double-pushout graph rewriting.** The categorical theory of graph rewriting via pushouts is a mature literature; for finite category presentations like our ontologies, the double-pushout machinery is the appropriate formalism for substitution and rewriting. We don't import the theory wholesale — Eigenius's setup is simpler than general graph rewriting in important ways — but the literature is the reference point for "what could go wrong with naive pushouts" and informs the §11 open questions.

## 14. References

- D23 — Out-of-Core Layer Architecture (the Phase 14 trivial merge surface this doc extends)
- D1 — Eigon Serialization Format (the typing rules every resolution respects)
- *Knowing What You Know* (CACM article) — the user-facing framing of epistemic categories that resolutions must preserve
- *The Comorphism, Realized* — the institution-theoretic substrate; §3 in particular for the comorphism shape this doc reuses
- Fong, B. and Spivak, D.I. *Seven Sketches in Compositionality: An Invitation to Applied Category Theory*. Chapter 3 (Databases: Categories, functors, and universal constructions), pp. 77–115. arXiv:1803.05316, 2018. The direct theoretical reference.
- Goguen, J.A. and Burstall, R.M. *Institutions: Abstract model theory for specification and programming.* Journal of the ACM 39(1), 1992.
- Diaconescu, R. *Institution-independent Model Theory.* Studies in Universal Logic. Springer, 2nd ed. 2025.

Source code touchpoints (entering Phase 15):

- `kernel/src/layer/merge.rs` (new) — pushout computation, span transformations
- `kernel/src/layer/cascade.rs` (new) — closed reference walk, cascade item generation
- `kernel/src/storage/branch.rs` — `update_branch` outcome enum extension
- `kernel/src/comorphism/mod.rs` — `MergeComorphism` resource shape (alongside the existing cross-institution `Comorphism`)
- `proto/eigenius_kernel.proto` — `NeedsResolution` outcome, `submit_resolution`, `preview_cascade` RPCs
- `cli/src/db/merge.rs` (new) — resolution and preview commands