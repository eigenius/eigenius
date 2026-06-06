# D14: Institution Realisation

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 6; WASM + runtime-substrate institutions live)
**Supersedes:** D10 (Grothendieck Institution Protocol). D14 is the canonical reference for institutions in Eigenius; D10 is retained as a redirect file but carries no content.
**Depends on:** D1 (Eigon serialisation + structural validator), D9 (NbE / EigenTT), D12 (WASM extensibility), D19 (inductive types).
**Theoretical foundation:** Diaconescu, *Institution-independent Model Theory*, 2nd ed., Studies in Universal Logic, Springer 2025 (`diaconescu2025`). Chapter 14 is the canonical reference for the comorphism-based Grothendieck construction; cited inline as "Diaconescu 2025, Ch. 14, §X.Y". Goguen and Burstall (1992), "Institutions: Abstract model theory for specification and programming", JACM (`goguen1992`) — the underlying institution notion. The published paper [`docs/papers/eigenius-institutions.tex`](../papers/eigenius-institutions.tex) is the high-level narrative; D14 is the implementation contract.

---

## 1. Why this document exists

Eigenius is a typed knowledge-graph platform whose extensibility comes from *institutions* — domain-specific reasoning systems (FEA stress analysis, molecular docking, ADMET prediction, Lean 4 formal verification, LLM-based extraction) that contribute structured fibres of typed claims to a shared knowledge graph. The platform provides the *machinery* to realise a **Grothendieck institution** in the sense of Diaconescu (2025, Ch. 14): when multiple institutions are registered and comorphisms are declared between their fibres, the resulting indexed co-institution induces a Grothendieck institution whose flat-but-fibred body of knowledge lives in the layer chain. With zero comorphisms the platform holds only a *collection* of institutions co-existing over the Eigon base; the Grothendieck construction emerges only once cross-fibre comorphisms appear. D14 specifies the machinery — institutions, comorphisms, dispatch — not the construction itself, which is induced by what domain authors choose to declare.

This document specifies how that abstraction lands in code. It states the load-bearing semantic decision (what kind of institution-theoretic object a `Resource` is), describes the ontology shapes through which an institution declares its surface, defines the triadic realisation of a comorphism, the trait surface for the institution's runtime, and the kernel's dispatch model.

D14 supersedes the earlier D10 protocol document. D10 had four structural problems that surfaced when we tried to implement worked-example demos on top of it:

1. The model-vs-sentence interpretation of `Resource` was never made explicit, so the direction of `translate` (comorphism vs. morphism) was undecidable from the doc alone.
2. Comorphisms were modelled as a single function owned by the target institution, forcing the target to know the source institution's representational invariants — violating institutional encapsulation.
3. Declarations were procedural (a `fiber_declaration()` method returning a struct), creating two sources of truth: code and chain.
4. `validate_morphism`, `decide`, `query`, and `discover_morphisms` were treated as four separate trait methods, when they are operationally one primitive (a function from one resource class to another in the institution's fibre) plus the pure boundary translation.

D14 fixes all four. The migration is not strictly additive; the existing institution surface (`FiberReasoner`, `FiberDeclaration`, `translate`, `validate_morphism`, `discover_morphisms`, the corresponding WIT exports) is replaced rather than extended. §13 lays out the delta.

### 1.1 What is not an institution

The kernel itself is not an institution. Two kernel services provide the fixed foundation:

- **Eigon structural validator** — the 12 validation rules from D1 §5.4. Checks resource well-formedness against class definitions. Fixed, trusted, not extensible via the institution protocol.
- **EigenTT type checker** — the NbE-based CIC type checker from D9. Checks program composition, ground types, capability levels. Fixed, trusted, part of the kernel.

These are the switchboard — they validate and dispatch. Institutions provide domain-specific reasoning that the kernel mediates.

### 1.2 What is an institution

An institution is a domain-specific reasoning system that:

- Has its own notion of well-formedness — a satisfaction relation expressible (at runtime) as functions over typed Resources.
- Produces results with internal structure — morphisms within its fibre.
- Can answer queries about its own results.
- Registers with the kernel by committing typed declarations to the layer chain (§3) and providing a runtime that handles the boundary translations (§8).

Examples: FEA stress analysis, molecular docking, ADMET prediction, Lean 4 formal verification, LLM-based extraction.

### 1.3 Eigon as the shared signature category

Eigon is *neither* the kernel *nor* an institution. It is the **shared signature category** — the base over which the Grothendieck construction is performed.

In institution-theoretic terms:

- The **category of Eigon signatures** has ontology snapshots (layer-stack configurations) as objects and layer extensions as morphisms.
- The **base satisfaction condition** is the 12 structural validation rules — they define what it means for a resource to be well-formed against its class definition.
- Every institution builds **over** this base. Institution-specific sentences, morphisms, queries, and satisfaction relations add domain-specific structure to the shared Eigon foundation.

This is not a design choice — it is a mathematical necessity. The Grothendieck construction requires a base category; Eigon is that base. Without it, there is no shared language for institutions to register their types, declare their morphisms, or exchange data. Making Eigon an institution would require a meta-Eigon for institutions to register with, leading to infinite regress.

Concretely:

- **Resources, IRIs, properties, layers** — the Eigon data model is the kernel's native representation. It cannot be replaced or made optional.
- **The 12 validation rules** — structural satisfaction over the Eigon base. Fixed, not extensible.
- **EigenTT** — type checking of programs over Eigon ground types. A kernel service operating on the base.
- **Institutions** — domain-specific reasoning systems that register typed morphisms, queries, and satisfaction relations *as Eigon resources*. They build fibres over the base, not alternatives to it.

The relationship: Eigon provides the shared language. Institutions provide the domain-specific meaning. Neither can function without the other. The Grothendieck construction glues them together.

---

## 2. Institution theory mapping

From Goguen and Burstall (1992), an institution $\mathcal{I} = (\mathrm{Sign}, \mathrm{Sen}, \mathrm{Mod}, \models)$ consists of:

- $\mathrm{Sign}$ — a category of signatures
- $\mathrm{Sen}: \mathrm{Sign} \to \mathbf{Set}$ — a functor assigning each signature its sentences
- $\mathrm{Mod}: \mathrm{Sign}^{\mathrm{op}} \to \mathbf{Cat}$ — a functor assigning each signature its category of models
- $\models_\Sigma \subseteq |\mathrm{Mod}(\Sigma)| \times \mathrm{Sen}(\Sigma)$ — a satisfaction relation

The move from flat fibres ($\mathrm{Mod}(\Sigma) = \mathbf{Set}$) to structured fibres ($\mathrm{Mod}(\Sigma) = \mathbf{Cat}$) is the Grothendieck construction. Each institution contributes a *category* of models — objects with morphisms between them — not just a set of data points.

### 2.1 Indexed co-institutions and the comorphism-based variant

Diaconescu (2025, Ch. 14, §14.1) develops the Grothendieck construction in two equivalent forms: a *morphism-based* variant over an indexed institution $\mathcal{J}: I^{\mathrm{op}} \to \mathbf{Ins}$, and a *comorphism-based* variant over an indexed co-institution $\mathcal{J}: I^{\mathrm{op}} \to \mathbf{coIns}$. Proposition 14.10 shows the two yield isomorphic Grothendieck institutions whenever the indexed (co-)institutions are dual through an adjoint structure.

The Grothendieck construction itself is **not what any single institution does**. It is what emerges from an indexed co-institution — a *system* of institutions linked by comorphisms — when the construction is applied. Concretely in Eigenius: registering one institution gives you one fibre over Eigon; registering many institutions without comorphisms between them gives you many disjoint fibres; only when comorphisms are declared do those fibres become connected, yielding the indexed co-institution whose Grothendieck institution is what the published narrative ([`docs/papers/eigenius-institutions.tex`](../papers/eigenius-institutions.tex)) refers to. The kernel provides the machinery (the registry, the dispatch, the comorphism resource shape, the EigenTT middle); domain authors decide which fibres are connected, and that determines whether and how the Grothendieck institution exists.

Eigenius commits to the **comorphism-based variant** for two reasons:

1. **Theoretical convergence on the practical case.** Diaconescu (2025, §14.2 opening) states that "for the practical applications of Grothendieck institutions in computing… we will rely on the comorphism-based variant of the Grothendieck institutions, as in this case this is technically more convenient than the morphism-based variant." Theory co-limits and model amalgamation — exactly the operations layer reconciliation across institutions will rely on — are simpler to lift in the comorphism-based form.
2. **Empirical fit.** Practical inter-domain translations in Eigenius (e.g. docking ΔG → predicted IC₅₀; ADMET clearance → PK compartmental parameters) are not adjoint to anything; they are forward-only domain mappings. The morphism-based view would require fabricating fictitious adjoints.

This is a load-bearing decision. The kernel does not expose institution morphisms as a parallel mechanism — there is no "morphism dispatch" alongside comorphism dispatch — and the IRI classification table (§9) recognises only `Comorphism` and `Predicate` as institution-level call kinds. The "category of indices" $I$ is implicit: the registered institution IRIs are objects, the declared comorphisms are the generating morphisms, and we deliberately do not close $I$ under composition (§5.2).

### 2.2 CIC manifestation: Resources are sentences

The Goguen–Burstall presentation is set-theoretic. Eigenius is constructive: the kernel's type theory (EigenTT) is a fragment of CIC, and the knowledge graph is a Σ-typed bundle of typed terms. The institution-theoretic concepts therefore manifest as constructive analogues.

The load-bearing decision: **a typed `Resource` in the layer chain is a sentence in the institution's logic — a typed claim about the world. It is not a model.** Models are the implicit ground truth those sentences claim things about; the kernel does not represent models explicitly.

This is consistent with how institution theory is used in software engineering (CASL, Hets, the Diaconescu programme generally): the data-level objects in such systems are typed specifications, not interpretations. It is also consistent with our concrete representation: a `DockingResult` resource carries a *prediction* about a binding affinity; an `AssayMeasurement` resource carries an *observation* about an IC₅₀; both are claims, not the underlying molecule. The molecule itself is the model — never a kernel object.

| Component | Set-theoretic form (Goguen–Burstall) | Constructive form (Eigenius / EigenTT) |
|---|---|---|
| Signatures $\mathrm{Sign}$ | Objects of a category | Layer-chain ontology snapshots; classes are inductive types and properties are typed fields |
| Sentences $\mathrm{Sen}(\Sigma)$ | Sets of formulas over $\Sigma$ | Typed Resources in the layer chain — instances of registered classes, carrying ground claims |
| Models $\mathrm{Mod}(\Sigma)$ | Categories of $\Sigma$-structures | Implicit. Models are the "real world"; the kernel never materialises them as Resources |
| Satisfaction $\models_\Sigma$ | Boolean-valued relation | Implicit relation between sentences and the unrepresented ground truth. The kernel cannot evaluate truth-in-the-world; it can evaluate well-formedness (structural validation) and domain acceptance (institution validation, §6.5) |
| Morphisms in a fibre | Morphisms in $\mathrm{Mod}(\Sigma)$ | Typed functions from one resource class to another internal to one institution — institutions' queries (§6) |
| Comorphisms between fibres | $\rho^{\mathrm{Sen}}: \mathrm{Sen} \Rightarrow \rho^{\mathrm{Sign}}; \mathrm{Sen}'$ (sentences forward) | Triples $(s, m, t)$ realising sentence translation across an institution boundary (§5) |

Phase 11c's decide procedures already realise satisfaction-as-evidence directly: a `DecResult::Holds` reduces `Exp::NativeDecide(c, v)` to $\mathrm{Refl}(v)$ — that $\mathrm{Refl}$ *is* the witness term inhabiting the satisfaction type. `DecResult::Fails` produces a failing neutral (empty inhabitation), and `DecResult::Undecidable` leaves the constraint as a passthrough neutral. The `Verdict` shape (§6.1) generalises this for institution-bound predicates.

**Σ-types as Grothendieck signatures.** The Grothendieck construction's signatures are pairs $\langle i, \Sigma \rangle$ where $i$ is an index and $\Sigma$ is a $\mathrm{Sig}^i$-signature. In CIC this is the dependent sum:

$$\mathrm{Sign}_{\sharp} = \sum_{i : I} \mathrm{Sign}(i)$$

Eigenius realises this Σ-type implicitly — class IRIs are flat strings, but the registry's class-IRI → institution-IRI mapping (built by chain scan, §3) recovers the dependent pair structure on demand. We do not need to materialise the Σ-type explicitly because a single lookup answers "which $i$ does this $\Sigma$-signature belong to?".

---

## 3. The ontology-first principle

Every concept an institution exposes is a typed Resource committed to the layer chain. The kernel's institution registry is a **derived index** built by scanning the chain — not a parallel source of truth.

There is no procedural `fiber_declaration()`. An institution ships:

- **Code** — the WASM binary that handles dispatch when the kernel calls the institution's runtime (§8).
- **Ontology** — a set of typed Resource declarations (Institution, ExportFormat, ImportFormat, QueryClass, Comorphism, plus the institution's own classes and properties) that describe what the institution exposes.

Code and ontology can be packaged together (the WASM binary returns its declaration document at install time and the kernel commits it as a layer) or shipped separately (a `*.wasm` plus a `*.eigon-json`); the ontology shape is the same either way. This separation lets an institution evolve its declared surface — adding a new ExportFormat, deprecating a query class — without rebuilding the WASM, and lets external authors contribute declarations that reference an existing institution's procedure handlers.

Phase 14 layer reconciliation across branches handles institution-declaration merges using ordinary layer-reconciliation rules — there is no special path for institution metadata. Phase 9a rehydration walks the chain at startup; institutions become consumers of the chain rather than co-publishers parallel to it.

---

## 4. The vocabulary of declarations

Five resource shapes carry the institution's declared surface. All live in `ontologies/institution/institution-ontology.json` (not `core` — they are not part of the bootstrap signature).

### 4.1 `Institution`

Identity and capability metadata for a registered reasoning system. One per institution.

| Property | Type | Meaning |
|---|---|---|
| `institution_iri` | IRI string | Unique identifier (e.g. `urn:eigenius:institutions:dock`). |
| `name` | string | Human-readable label. |
| `runtime` | IRI of a runtime kind (`Wasm`, `External`, `InProcess`) | How the kernel reaches this institution. Determines dispatch policy. |

### 4.2 `ExportFormat`

A typed *outbound* view of one of the institution's resource classes. The institution advertises one ExportFormat per (resource class, payload type) pair it wishes to expose.

| Property | Type | Meaning |
|---|---|---|
| `from_class` | IRI of a Class | The resource class this format extracts from. |
| `payload_type` | IRI of a EigenTT type | The EigenTT type of the extracted payload. May be a primitive (`Float`), a tuple, a record, an inductive type. |
| `institution_ref` | IRI of an Institution | Declaring institution. |
| `procedure` | IRI | The dispatch key the institution's `extract_typed` handler receives (§8). |

A class can have multiple ExportFormats. `DockingResult` might publish three: one extracting just ΔG as a `Float`, one extracting the thermodynamic decomposition as a `(Float, Float, Float)`, one extracting the full conformer information as an inductive value.

### 4.3 `ImportFormat`

The dual: a typed *inbound* constructor that lifts a EigenTT payload into a target-class resource.

| Property | Type | Meaning |
|---|---|---|
| `to_class` | IRI of a Class | The resource class this format constructs. |
| `payload_type` | IRI of a EigenTT type | The EigenTT type of the input payload. |
| `institution_ref` | IRI of an Institution | Declaring institution. |
| `procedure` | IRI | The dispatch key the institution's `reify` handler receives. |

A class can have multiple ImportFormats. `AssayPrediction` might accept just `Float` (an IC₅₀) or `(Float, Float)` (IC₅₀ with confidence interval).

### 4.4 `QueryClass`

A typed function in the institution's fibre — input and output resource classes plus an implementation. Subsumes the prior `FiberQuery`, decide-procedure, and `validate_morphism`-bound predicate notions; the difference between them is the *dispatch role*, not the primitive.

| Property | Type | Meaning |
|---|---|---|
| `query_class` | IRI of a Class | The class of query *input* resources. The kernel dispatches on this class IRI. |
| `result_class` | IRI of a Class | The class of result resources the query produces. |
| `dispatch_role` | enum | One of `OnDemand` (explicit FIBER / RPC), `AutoOnLoad` (fired automatically when a resource of `query_class` enters the chain — replaces `validate_morphism`), `Decidable` (referenced from `Exp::NativeDecide`; replaces `decide`). A single QueryClass may declare multiple roles. |
| `implementation` | IRI | Either a EigenTT Component IRI (the kernel handles the call entirely via extract → component → reify) or a procedure IRI dispatched to the institution's `query` handler. |
| `institution_ref` | IRI of an Institution | Declaring institution. |

When `dispatch_role` includes `AutoOnLoad`, `result_class` must be a `Verdict` shape (§6.1): the kernel uses the verdict to gate the Load.

When `dispatch_role` includes `Decidable`, `result_class` must again be a `Verdict`: the kernel reduces `Exp::NativeDecide` accordingly (Holds → `Refl(v)`, Fails → failing neutral, Undecidable → passthrough).

### 4.5 `Comorphism`

The triadic translation across an institution boundary (§5). Owned by no single institution; aggregates contributions from source, EigenTT, and target.

| Property | Type | Meaning |
|---|---|---|
| `export_format` | IRI of an ExportFormat | The source-side $s$ — extracts a typed payload from a source-class resource. |
| `transformation` | IRI of a EigenTT expression (typed term) | The middle $m: S \to T$, where $S = \mathrm{payload\_type}(\mathrm{export\_format})$ and $T = \mathrm{payload\_type}(\mathrm{import\_format})$. **The transformation is a EigenTT *term*, not an opaque Component.** The natural shape is a Lambda whose body is a typed expression — for pure transformations the body is fully transparent (e.g. `λ Δg. exp(-Δg / RT) * 1e9` for Arrhenius); for institution-runtime transformations the body bottoms at a `program:Component` reference (an expression form), which the kernel evaluates by dispatching into the institution's worker. Either way the transformation slot carries an inspectable, type-checkable, composable typed term — the property that lets the kernel reason about $m$ rather than treating it as a black box. |
| `import_format` | IRI of an ImportFormat | The target-side $t$ — constructs a target-class resource from a typed payload. |
| `exact` | boolean | Whether the comorphism preserves model amalgamation in the sense of Diaconescu (2025, Thm. 14.15 + Prop. 14.14). Absent or `false` is the safe default; only an explicit `true` is a claim of exactness. |

`source_institution` and `target_institution` are derivable from `export_format.institution_ref` and `import_format.institution_ref`; the kernel may index them but the Comorphism resource does not need to repeat them.

The kernel statically type-checks a Comorphism resource at commit time: the transformation term's type must equal `(payload_type(export_format)) → (payload_type(import_format))`. A type-incorrect Comorphism is rejected by structural validation — comorphism well-typedness is a kernel-level invariant rather than a runtime hope.

---

## 5. The triadic realisation of a comorphism

A comorphism in our system is the triple

$$\rho \;=\; (s,\ m,\ t)$$

where:

- **$s$** is the *source* institution's typed extraction, declared as an ExportFormat. $s$ takes a resource of class $C_S$ (in the source institution's vocabulary) and returns a EigenTT value of type $S$. The source institution is the only party that knows how to traverse $C_S$'s representational invariants.
- **$m$** is a *cross-institution* transformation, declared as a EigenTT term (typed expression) with type $S \to T$. $m$ owns the actual mathematical content of the comorphism. It belongs to neither institution; it is the bridge. The term is evaluable through the kernel's existing Component infrastructure — Lambda body reduction for pure transformations, Component-as-expression-form dispatch at the leaf for institution-runtime transformations — so the same evaluation path covers both.
- **$t$** is the *target* institution's typed reification, declared as an ImportFormat. $t$ takes a EigenTT value of type $T$ and constructs a resource of class $C_T$ (in the target institution's vocabulary). The target institution is the only party that knows how to construct well-formed $C_T$ instances.

Reading the comorphism this way:

- **Each institution stays inside its own representational concerns.** The source publishes its typed views; the target publishes its typed constructors. Neither knows the other's resource shape directly.
- **The cross-institution mathematics is expressible in EigenTT.** $m$ is an ordinary typed term, evaluable through the kernel's existing Component infrastructure, sharable across multiple comorphisms (the same `λ Δg. exp(-Δg / RT)` could underpin both ρ_{Dock→Assay} and a hypothetical ρ_{Dock→ITC}), and amenable to proof when the comorphism is exact.
- **The Satisfaction Condition becomes a typing obligation.** Diaconescu's $\rho^{\mathrm{Sen}}$ (sentences forward) is realised by the composition $s ; m ; t$: the kernel verifies at commit time that $m$'s signature matches the export's payload type and the import's payload type, so the three pieces compose by construction. The implicit $\rho^{\mathrm{Mod}}$ (models backward) is the unstated correspondence "the same molecule, viewed through different observational lenses" — never materialised as a kernel verb.
- **Exactness is a property of $m$.** A comorphism is exact when $m$ is provably correct in some appropriate sense — for instance, when $m$'s EigenTT term carries a derivation witnessing that the source-side claim's truth implies the target-side claim's truth. The current kernel does not enforce such derivations (Phase 12-era plumbing), but the structural shape supports them. An inexact comorphism (deliberate approximation, e.g. $IC_{50} \approx \exp(-\Delta G / RT)$ ignoring entropy and solvent effects) is honestly marked as such; cross-institution query results that traverse it carry provenance recording the loss of amalgamation guarantees.

### 5.1 Worked example — ρ_{Dock→Assay}

Concrete instantiation:

```
ExportFormat ef_dock_to_dg
  from_class: DockingResult
  payload_type: Float
  institution_ref: dock
  procedure: urn:eigenius:dock:extract_dg

ImportFormat if_assay_from_ic50
  to_class: AssayPrediction
  payload_type: Float
  institution_ref: assay
  procedure: urn:eigenius:assay:reify_ic50

Component cm_arrhenius
  type: Float -> Float
  body: λ dg. exp(-dg / (R * T))

Comorphism ρ_dock_to_assay
  export_format: ef_dock_to_dg
  transformation: cm_arrhenius
  import_format: if_assay_from_ic50
  exact: false
  description: "Approximation: IC₅₀ ≈ exp(-ΔG / RT). Ignores entropic/solvent contributions."
```

A future ρ_{Dock→PK} could share `ef_dock_to_dg` (extracting the same ΔG payload) with a different transformation Component and a PK-side ImportFormat, without touching either the dock or assay institution's WASM code.

### 5.2 No path composition

The kernel deliberately does not close the comorphism set under composition. If $\rho_{A \to B}$ and $\rho_{B \to C}$ are declared, the registry does **not** synthesise a $\rho_{A \to C}$ by composition.

The reason is in Diaconescu (2025, Fact 14.9 and surrounding text): in general, composing left adjoints corresponding to two index morphisms yields a translation only *isomorphic to* — not equal to — the left adjoint of the composed index morphism. Identifying these programmatically is a Satisfaction Condition violation in disguise. The chapter highlights `fol` as a concrete example of an indexed institution that fails to be coherent for exactly this reason.

If a user needs $\rho_{A \to C}$, they declare it explicitly as a separate Comorphism resource with its own export / transformation / import. The kernel matches comorphism dispatch by exact IRI only.

---

## 6. Query as the universal reasoning primitive

`validate_morphism`, `decide`, `query`, and `discover_morphisms` collapse into one primitive: **a function from one object in the institution's category to another**, dispatched by the input class IRI and producing a typed result resource.

The four operational profiles fall out as different `dispatch_role` settings on a `QueryClass` declaration:

| Old name | New shape |
|---|---|
| `validate_morphism` | A `QueryClass` whose `dispatch_role` includes `AutoOnLoad`, `query_class` is the morphism class to be validated, and `result_class` is the `Verdict` shape. The kernel fires it automatically when a matching resource enters the chain. |
| `decide` (predicate dispatch) | A `QueryClass` whose `dispatch_role` includes `Decidable`, with `result_class` again `Verdict`. The kernel fires it when reducing `Exp::NativeDecide`. |
| `query` (FIBER / RPC) | A `QueryClass` whose `dispatch_role` includes `OnDemand`. The kernel fires it from EigenQL FIBER clauses or `RunFiberQuery` RPC. `result_class` is whatever the institution wants to return. |
| `discover_morphisms` | A `QueryClass` whose `result_class` is a list of morphism resources — a regular `OnDemand` query. The kernel does not need a separate primitive. |

A single `QueryClass` may declare multiple roles. A predicate that is auto-fired on Load *and* available for `NativeDecide` reduction *and* invokable from EigenQL just lists all three.

### 6.1 Verdict shape

`Verdict` is a small inductive type (or a result class with three inhabitants — `Holds`, `Fails`, `Undecidable`) used by all role-typed queries that need a tri-state pass/fail/defer outcome:

- `Holds` — the kernel reduces the surrounding `NativeDecide` to `Refl(v)`, or accepts the resource on Load.
- `Fails` — the kernel emits a failing neutral, or rejects the resource on Load.
- `Undecidable` — the kernel leaves the constraint as a passthrough, or accepts the resource on Load with no domain commitment.

### 6.2 EigenTT-implemented vs. institution-implemented queries

A `QueryClass` declares its `implementation` as either a EigenTT Component IRI or a procedure IRI dispatched to the institution's runtime. The kernel orchestrates accordingly:

- **Component-implemented**: the kernel extracts the typed payload from the input resource via the matching ExportFormat, applies the Component, reifies the result via the matching ImportFormat. The institution's runtime is never called for this query class.
- **Institution-implemented**: the kernel calls the institution's `query` trait method (§8) with the input resource and the procedure IRI. The institution returns a result resource directly.

Institutions whose reasoning is expressible in EigenTT (Pareto-dominance checks, threshold comparisons, simple arithmetic on numeric fields) can declare every QueryClass as Component-implemented and implement no `query` handler at all. Institutions whose reasoning lives in opaque code (a docking-pose generator, an LLM, a Lean 4 server) implement `query`. The trait method is the *escape hatch* for non-EigenTT reasoning.

---

## 7. Epistemic status and institutions

Eigenius distinguishes four epistemic categories for resources in the layer chain. Institutions interact with these categories without owning them — the categories are kernel-level provenance, the institutions provide the reasoning that promotes resources between them.

### 7.1 Epistemic categories

| Status | What determines it | Institution involvement |
|---|---|---|
| **Declared** | Resource committed by human assertion | None. |
| **Observed** | Resource imported from external source with provenance | None. |
| **Derived** | Resource produced by program execution with reasoning traces | The institutions whose QueryClasses / Comorphisms ran during execution are recorded in the trace. |
| **Verified** | Resource carries a formal proof from a verification institution | A verification institution (e.g. Lean 4) accepted the resource via an `AutoOnLoad` QueryClass returning `Holds`, attaching a proof witness as the trace. |

These statuses form a partial order: a resource gains the strongest status whose criteria it meets. Promotion from derived → verified requires an explicit verification step; the kernel does not silently upgrade.

### 7.2 Lean 4 as a verification institution

Lean 4 is an external CIC reasoning system — *not* EigenTT (which is the kernel's own kernel-level type checker). It is a separate, more powerful type theory exposed to Eigenius as an institution.

Lean 4 declares (sketch):

- A class `LeanProofTerm` carrying a Lean 4 proof term and the proposition it proves.
- An `ExportFormat` extracting the proof term + proposition as a EigenTT-side typed payload.
- A `QueryClass` `ProofCheck` with `dispatch_role: AutoOnLoad`, bound to `LeanProofTerm`, returning `Verdict`. The implementation is institution-runtime: the institution's `query` handler dispatches the proof term to a Lean 4 server, accepts the verdict back, and returns `Holds` / `Fails` / `Undecidable`.
- Optionally: a `QueryClass` `ProofSearch` with `dispatch_role: OnDemand`, taking a proposition resource and returning an inhabited `LeanProofTerm` if Lean finds a proof.

The verification flow:

1. A program produces a derived result with a ProgramTrace.
2. A verification step constructs a Lean 4 proof term that the result is correct (this may itself be a separate program; Lean 4 is invoked as a tool).
3. The `LeanProofTerm` resource is loaded; the `ProofCheck` QueryClass auto-fires; the proof term goes to the Lean server.
4. If the Lean server accepts, the kernel attaches the proof term as the resource's reasoning trace and promotes the resource's epistemic status from derived → verified.
5. If Lean rejects, Load fails; the kernel surfaces the typed error and the resource never enters the chain.

EigenTT cannot do this: it checks program *composition* (well-typed reductions in our small CIC fragment), not domain *correctness* (arbitrary mathematical propositions). Lean 4 can express and check arbitrary mathematical propositions about the domain — that is the entire reason it is useful as an institution rather than as a built-in kernel feature.

---

## 8. The trait surface

Three methods. The institution's only mandatory responsibility is boundary translation; reasoning is optional.

```rust
pub trait Institution: Send + Sync {
    /// The institution's IRI. Used for registry indexing.
    fn institution_iri(&self) -> &Iri;

    /// Boundary: extract a typed EigenTT value from a resource via a
    /// procedure declared by an `ExportFormat` resource owned by this
    /// institution. Always required.
    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError>;

    /// Boundary: construct a target-class resource from a typed value
    /// via a procedure declared by an `ImportFormat` resource owned by
    /// this institution. Always required.
    fn reify(
        &self,
        procedure_iri: &Iri,
        value: &Val,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;

    /// Apply an institution-defined query — input resource of one
    /// class, output resource of another. Subsumes the prior
    /// `validate_morphism` / `decide` / `query` / `discover_morphisms`
    /// trichotomy; dispatch role is determined by the QueryClass
    /// declaration, not the trait method.
    ///
    /// Optional: the default implementation returns
    /// `InstitutionError::NotImplemented`. Institutions whose
    /// QueryClasses are all Component-implemented never see this
    /// method called and need not override it.
    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        let _ = (procedure_iri, input, ctx);
        Err(InstitutionError::NotImplemented(
            "this institution has no institution-runtime queries".into(),
        ))
    }
}
```

`procedure_iri` is the QueryClass-declared procedure key (when `dispatch_role` is `OnDemand`), the bound morphism-class IRI (when `AutoOnLoad`), or the constraint IRI (when `Decidable`). The kernel passes whichever is applicable; the institution can dispatch on the IRI to choose its handler.

---

## 9. The dispatch model

The kernel maintains a derived **InstitutionRegistry** built by scanning the layer chain for ExportFormat / ImportFormat / QueryClass / Comorphism resources (and the institutions that declare them). The registry is rebuilt on:

- Initial bootstrap (chain known at startup).
- Phase 9a rehydration from a persistent backend.
- Successful Load commit (newly committed declarations enter the index).

The registry indexes:

- procedure IRI → (institution IRI, procedure kind: extract / reify / query) — for boundary and on-demand dispatch
- query class IRI → list of QueryClass declarations that match — for auto-on-load and explicit-FIBER dispatch
- constraint IRI → QueryClass declaration with `Decidable` role — for `NativeDecide` dispatch
- comorphism IRI → (export_format IRI, transformation Component IRI, import_format IRI) — for `Exp::InstitutionInvoke` dispatch

### 9.1 What the kernel does on Load

For each newly committed resource whose class has at least one `QueryClass` with `dispatch_role` including `AutoOnLoad`:

1. Resolve the QueryClass.
2. If the implementation is a Component: extract via the relevant ExportFormat, apply the Component, read off the Verdict.
3. If the implementation is institution-runtime: call `Institution::query(procedure_iri, resource, ctx)`, parse the returned resource as a Verdict.
4. Apply the Verdict: `Holds` and `Undecidable` accept; `Fails` aborts the Load with a typed validation error.

Structural validation runs first, as before; institution-aware QueryClass dispatch fires on top.

### 9.2 What the kernel does on `Exp::NativeDecide` reduction

Given `NativeDecide(Constraint::Institution { iri, args }, v)`:

1. Resolve the QueryClass declaring `iri` with `dispatch_role` including `Decidable`.
2. Marshal `args` into a synthetic input resource (the QueryClass declaration carries the input class).
3. Run the QueryClass implementation (Component or institution-runtime).
4. Apply the Verdict: `Holds` reduces to `Refl(v)`; `Fails` produces a failing neutral; `Undecidable` leaves the constraint as a passthrough.

### 9.3 What the kernel does on `Exp::InstitutionInvoke`

Given `Exp::InstitutionInvoke { comorphism_iri, source, target_iri }`:

1. Resolve the Comorphism resource from the registry.
2. Look up the source institution from `export_format.institution_ref`. Call `extract_typed(export_format.procedure, source, ctx)` → typed value $v_S$ of type $S$.
3. Look up the transformation Component. Apply it to $v_S$ → typed value $v_T$ of type $T$. Component application uses the existing kernel evaluator.
4. Look up the target institution from `import_format.institution_ref`. Call `reify(import_format.procedure, v_T, ctx)` → target-class resource. Assign the produced resource a chain-resident `@id`: the optional `target_iri` (set by surface-language overrides such as EigenQL `INTO`) takes precedence; otherwise the kernel mints a deterministic content-hash IRI of the form `urn:eigenius:comorphism-output:<comorphism-tail>:<hex>` over the canonical Eigon-CBOR of the resource (with `@id` cleared). Identical reify outputs collide on the same IRI, which is the cross-fibre dedup property the Grothendieck construction wants.
5. **Post-translation validation invariant.** Run any `AutoOnLoad` QueryClasses bound to the target class on the produced resource. Failure surfaces as a typed `EvalError::ComorphismProducedInvalidResource` — this distinguishes a comorphism implementation bug from a user error, and operationalises the Satisfaction Condition (§5) as a runtime invariant.
6. **Reinsert the produced resource into the chain.** The IRI'd resource is collected at the run-boundary and committed to the program-run layer alongside the trace artefacts via [`commit_with_validation`](../../kernel/src/context/mod.rs), so AutoOnLoad fires *as part of* normal chain entry. The resource is then addressable to downstream EigenQL, Inspect, and component-side `resolve` — comorphism-translated sentences are first-class chain residents, not transport-only values. The same elevation applies to a program's final-step Resource value (under namespace `urn:eigenius:program-output:<program-tail>:<hex>`); a `RunProgramResponse.output_resource_iris` field exposes the assigned IRIs to clients.

### 9.4 What the kernel does on EigenQL FIBER

EigenQL morphism resources are queryable as ordinary Resources — no language change is needed. A FIBER clause in addition references a `QueryClass` with `dispatch_role` including `OnDemand`:

```eigenql
USING "urn:eigenius:fea:MeshRefinement",
      "urn:eigenius:fea:StressResult"
MATCH MeshRefinement(?m) {
    source: ?s1, target: ?s2,
    convergence_delta: ?delta
},
StressResult(?s2) { safety_factor: ?sf }
WHERE ?delta > 0.05
RETURN [] { result: ?s2, factor: ?sf, delta: ?delta }
```

For queries requiring institutional reasoning (e.g. "is this mesh converged?"), the FIBER clause names a `QueryClass`; the evaluator resolves the QueryClass, runs its implementation (Component-orchestrated or institution-runtime), and binds the resulting resource for downstream pattern matching.

A FIBER clause may carry an optional `INTO "<iri>"` suffix that pins the response in the regular chain:

```eigenql
FIBER assay:validate_prediction {
    candidate: dock_to_assay(?d)
} AS ?v INTO "urn:eigenius:my:validation_run_42"
```

Without `INTO`, the response lives in the per-query overlay and disappears at query end (no chain entry, no AutoOnLoad firing — the query is read-only). With `INTO`, the response stamps the named IRI, the query's commit cycle lifts it through `commit_with_validation` so AutoOnLoad QueryClasses bound to its class fire on chain entry, and the IRI surfaces back to the caller via `QueryResponse.output_resource_iris`. This is the EigenQL surface for §9.3 chain reinsertion — paralleling what `Exp::InstitutionInvoke`'s `target_iri: Option<Iri>` does for ESL / programmatic dispatch.

### 9.5 IRI classification at parse time

Phase 11e established that surface-language compilers (ESL, EigenQL) classify a function-call-shaped IRI at compile time to choose the right kernel AST node. Under D14 the classification table recognises three institution-level kinds:

- `Comorphism` — emit `Exp::InstitutionInvoke { comorphism_iri, source }`. Looked up in the registry's comorphism index.
- `Predicate` — emit `Exp::NativeDecide(Constraint::Institution { iri, args }, _)`. Looked up in the registry's `Decidable` QueryClass index.
- `QueryClass` (OnDemand) — emit a fiber-query call. Looked up in the registry's OnDemand QueryClass index.

Anything else falls through to non-institution lookups (component registry, class constructor, unbound variable) — same as today.

---

## 10. Categorical grounding

§2.1 commits to the comorphism-based variant. Concretely realised in our system:

- **$\rho^{\mathrm{Sign}}$** — the implicit signature mapping: `from_class` (source-side class) maps to `to_class` (target-side class) under a comorphism. The "category of indices" is the institution registry.
- **$\rho^{\mathrm{Sen}}$** — the composition $s; m; t$. A source-side sentence (resource) is translated through extraction, EigenTT transformation, and target reification into a target-side sentence. Type-checked at commit; runtime-checked by the post-translation invariant.
- **$\rho^{\mathrm{Mod}}$** — implicit. The "models" $M$, $M'$ are the ground-truth worlds in which our sentences claim things; the kernel never represents them directly. A model-backward reading is "the same molecule observed through the source's instruments versus the target's instruments." We do not need to evaluate it.

The Satisfaction Condition for a comorphism is

$$M' \models'_{\rho\Sigma} \rho^{\mathrm{Sen}}_\Sigma(\varphi) \iff \rho^{\mathrm{Mod}}_\Sigma(M') \models_\Sigma \varphi$$

Operationally:

- Compile-time: well-typedness of $m$ ensures the three-piece composition is well-defined.
- Run-time: post-translation validation (§9.3 step 5) ensures the translated sentence is well-formed in the target institution.

We cannot check the *truth* of either side (that would require representing $M, M'$); we *can* check the structural compatibility that the Satisfaction Condition demands the institution-theoretic mapping preserve. That is the strongest enforceable form of the condition in a system that does not represent models.

---

## 11. Reasoning depth

D14 specifies the *protocol* — the typed dispatch surface, the boundary contract, the well-typedness invariants. It does not specify the *theorems* the institutions reason about. There are two distinguishable levels of demonstration this design supports:

**Plumbing demos.** Institutions whose reasoning is shallow numeric / threshold logic. The QueryClass implementations are simple EigenTT Components or short institution-runtime checks; comorphisms are Float-to-Float transformations. The protocol round-trips data correctly through typed dispatch. Examples: refinement-delta thresholding, Pareto dominance comparison, replicate-CV ceiling, $IC_{50} \approx \exp(-\Delta G / RT)$ approximation.

**Deep demos.** Institutions whose QueryClasses or comorphism transformations carry actual derivations: a Hill-equation fit witnessed in EigenTT, a refinement-chain Cauchy proof, a compartmental-model identifiability witness, a Lean 4 proof of binding-mode optimality. The transformation Component is no longer a one-line lambda; it is a typed term whose body is a derivation.

The first crop of demos under D14 will be plumbing-only — landing the typed dispatch surface is necessary before reasoning content can ride on it. The published narratives (the worked-examples paper, the platform-guide chapters) should be honest about which level they exhibit. One demo eventually goes deep; the others stay plumbing showcases.

This is a deliberate scope choice, not a limitation of the protocol.

---

## 12. WIT world and SDK shape

The `eigenius-institution` WIT world exports the boundary methods plus the optional reasoning method:

```wit
world eigenius-institution {
    import read-access;
    import query-access;

    export extract-typed: func(
        procedure-iri: iri,
        resource: resource-data,
    ) -> result<typed-value, string>;

    export reify: func(
        procedure-iri: iri,
        value: typed-value,
    ) -> result<resource-data, string>;

    /// Optional: institutions whose QueryClasses are all
    /// Component-implemented may export this as a stub returning
    /// `not-implemented`.
    export query: func(
        procedure-iri: iri,
        input: resource-data,
    ) -> result<resource-data, string>;
}
```

`typed-value` is a CBOR-encoded marshalling of a EigenTT value; the SDK provides round-trip helpers between Rust types and `typed-value`.

The SDK provides resource builders for `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, and `Comorphism` resources (so guests can construct their declaration documents in idiomatic Rust). It does *not* provide a `FiberDeclaration` struct — declarations are ordinary typed Resources, not a special metadata type.

---

## 13. Migration plan

The current institution implementation is removed wholesale and rebuilt. A fresh `kernel/src/institution/` is preferred over an in-place edit because the trait surface, registry shape, and dispatch model all change.

### 13.1 Removed

- `kernel/src/institution/comorphism.rs` (already-dead Rust trait + registry).
- `kernel/src/institution/mod.rs::FiberReasoner` trait — replaced by `Institution`.
- `kernel/src/institution/mod.rs::FiberDeclaration` struct — replaced by ontology resource builders.
- `kernel/src/institution/mod.rs::DecResult` — folded into the Verdict shape.
- `kernel/src/institution/mod.rs::InstitutionCapability` — folded into `dispatch_role`.
- `sdk/wasm-sdk/src/institution.rs::FiberDeclaration` — replaced by per-resource builders.
- `wit/eigenius-component.wit` `eigenius-institution` world — replaced with the §12 shape.
- `kernel/src/capability/wasm_institution.rs` — host-side `WasmFiberReasoner` rewritten against the new trait.
- `kernel/src/validation/mod.rs::validate_with_institutions` — replaced by the §9.1 dispatch.

### 13.2 Added

- `ontologies/institution/institution-ontology.json` — the five resource shapes from §4.
- `kernel/src/ontology/well_known.rs` — constants for the new IRIs.
- `kernel/src/institution/mod.rs::Institution` trait — the §8 surface.
- Kernel-side type-checker for Comorphism resources (commit-time well-typedness check; §4.5 / §5).
- `Verdict` inductive type or core class.
- `kernel/src/institution/registry.rs` — derived registry built from chain scan.

### 13.3 Reused

- `kernel/src/program/expr.rs::Exp::InstitutionInvoke` — same AST node, new evaluator (§9.3).
- `Exp::NativeDecide` — same dispatch path, simplified (§9.2).
- The `EigenQL` FIBER clause grammar — unchanged surface; new dispatch under the hood.
- The Phase 7 Component infrastructure for the EigenTT middle of comorphisms (no new mechanism).
- Phase 9a rehydration walking the chain — the new registry consumes its output.

### 13.4 Implementation milestones

A separate plan document will sequence the milestones; the rough shape:

- **M1**: ontology shape + well-known IRIs + Verdict type. No code dispatch yet. Doc + ontology only.
- **M2**: derived registry (chain scan, indexing).
- **M3**: `Institution` trait, in-process dispatch, kernel-side type-checking of Comorphisms.
- **M4**: WIT world + SDK update. Existing WASM ordering institution rewritten.
- **M5**: `Exp::InstitutionInvoke` evaluator on the new shape (the four-step pipeline).
- **M6**: `Exp::NativeDecide` dispatch on the new shape.
- **M7**: AutoOnLoad dispatch in the Load path.
- **M8**: A worked-example demo that exercises the full surface (plumbing-only, single comorphism, single typed component middle, end-to-end test).

Each milestone is independently reviewable and committable.

---

## 14. Open questions

1. **Where does $m$ live for a comorphism?** Component IRI (kernel-registered, sharable, leverages Phase 7 dispatch) versus inline `Exp` term on the Comorphism resource (self-contained, no extra registration). I lean toward Component IRI; it reuses existing infrastructure and keeps comorphisms separable from their transformations.

2. **Single-artefact vs. dual-artefact shipping for institutions.** The WASM binary returning a declaration document at install is the pragmatic choice and matches the existing pipeline. Separate `*.wasm` + `*.eigon-json` is conceptually cleaner — code and ontology decouple, ontology can evolve independently. Default to single-artefact for now; treat the kernel as receiving the declaration *as ordinary chain data* so the migration to dual-artefact later is a packaging change, not an architectural one.

3. **Verdict as core class vs. inductive type.** A `Verdict` core class with `Holds`, `Fails`, `Undecidable` instances mirrors how other tri-state outcomes appear in the chain. A `Verdict` inductive type lives in EigenTT and is a true sum type. Component-implemented QueryClasses naturally produce inductive-type values; institution-runtime QueryClasses naturally produce class-instance resources. We need to settle which is canonical and provide a coercion at the boundary.

4. **Multiple ExportFormats per source class with overlapping payload types.** If `DockingResult` declares both `(DockingResult → Float)` (just ΔG) and `(DockingResult → (Float, Float, Float))` (thermodynamic decomposition), and a comorphism wants `Float`, which ExportFormat applies? Either (a) the Comorphism resource references the specific ExportFormat by IRI (no ambiguity), or (b) the Comorphism declares the source class and payload type, and the kernel selects. (a) is simpler and is what §4.5 currently specifies; (b) might be friendlier ergonomically. Default to (a) until we have a use case for (b).

5. **Reasoning-depth roadmap.** Which demo eventually carries real domain derivations rather than threshold checks? The biopharma example is a natural candidate (the chemistry has actual derivations to encode); the mechanical-engineering example would stay plumbing-only. This is a planning question, not a protocol question, but worth flagging here.

---

## 15. References

- D1 — Eigon serialisation format and the 12 structural validation rules.
- D9 — NbE / EigenTT. The kernel's type theory used as the cross-institution middle.
- D12 — WASM extensibility. The runtime mechanism for institution code.
- D19 — Inductive types. Used for the Verdict shape and arbitrary typed payloads.
- Diaconescu, R. *Institution-independent Model Theory*, 2nd ed., Studies in Universal Logic, Springer 2025 (`diaconescu2025`). Chapter 14 is the canonical reference for the comorphism-based Grothendieck construction. Specifically: §14.1 (the construction), §14.2 (theory co-limits and model amalgamation, including Thm. 14.15 on semi-exactness), §14.3 (interpolation).
- Diaconescu, R. (2002), "Grothendieck institutions", *Applied Categorical Structures* (`diaconescu2002`) — the original.
- Goguen, J. A. and Burstall, R. M. (1992), "Institutions: Abstract model theory for specification and programming", *JACM* (`goguen1992`) — the underlying institution notion.
- [`docs/papers/eigenius-institutions.tex`](../papers/eigenius-institutions.tex) — the high-level published narrative; D14 is the implementation contract for it.
- Spivak, *Functorial Data Migration* — the typed-instance-translation framework whose Σ / Δ / Π adjoints we *don't* directly map onto our `translate` (their adjoints act on instances/models; ours acts on sentences). Cited for context on why the analogy breaks down.
