# D10: Grothendieck Institution Protocol

*Design document for the Eigenius project — April 2026*

**Status:** Draft
**Required before:** Phase 6 implementation
**Depends on:** D6b (reasoning traces), D9 (NbE unification), eigenius-institutions.tex (theoretical foundation)

---

## 1. Overview

Eigenius's capability protocol implements a Grothendieck institution: multiple logical systems share a typed knowledge graph, with each institution contributing structured fibers — categories of results with their own internal morphisms — rather than flat sets of data points.

This document specifies the concrete protocol by which domain institutions register with the kernel, declare their fiber structure, and participate in reasoning. It bridges the category-theoretic framework from `eigenius-institutions.tex` with Rust traits and Eigon resources.

### 1.1 What Is Not an Institution

The kernel itself is not an institution. Two kernel services provide the fixed foundation:

- **Eigon structural validator** — the 12 validation rules from D1 §5.4. Checks resource well-formedness against class definitions. Fixed, trusted, not extensible via the institution protocol.
- **Mini-TT type checker** — the NbE-based CIC type checker from D9. Checks program composition, ground types, capability levels. Fixed, trusted, part of the kernel.

These are the switchboard — they validate and dispatch. Institutions provide domain-specific reasoning that the kernel mediates.

### 1.2 What Is an Institution

An institution is a domain-specific reasoning system that:
- Has its own notion of well-formedness (satisfaction relation)
- Produces results with internal structure (morphisms within fibers)
- Can answer queries about its own results (fiber reasoning)
- Registers with the kernel via the capability protocol

Examples: FEA stress analysis, molecular docking, ADMET prediction, Lean 4 formal verification, LLM-based extraction.

### 1.3 Eigon as the Shared Signature Category

Eigon is *neither* the kernel *nor* an institution. It is the **shared signature category** — the base over which the Grothendieck construction is performed.

In institution-theoretic terms:
- The **category of Eigon signatures** has ontology snapshots (layer stack configurations) as objects and layer extensions as morphisms.
- The **base satisfaction condition** is the 12 structural validation rules — they define what it means for a resource to be well-formed against its class definition.
- Every institution builds **over** this base. Institution-specific sentences, models, and satisfaction relations add domain-specific structure to the shared Eigon foundation.

This is not a design choice — it's a mathematical necessity. The Grothendieck construction requires a base category. Eigon is that base. Without it, there is no shared language for institutions to register their types, declare their morphisms, or exchange data. Making Eigon an institution would require a meta-Eigon for institutions to register with, leading to infinite regress.

Concretely:
- **Resources, IRIs, properties, layers** — the Eigon data model is the kernel's native representation. It cannot be replaced or made optional.
- **The 12 validation rules** — structural satisfaction over the Eigon base. Fixed, not extensible.
- **Mini-TT** — type checking of programs over Eigon ground types. A kernel service operating on the base.
- **Institutions** — domain-specific reasoning systems that register typed morphisms, queries, and satisfaction relations *as Eigon resources*. They build fibers over the base, not alternatives to it.

The relationship: Eigon provides the shared language. Institutions provide the domain-specific meaning. Neither can function without the other. The Grothendieck construction glues them together.

---

## 2. Institution Theory Mapping

From Goguen and Burstall (1992), an institution $\mathcal{I} = (\mathrm{Sign}, \mathrm{Sen}, \mathrm{Mod}, \models)$ consists of:

| Formal | Eigenius Concrete |
|--------|-------------------|
| $\mathrm{Sign}$ — category of signatures | Eigon ontology snapshots (the shared signature category) |
| $\mathrm{Sen}(\Sigma)$ — sentences over signature $\Sigma$ | Typed constraints and queries expressible in the institution's logic |
| $\mathrm{Mod}(\Sigma)$ — category of models over $\Sigma$ | Resources in the knowledge graph satisfying the institution's constraints, with morphisms |
| $\models_\Sigma$ — satisfaction relation | The institution's validation/checking logic (on top of Eigon's base satisfaction) |

The move from flat fibers ($\mathrm{Mod}(\Sigma) = \mathrm{Set}$) to structured fibers ($\mathrm{Mod}(\Sigma) = \mathrm{Cat}$) is the Grothendieck construction. Each institution contributes a *category* of models — objects with morphisms between them — not just a set of data points.

---

## 3. The FiberReasoner Trait

### 3.1 Trait Definition

```rust
pub trait FiberReasoner: Send + Sync {
    /// Declare this institution's fiber structure.
    /// Called once at registration time.
    fn fiber_declaration(&self) -> FiberDeclaration;

    /// Execute a fiber query.
    /// The query is a typed Eigon resource (subclass of FiberQuery).
    /// Returns a typed result resource.
    fn query(
        &self,
        query: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;

    /// Validate a claimed morphism against the institution's domain logic.
    /// Structural validation (required properties, types) is the kernel's job.
    /// This checks domain-specific validity.
    fn validate_morphism(
        &self,
        morphism: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<MorphismValidation, InstitutionError>;

    /// Discover morphisms not yet in the knowledge graph.
    /// Given resources in this institution's fiber, infer morphisms.
    /// Returns resources; the caller decides whether to commit them.
    fn discover_morphisms(
        &self,
        resources: &[Resource],
        ctx: &ExecutionContext,
    ) -> Result<Vec<Resource>, InstitutionError>;
}
```

### 3.2 FiberDeclaration

```rust
pub struct FiberDeclaration {
    /// The institution's IRI (e.g., "urn:eigenius:institutions:fea")
    pub institution_iri: Iri,

    /// Human-readable name
    pub name: String,

    /// Morphism classes this institution defines.
    /// Each is a Class resource with source_type, target_type, and properties.
    /// E.g., MeshRefinement with source: StressResult, target: StressResult,
    /// convergence_delta: float.
    pub morphism_types: Vec<Resource>,

    /// FiberQuery subclasses this institution can answer.
    /// E.g., ConvergenceQuery, EnsembleQuery.
    pub query_types: Vec<Resource>,

    /// Advisory structural properties of morphisms.
    /// E.g., "MeshRefinement is a partial order."
    /// The kernel stores these but does not enforce them.
    /// The institution enforces its own properties via validate_morphism.
    pub structural_properties: Vec<Resource>,
}
```

### 3.3 MorphismValidation

```rust
pub enum MorphismValidation {
    /// The morphism is valid according to the institution's domain logic.
    Valid,
    /// The morphism is invalid, with a reason.
    Invalid(String),
    /// The institution cannot determine validity (e.g., requires external computation).
    Undecidable,
}
```

### 3.4 InstitutionError

```rust
pub enum InstitutionError {
    /// The query/morphism type is not recognized by this institution.
    UnknownType(String),
    /// Internal computation error.
    ComputationFailed(String),
    /// The institution requires resources not available in the context.
    MissingDependency(String),
}
```

---

## 4. Institution Registration

### 4.1 Registration Flow

1. The institution implementation (a `Box<dyn FiberReasoner>`) is registered with the kernel.
2. The kernel calls `fiber_declaration()` to get the institution's metadata.
3. The kernel commits the morphism types, query types, and structural properties as ontology resources in a new layer.
4. The kernel records the institution IRI → FiberReasoner mapping in an internal registry (analogous to `ComponentRegistry`).

```rust
pub struct InstitutionRegistry {
    institutions: BTreeMap<Iri, Box<dyn FiberReasoner>>,
}

impl InstitutionRegistry {
    pub fn register(&mut self, reasoner: Box<dyn FiberReasoner>) -> Result<Vec<Resource>, String> {
        let decl = reasoner.fiber_declaration();
        let iri = decl.institution_iri.clone();

        // Collect ontology resources to commit
        let mut resources = Vec::new();
        resources.extend(decl.morphism_types);
        resources.extend(decl.query_types);
        resources.extend(decl.structural_properties);

        self.institutions.insert(iri, reasoner);
        Ok(resources)
    }

    pub fn get(&self, iri: &Iri) -> Option<&dyn FiberReasoner> {
        self.institutions.get(iri).map(|b| b.as_ref())
    }
}
```

### 4.2 Ontology Resources

Morphism types are ordinary ontology classes:

```json
{
  "@id": "urn:eigenius:fea:MeshRefinement",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
  "urn:eigenius:core:description": "A mesh refinement morphism between two FEA stress results.",
  "urn:eigenius:core:short_name": "MeshRefinement",
  "urn:eigenius:core:requires": [
    "urn:eigenius:fea:source",
    "urn:eigenius:fea:target",
    "urn:eigenius:fea:convergence_delta"
  ],
  "urn:eigenius:institutions:institution": "urn:eigenius:institutions:fea",
  "urn:eigenius:institutions:morphism_kind": "refinement"
}
```

The `institution` and `morphism_kind` properties link the class to its institution. The kernel uses these to route `validate_morphism` calls to the correct FiberReasoner.

---

## 5. Kernel Dispatch Model

### 5.1 Morphism Validation

When a resource with a morphism class enters the knowledge graph (via `Load` or `Reflect`):

1. The kernel performs structural validation (12 rules).
2. The kernel checks the `institution` property on the resource's class.
3. If an institution is registered for that IRI, the kernel calls `validate_morphism`.
4. If the institution returns `Invalid`, the resource is rejected.
5. If the institution returns `Undecidable`, the resource is accepted with a warning.

### 5.2 Fiber Query Dispatch

When an EigenQL query matches resources that are `FiberQuery` subclasses:

1. The kernel recognizes the query type's `institution` property.
2. The kernel dispatches to the institution's `query` method.
3. The result is returned as a typed resource.

Alternatively, programs can invoke fiber queries directly via `Apply`:

```esl
let convergence : fea:ConvergenceReport = FiberQuery(results) {
    institution:query_type = fea:ConvergenceQuery;
    institution:parameters = { ... };
};
```

### 5.3 Morphism Discovery

Programs can request the institution to discover morphisms:

```esl
let morphisms : core:resource_array = DiscoverMorphisms(results) {
    institution:institution = fea:FEA;
};
```

The kernel dispatches to `discover_morphisms`, receives candidate morphisms, and the program decides whether to commit them.

---

## 6. Comorphisms

### 6.1 What Is a Comorphism

An institution comorphism $\rho: \mathcal{I}_1 \to \mathcal{I}_2$ translates between institutions:
- Signatures translate forward: $\rho^{\mathrm{Sign}}: \mathrm{Sign}_1 \to \mathrm{Sign}_2$
- Sentences translate forward: $\rho^{\mathrm{Sen}}: \mathrm{Sen}_1 \to \mathrm{Sen}_2$
- Models translate backward: $\rho^{\mathrm{Mod}}: \mathrm{Mod}_2 \to \mathrm{Mod}_1$

The satisfaction condition is preserved: $M_2 \models_2 \rho(\varphi)$ iff $\rho(M_2) \models_1 \varphi$.

### 6.2 Eigenius Comorphisms

In Eigenius, a comorphism is a typed translation between two institutions:

```rust
pub trait Comorphism: Send + Sync {
    /// Source institution
    fn source(&self) -> &Iri;
    /// Target institution
    fn target(&self) -> &Iri;

    /// Translate a resource from the source institution's fiber
    /// into the target institution's fiber.
    fn translate_forward(
        &self,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;

    /// Translate a resource from the target institution's fiber
    /// back into the source institution's fiber.
    fn translate_backward(
        &self,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError>;
}
```

### 6.3 Existing Comorphisms

Two comorphisms already exist implicitly in the kernel:

**Ground type resolution** ($\rho: \text{Eigon} \to \text{Mini-TT}$):
- Forward: class IRI → Sigma type (via `ground.rs`)
- Backward: well-typed term → valid resource (via execution)
- This is a kernel-internal comorphism, not registered via the protocol

**Docking → Assay** ($\rho: \text{Dock} \to \text{Assay}$) from the paper:
- Forward: predicted binding affinity → expected IC₅₀ range
- Backward: dose-response curve → docking parameters
- This would be a registered comorphism between two domain institutions

### 6.4 Comorphism Registry

```rust
pub struct ComorphismRegistry {
    comorphisms: Vec<Box<dyn Comorphism>>,
}

impl ComorphismRegistry {
    pub fn find(&self, source: &Iri, target: &Iri) -> Option<&dyn Comorphism> {
        self.comorphisms.iter()
            .find(|c| c.source() == source && c.target() == target)
            .map(|c| c.as_ref())
    }
}
```

---

## 7. Epistemic Status and Institutions

### 7.1 Epistemic Categories Arise from Fiber Membership

| Status | What determines it |
|--------|-------------------|
| **Declared** | Resource committed by human assertion. No institution involvement. |
| **Observed** | Resource imported from external source with provenance. No institution reasoning. |
| **Derived** | Resource produced by program execution with traces. IO institutions dispatched. |
| **Verified** | Resource carries a formal proof from a verification institution (e.g., Lean 4). |

### 7.2 Lean 4 as a Verification Institution

Lean 4 is an external CIC institution. It is *not* Mini-TT — it's a separate, more powerful type theory that provides the "verified" epistemic level.

```rust
struct Lean4Institution {
    // Connection to Lean 4 server (e.g., via LSP or custom protocol)
    lean_endpoint: String,
}

impl FiberReasoner for Lean4Institution {
    fn fiber_declaration(&self) -> FiberDeclaration {
        FiberDeclaration {
            institution_iri: Iri::parse("urn:eigenius:institutions:lean4").unwrap(),
            name: "Lean 4 Formal Verification".to_string(),
            morphism_types: vec![
                // ProofReduction: a morphism between proof terms
                // representing definitional equality / reduction steps
            ],
            query_types: vec![
                // TypeCheck: verify that a term has a type
                // ProofSearch: find a proof of a proposition
            ],
            structural_properties: vec![],
        }
    }

    fn validate_morphism(
        &self,
        morphism: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<MorphismValidation, InstitutionError> {
        // Send the proof term to Lean 4 for checking
        // If Lean accepts it, the morphism (proof reduction) is valid
        todo!("dispatch to Lean 4 server")
    }

    fn query(
        &self,
        query: &Resource,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        // Type-check a term or search for a proof
        todo!("dispatch to Lean 4 server")
    }

    fn discover_morphisms(
        &self,
        _resources: &[Resource],
        _ctx: &ExecutionContext,
    ) -> Result<Vec<Resource>, InstitutionError> {
        // Lean doesn't discover morphisms — proofs are constructed, not found
        Ok(vec![])
    }
}
```

### 7.3 The Verification Flow

1. A program produces a derived result with a ProgramTrace.
2. A verification program constructs a Lean 4 proof term that the result is correct.
3. The proof term is submitted to the Lean 4 institution via `validate_morphism`.
4. If valid, a `VerificationTrace` is committed with the proof term.
5. The resource's epistemic status is promoted from derived → verified.

Mini-TT cannot do this — it checks program *composition*, not domain *correctness*. Lean 4 can express and check arbitrary mathematical propositions about the domain.

---

## 8. EigenQL Integration

No query language changes are needed. Morphisms are ordinary resources:

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

For queries requiring domain reasoning (e.g., "is this mesh converged?"), the query contains a `FiberQuery` subclass that the evaluator dispatches to the institution's `query` method.

---

## 9. Worked Examples

### 9.1 Mechanical Engineering (CAD + FEA + GenAI)

Three institutions over a shared bracket design model:

**$\mathcal{I}_{\text{FEA}}$:**
- Morphism types: `MeshRefinement` (convergence_delta), `QuantityExtraction` (von Mises, deflection)
- Fiber queries: `ConvergenceQuery` (is the mesh refined enough?)
- Structural property: `MeshRefinement` is a preorder (reflexive, transitive)

**$\mathcal{I}_{\text{GenAI}}$:**
- Morphism types: `ParetoDominance` (on mass-vs-strength front)
- Fiber queries: `ParetoFrontQuery` (which candidates are optimal?)

**$\mathcal{I}_{\text{CAD}}$:**
- Morphism types: `ParametricVariation` (changing a dimension produces a related geometry)
- Fiber queries: `ConstraintSatisfactionQuery` (does this geometry satisfy the bolt pattern?)

**Cross-institution query:** "For each GenAI candidate, find the finest-mesh FEA result and check if safety factor > 2.0"

### 9.2 Biopharmaceutical R&D (Docking + ADMET + Assays + PK)

Four institutions over a shared compound knowledge graph:

**$\mathcal{I}_{\text{Dock}}$:**
- Morphism types: `ConformationalProximity` (RMSD between poses), `ReScoring` (same pose, different force field)
- Fiber queries: `EnsembleQuery` (binding mode clusters)

**$\mathcal{I}_{\text{ADMET}}$:**
- Morphism types: `ModelAgreement` (ensemble sub-model predictions)
- Fiber queries: `DisagreementQuery` (where do models disagree?)

**$\mathcal{I}_{\text{Assay}}$:**
- Morphism types: `ReplicateRelationship`, `ProtocolVariation`
- Fiber queries: `DoseResponseQuery` (fitted IC₅₀ from raw data)

**$\mathcal{I}_{\text{PK}}$:**
- Morphism types: `CompartmentRefinement` (1-compartment → 2-compartment)
- Fiber queries: `TherapeuticWindowQuery` (C_max at dose within target range?)

**Comorphism $\rho_{\text{Dock} \to \text{Assay}}$:** predicted ΔG → expected IC₅₀ range.

---

## 10. Implementation Plan

### 10.1 Steps

1. Define `FiberReasoner`, `FiberDeclaration`, `Comorphism` traits in `kernel/src/institution/mod.rs`
2. Implement `InstitutionRegistry` and `ComorphismRegistry`
3. Add `institution` property to ontology classes for dispatch routing
4. Wire morphism validation dispatch into the validator
5. Wire fiber query dispatch into EigenQL evaluator
6. Implement a test institution (e.g., a simple ordering institution with transitivity)
7. Implement one worked example as an integration test

### 10.2 New Files

```
kernel/src/institution/
    mod.rs          — FiberReasoner trait, InstitutionRegistry
    comorphism.rs   — Comorphism trait, ComorphismRegistry
    error.rs        — InstitutionError types
```

### 10.3 Modified Files

- `kernel/src/validation/mod.rs` — dispatch to institution for morphism validation
- `kernel/src/query/evaluate.rs` — dispatch fiber queries to institutions
- `kernel/src/server/mod.rs` — expose institution registration and fiber queries via gRPC
- `proto/eigenius.proto` — add FiberQuery, DiscoverMorphisms, and ListInstitutions RPCs

### 10.4 Proto Changes

```proto
// In EigeniusKernel service:

// Execute a fiber query against a registered institution.
rpc FiberQuery(FiberQueryRequest) returns (FiberQueryResponse);

// Discover morphisms between resources within an institution's fiber.
rpc DiscoverMorphisms(DiscoverMorphismsRequest) returns (DiscoverMorphismsResponse);

// List registered institutions and their declared fiber structure.
rpc ListInstitutions(ListInstitutionsRequest) returns (ListInstitutionsResponse);

message FiberQueryRequest {
  string institution_iri = 1;   // Which institution to dispatch to
  bytes query = 2;              // Query resource as CBOR or Eigon-JSON
  string content_type = 3;
}

message FiberQueryResponse {
  bool success = 1;
  bytes result = 2;             // Result resource as CBOR
  string error = 3;
}

message DiscoverMorphismsRequest {
  string institution_iri = 1;   // Which institution to dispatch to
  repeated bytes resources = 2; // Resources to analyze, as CBOR
  string content_type = 3;
}

message DiscoverMorphismsResponse {
  bool success = 1;
  repeated bytes morphisms = 2; // Discovered morphism resources as CBOR
  string error = 3;
}

message ListInstitutionsRequest {}

message ListInstitutionsResponse {
  repeated InstitutionInfo institutions = 1;
}

message InstitutionInfo {
  string iri = 1;
  string name = 2;
  repeated string morphism_types = 3;  // IRIs of declared morphism classes
  repeated string query_types = 4;     // IRIs of declared query classes
}
```

The `FiberQuery` RPC dispatches to the institution's `query` method. The `DiscoverMorphisms` RPC dispatches to `discover_morphisms`. Morphism validation happens automatically during `Load` — no separate RPC needed (it's part of the validation pipeline).

`ListInstitutions` is a diagnostic/tooling RPC — it returns the registered institutions and their declared types, useful for MCP tools and CLI introspection.

---

## 11. WASM Compatibility (Phase 8 Preview)

The `FiberReasoner` trait maps to a WASM export interface:
- Each method becomes an exported function
- Input/output as CBOR-encoded resources
- Memory isolation, fuel-bounded execution
- Same sandbox constraints as other WASM capabilities

An institution implemented in Rust compiles to WASM and runs in-kernel via Wasmtime. An institution implemented as an external service (e.g., Lean 4 server) connects via gRPC.

---

## 12. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Is the kernel an institution? | No — it's the fixed foundation | Avoids self-referentiality; kernel validates and dispatches |
| Is Mini-TT an institution? | No — it's a kernel service | It's necessary for the kernel to function; institutions are optional domain services |
| Can Lean 4 be an institution? | Yes — it provides "verified" epistemic level | External CIC, more powerful than Mini-TT, domain-specific formal verification |
| Morphism storage | Ordinary ontology resources in the knowledge graph | Queryable via EigenQL, validated by the kernel, no special storage |
| Structural properties | Advisory, not enforced by the kernel | The institution enforces its own invariants via validate_morphism |
| Comorphism implementation | Rust trait, registered at startup | Type-safe translations between institutions |
| Fiber reasoner dispatch | By institution IRI on the morphism/query class | Same pattern as component dispatch |
| WASM institutions | Via the same WASM sandbox as capabilities (Phase 8) | Unified extensibility model |
| Fiber reasoner hosting | In-process Rust trait objects for Phase 6; gRPC for external services (e.g., Lean 4); WASM in Phase 8 | Start simple, extend later |
| Institution registration | At server startup, same as component registration | No dynamic registration RPC needed initially |
| Test institution | Simple ordering/refinement institution with transitivity checking | Minimal but exercises all four FiberReasoner methods |
| Morphism validation RPC | No separate RPC — dispatched automatically during Load | Validation is part of the existing pipeline, not a new endpoint |
