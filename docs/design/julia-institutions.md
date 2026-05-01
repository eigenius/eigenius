# Julia Institutions

**Status:** Draft — outline for the design specification
**Scope:** What it takes to bring Julia up as the first concrete instance of the [runtime substrate](runtime-substrate.md), and to register specific Julia libraries as Eigenius institutions on top of it under the [D14 institution protocol](d14-institution-realisation.md). Covers the Julia-specific resource subclasses, the `eigon-julia-gen` mirror generator, three reference institutions (`Symbolics` / `ModelingToolkit`, `JuMP`, `IntervalArithmetic`), and the future Lean / Julia bridge.
**Related:** [`d14-institution-realisation.md`](d14-institution-realisation.md) (the institution protocol — typed declarations, trait surface, dispatch model, Verdict, Comorphism shape — that each Julia institution instantiates), [`runtime-substrate.md`](runtime-substrate.md) (the language-agnostic substrate this layers on), [`lean-4-as-institution.md`](lean-4-as-institution.md) (the proof-bearing institution the Julia integration eventually pairs with), `boundary-contracts.md` (meta-spec context — under D14 the per-institution `BoundaryContract` collapses into typed declarations + Verdict; see §5)

## 1. Purpose and scope

The runtime substrate ([`runtime-substrate.md`](runtime-substrate.md)) is what makes Julia code executable inside Eigenius with full provenance — `RunRuntimeScript` and `CallRuntimeMethod` components, content-addressed `RuntimeScript` and `RuntimePackage` resources, OCI-image-pinned `RuntimeEnvironment` resources, all the boundary-check and worker-pool machinery. That gets Julia onto the platform.

This document covers what comes after: **what makes Julia interesting beyond "another runtime."** Specifically:

- The Julia-specific subclasses of the substrate's parent resource classes (`JuliaScript extends RuntimeScript`, etc.).
- The `eigon-julia-gen` mirror generator and its faithful-translation specification.
- Three reference institutions wrapping Julia libraries that implement formal reasoning systems with their own fibers:
  - **`Symbolics` / `ModelingToolkit`** — symbolic algebra, equation simplification, substitution.
  - **`JuMP`** — optimisation with solver-side certificates.
  - **`IntervalArithmetic`** — rigorous numerical bounds.
- The future Lean / Julia bridge — once both integrations are mature, a Julia computation produces a *derived* result and a Lean proof asserts a property of the algorithm or its bounds.

### 1.1 Why Julia first

Julia is the first language substrate to land for four concrete reasons:

- **Type system.** Multiple dispatch on rich parametric types is unusually well-aligned with Eigon class IRIs becoming Julia struct types. The mirror-generator pattern works in Python (with stubs) and R (less cleanly) but is most natural in Julia.
- **Reproducibility primitives.** `Project.toml` + `Manifest.toml` are first-class, idiomatic, and already what scientific Julia teams rely on. No equivalent baseline in Python (`requirements.txt` underspecifies, `poetry.lock` is partial, `nix` works but isn't typical).
- **Performance.** Julia's "two-language problem" thesis applies: the integration substrate doesn't have to be fast, but the user code running inside it does, and Julia native performance lets the substrate focus on provenance and dispatch rather than micro-optimising data movement.
- **Reasoning libraries.** `Symbolics.jl`, `JuMP`, `IntervalArithmetic.jl`, `Catalyst.jl`, `ModelingToolkit.jl` — unusually substantial for a single ecosystem and they map onto Eigenius institutions cleanly.

### 1.2 Non-goals

- This is not a plan to embed Julia's runtime in the kernel. The substrate's worker model handles process lifecycle.
- This is not a route to *verified* knowledge. Julia produces *derived* with high-quality provenance. *Verified* claims about Julia computations come from pairing with Lean (§6).
- This is not a privileged language integration. Python, R, MATLAB substrates plug into the same runtime substrate when their integrations land.

### 1.3 D14 in one paragraph (so the rest of this doc is readable in isolation)

Under D14, every institution registers by committing five kinds of typed Resources to the layer chain: an `Institution` (identity + runtime kind), `ExportFormat`s (typed extractions of class instances into Mini-TT payloads), `ImportFormat`s (typed constructors of class instances from Mini-TT payloads), `QueryClass`es (typed functions in the institution's fibre with `dispatch_role` of `OnDemand` / `AutoOnLoad` / `Decidable` and a result class — `Verdict` for the gate-on-commit and decide-procedure roles), and `Comorphism`s (triples `(s, m, t)` where `s` is an ExportFormat, `m` is a Mini-TT Component, and `t` is an ImportFormat — the cross-institution bridge). The institution implements an `Institution` Rust trait with three methods: `extract_typed`, `reify`, and an optional `query`. Each Julia institution this doc describes — `Symbolics`/`ModelingToolkit`, `JuMP`, `IntervalArithmetic` — is its own D14 institution: one `Institution` resource per crate, its own typed declarations, its own trait implementation. They share the substrate's authoring-side machinery (image-pinned environment, mirror generator, worker pool) but they are independent reasoning systems at the institution-protocol level.

## 2. Julia-specific resource subclasses

The substrate commits parent classes (`RuntimeScript`, `RuntimePackage`, etc.); this layer commits Julia subclasses that add language-specific fields.

### 2.1 `JuliaScript` extends `RuntimeScript`

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | Always `"julia"`. |
| `source` | inherited | Julia source text. |
| `entry_point` | inherited | Method name as a Julia symbol. |
| `entry_point_signature` | inherited | IRI of a `JuliaMethodSignature`. |
| `requires_environment` | inherited | IRI of a `JuliaEnvironment`. |
| `requires_mirror_classes` | inherited | Eigon class IRIs the script's mirror-struct usage covers. |
| `julia_version_constraint` | new | Optional version compatibility expression (`"^1.10"`). The substrate uses this as a sanity check at dispatch — incompatible version → refuse. |
| `module_imports` | new | Declared `using`/`import` statements. Used by the substrate's static analyser to confirm all referenced packages are in the env. |

### 2.2 `JuliaPackage` extends `RuntimePackage`

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | `"julia"`. |
| `name` | inherited | The Julia package name (`MyAnalysis`). |
| `version` | inherited | Internal version string. |
| `manifest` | inherited | The package's `Project.toml`, embedded. |
| `source_tree` | inherited | Source archive or external reference. |
| `entry_points` | inherited | List of `JuliaMethodSignature` IRIs the package exports. |
| `julia_compat` | new | The `Project.toml` `[compat]` section as a structured field for fast querying. |

### 2.3 `JuliaEnvironment` extends `RuntimeEnvironment`

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | `"julia"`. |
| `runtime_version` | inherited | Exact Julia version (e.g. `"1.10.4"`). |
| `manifest` | inherited | `Manifest.toml` content, embedded. Verbatim bytes; the re-instantiation anchor. |
| `pinned_packages` | inherited | List of `JuliaPackagePin` IRIs (§2.7) — parsed Eigon view of the manifest. |
| `included_packages` | inherited | List of `JuliaPackage` IRIs (user-authored libraries) baked into the image. |
| `mirror_dependency` | inherited | IRI of the `JuliaPackageMirror`. |
| `image_digest` | inherited | OCI image digest. Production reproducibility anchor. |
| `image_reference` | inherited | Optional registry tag. |
| `project_toml` | new | The top-level `Project.toml` (separate from `manifest` because Julia treats them as distinct artifacts). |

### 2.4 `JuliaPackageMirror` extends `RuntimePackageMirror`

Same shape as the parent; no Julia-specific extensions beyond the structural mirror (the rendered Julia source is `library_content`).

### 2.5 `JuliaInvocation` extends `RuntimeInvocation`

| Property | Inherited / new | Purpose |
|---|---|---|
| All substrate-level fields | inherited | See substrate doc §5.5. |
| `julia_dispatch_method` | new | Fully-qualified Julia method `Module.method(::Type1, ::Type2, ...)` resolved by multiple dispatch — more specific than the substrate's generic `dispatched_to`. Recorded post-call from Julia's `which` introspection. |
| `julia_blas_vendor` | new | Which BLAS implementation was loaded (`"OpenBLAS"` / `"MKL"` / `"AppleAccelerate"`). Affects numerical reproducibility. |

### 2.6 `JuliaMethodSignature` extends `RuntimeMethodSignature`

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | `"julia"`. |
| `method_name` | inherited | The Julia method name. |
| `input_types` | inherited | Eigon class IRIs. |
| `output_type` | inherited | Eigon class IRI. |
| `package` | inherited | Optional `JuliaPackage` IRI or registry package name. |
| `julia_module_path` | new | Full module path (`MyAnalysis.Submodule`) where the method lives. |

### 2.7 `JuliaPackagePin` extends `RuntimePackagePin`

A parsed projection of one entry in `Manifest.toml`. Eigenius doesn't generate the manifest — Julia's `Pkg.resolve` does, as part of `Pkg.instantiate()` during image build. The substrate captures the bytes verbatim into `JuliaEnvironment.manifest`; the per-package `JuliaPackagePin` resources are then derived projections that make the dependency graph queryable through EigenQL without re-parsing TOML.

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | `"julia"`. |
| `package_name` | inherited | The Julia package name (`"Symbolics"`). |
| `package_identifier` | inherited | The Julia UUID (`"0c5d862f-..."`) — Julia's primary identifier. |
| `pinned_version` | inherited | The exact resolved version (`"5.4.2"`). |
| `source_hash` | inherited | The git tree hash from the manifest entry. |
| `source_origin` | inherited | The registry URL or git URL for the package. |
| `depended_on_by` | inherited | List of `JuliaPackagePin` IRIs that depend on this one. |
| `julia_compat_constraints` | new | The relevant `[compat]` constraints from `Project.toml` that constrained this resolution. Diagnostic / audit detail. |

Pins are content-addressed; a fresh `env create` against an unchanged Project + Manifest produces the same set of pin IRIs. Re-instantiation always goes through the verbatim `manifest`, never through reconstructing it from pins — pins are a read-only view.

## 3. The `eigon-julia-gen` mirror generator

A deterministic generator producing a Julia package mirroring Eigon class structure as Julia structs. Its outputs are committed back to Eigenius as `JuliaPackageMirror` resources.

### 3.1 What the mirror contains

For each Eigon class the user might call into Julia about:

- A Julia `struct` (or `mutable struct` where appropriate) with one field per required Eigon property.
- Type parameters where Eigon properties are resource-typed (the field's static type is the mirror struct of the referenced class).
- Constructor functions (`StressResult(...)`) that perform format-constraint validation at construction time. Format violations raise `EigenValidationError`; this matches Julia style (validation at the boundary).
- Conversion functions to/from Eigon-JSON / CBOR.
- An abstract type hierarchy reflecting Eigon's `subclass_of` relationships, so multiple-dispatch dispatch on supertypes works naturally.

### 3.2 What the mirror does NOT contain

- Constraint *predicates* as Julia values. Format constraints, `requires`/`recommends`, conditional requirements — checked at construction (validation), not encoded as Julia-level theorems.
- Behavioural specifications.

The mirror is **structural, not propositional**. Users who want to *prove* things about Eigon-shaped data use the Lean integration with `EigonFFI`; users who want to *compute* over them use Julia with this mirror.

### 3.3 Faithful translation

| Eigon construct | Julia construct |
|---|---|
| Class with required properties P₁..Pₙ | `struct ClassName{T₁,...,Tₙ}; field₁::T₁; ...; fieldₙ::Tₙ end` |
| Class with recommended properties | Same struct, optional fields with `Union{T, Nothing}` types |
| Subclass relationship `Sub <: Super` | `abstract type SuperType end` + `struct Sub <: SuperType` |
| `data_type: resource` property | Field type is the referenced class's mirror struct |
| `data_type: resource_array` | Field type is `Vector{<mirror struct>}` |
| `data_type: integer` / `float` / `boolean` / `string` | Julia primitive: `Int64` / `Float64` / `Bool` / `String` |
| `data_type: value_array` of T | `Vector{T_julia}` |
| Format constraints (regex, date, IRI pattern) | Constructor-level validation that raises on violation |

The faithful-translation specification is a finite, single-document artifact. It does not need to translate constraint predicates into refinement types or decide-procedure axiomatisations the way Lean's `EigonFFI` does.

### 3.4 Anchoring and compositionality

Inherited from the substrate's mirror model. A `JuliaPackageMirror` anchored to layer L₀ remains valid for invocations against any descendant layer L₁ ⊒ L₀ provided the mirrored classes haven't changed in L₁. When they have, the substrate rejects with `MirrorVersionMismatch`. The user regenerates and resubmits.

The generator binary is content-hashed; that hash is recorded in every `JuliaPackageMirror` it produces. Independent provenance verification: an auditor with the binary and the layer chain re-runs `eigon-julia-gen --layer L`, checks the content hash matches.

## 4. Reference institutions

Three Julia libraries that wrap as Eigenius institutions cleanly under D14. Each is its own crate (`eigenius-julia-symbolics`, `eigenius-julia-jump`, `eigenius-julia-intervals`) depending on `eigenius-julia` (the language crate, which depends on `eigenius-runtime-substrate`). Each crate ships:

1. An `Institution` resource declaring its IRI and its `External` runtime kind (it dispatches into a substrate-hosted Julia worker rather than running in-process).
2. A set of **resource classes** representing intra-fibre structure (the relations / typed claims the institution contributes).
3. **`ExportFormat`** declarations turning input class instances into Mini-TT payloads the institution's worker consumes.
4. **`ImportFormat`** declarations constructing result-class instances from worker-produced Mini-TT payloads.
5. **`QueryClass`** declarations with appropriate `dispatch_role`s — `AutoOnLoad` for relations the institution validates on commit, `OnDemand` for queries fired from EigenQL FIBER, `Decidable` for predicates referenced from `Exp::NativeDecide`.
6. Optionally **`Comorphism`** declarations bridging into other institutions (Julia ↔ Lean, Julia ↔ Julia, Julia → external).
7. A Rust `Institution` trait implementation. `extract_typed` / `reify` marshal between Eigon resources and CBOR-encoded Mini-TT payloads on the substrate RPC; `query` dispatches procedure IRIs to the worker over substrate RPC.

### 4.1 `Symbolics` / `ModelingToolkit` — symbolic algebra and equation simplification

The fibre of symbolic expressions has structure: equivalences modulo a rule set, substitutions, simplifications to canonical forms. The library implements the rule set; the institution wraps the dispatch.

#### 4.1.1 Resource classes (sentences in this fibre)

- **`SymbolicExpression`** — a symbolic-algebra expression resource. Carries the expression's typed term (a Mini-TT inductive shape representing the expression tree) and the rule set IRI it belongs to.
- **`SymbolicallyReducesTo`** — relates two `SymbolicExpression` resources under a rule set. Carries `expr1`, `expr2`, `rule_set`. The kernel auto-validates this on commit (§4.1.3 below).
- **`Substitutes`** — relates `(expr, var, value, result)`. Auto-validated on commit.
- **`SimplifiesTo`** — relates `(expr, normal_form, rule_set)`. Auto-validated.
- **`SatisfiesEquation`** — relates `(expr_lhs, expr_rhs, rule_set)`; "both sides simplify to the same normal form." Auto-validated.

#### 4.1.2 ExportFormats / ImportFormats

| Direction | IRI | Payload type | Procedure |
|---|---|---|---|
| Export from `SymbolicExpression` | `ef_symb_expr` | `SymbolicTerm` (Mini-TT inductive) | `urn:eigenius:symbolics:extract_expr` |
| Export from `SymbolicallyReducesTo` | `ef_symb_reduces_pair` | `(SymbolicTerm, SymbolicTerm, RuleSetIri)` | `urn:eigenius:symbolics:extract_reduces_pair` |
| Import to `SymbolicExpression` | `if_symb_expr` | `SymbolicTerm` | `urn:eigenius:symbolics:reify_expr` |
| Import to `SimplifiesTo` | `if_symb_simplifies_to` | `(SymbolicTerm, SymbolicTerm, RuleSetIri)` | `urn:eigenius:symbolics:reify_simplifies_to` |

#### 4.1.3 QueryClasses

| QueryClass | `query_class` | `result_class` | `dispatch_role` | Implementation |
|---|---|---|---|---|
| `qc_symb_validate_reduces_to` | `SymbolicallyReducesTo` | `Verdict` | `AutoOnLoad` | institution-runtime — the worker re-runs the rewrite and checks `expr1 → expr2` under the rule set. |
| `qc_symb_validate_simplifies_to` | `SimplifiesTo` | `Verdict` | `AutoOnLoad` | institution-runtime — the worker simplifies `expr` and checks the result equals `normal_form`. |
| `qc_symb_validate_satisfies_equation` | `SatisfiesEquation` | `Verdict` | `AutoOnLoad` | institution-runtime — both sides simplify to the same form. |
| `qc_symb_simplify` | `SymbolicExpression` | `SymbolicExpression` | `OnDemand` | institution-runtime — return canonical form. |
| `qc_symb_substitute` | `SymbolicSubstitutionInput` | `SymbolicExpression` | `OnDemand` | institution-runtime. |
| `qc_symb_check_equivalence` | `SymbolicEquivalenceInput` | `Verdict` | `OnDemand`, `Decidable` | institution-runtime — the OnDemand role permits FIBER-side equivalence queries; the Decidable role permits use from `Exp::NativeDecide` in user programs. |
| `qc_symb_extract_symbols` | `SymbolicExpression` | `SymbolicSymbolSet` | `OnDemand` | institution-runtime. |
| `qc_symb_to_latex` | `SymbolicExpression` | `LatexString` | `OnDemand` | institution-runtime. |
| `qc_symb_differentiate` | `SymbolicDifferentiationInput` | `SymbolicExpression` | `OnDemand` | institution-runtime. |

#### 4.1.4 Why this is an institution and not just a substrate component

The fibre has *transitivity* and *equivalence* structure that substrate components alone don't expose. A `SymbolicallyReducesTo` resource is a typed relation that the institution validates (the AutoOnLoad QueryClass), discovers (an OnDemand "find common form" query could return candidate reduction pairs), and that downstream programs can reason about transitively. Substrate `RunRuntimeScript` would just run a script and produce a result — it would not give that result the typed-relation status that lets EigenQL FIBER traverse a chain of `SymbolicallyReducesTo` resources or that lets Mini-TT's `NativeDecide` reduce a propositional equivalence.

### 4.2 `JuMP` — optimisation

Optimisation as a fibre. The institution wraps `JuMP`'s solver-agnostic interface; specific solver choice is a registration parameter (multiple `Institution` resources, one per solver).

#### 4.2.1 Resource classes

- **`OptimisationProblem`** — typed problem statement (objective, variables, constraints) plus solver-side metadata.
- **`OptimisesTo`** — relates `(problem, optimum, certificate)`. The optimum value plus the solver's primal/dual certificate. Auto-validated on commit by re-checking the certificate.
- **`Infeasible`** — relates `(problem, witness)` where the witness is an IIS or analogous structure. Auto-validated.
- **`BoundedBy`** — relates `(problem, lower, upper)`. Bounds short of optimality, useful for time-capped solves. Auto-validated.

#### 4.2.2 ExportFormats / ImportFormats

| Direction | IRI | Payload | Procedure |
|---|---|---|---|
| Export `OptimisationProblem` | `ef_jump_problem` | `JumpModelRepr` | `urn:eigenius:jump:extract_problem` |
| Import `OptimisesTo` | `if_jump_optimises_to` | `(Float, OptimumCertificate)` | `urn:eigenius:jump:reify_optimises_to` |
| Import `Infeasible` | `if_jump_infeasible` | `IIS` | `urn:eigenius:jump:reify_infeasible` |
| Import `BoundedBy` | `if_jump_bounded_by` | `(Float, Float)` | `urn:eigenius:jump:reify_bounded_by` |

#### 4.2.3 QueryClasses

| QueryClass | Input | Result | Roles | Implementation |
|---|---|---|---|---|
| `qc_jump_validate_optimum` | `OptimisesTo` | `Verdict` | `AutoOnLoad` | institution-runtime — re-checks the certificate; reports `Holds`, `Fails`, or `Undecidable` (e.g. for non-reproducible solver state). |
| `qc_jump_validate_infeasible` | `Infeasible` | `Verdict` | `AutoOnLoad` | institution-runtime. |
| `qc_jump_solve` | `OptimisationProblem` | `OptimisesTo` | `OnDemand` | institution-runtime. |
| `qc_jump_is_infeasible` | `OptimisationProblem` | `Verdict` | `OnDemand`, `Decidable` | institution-runtime. |
| `qc_jump_bounds_after` | `JumpBoundsAfterInput` (problem + time limit) | `BoundedBy` | `OnDemand` | institution-runtime. |
| `qc_jump_sensitivity_analysis` | `OptimisesTo` | `JumpSensitivityReport` | `OnDemand` | institution-runtime — LP only. |

#### 4.2.4 Solver as registration parameter

The institution registers per-solver: `eigenius-julia-jump-highs`, `eigenius-julia-jump-glpk`, `eigenius-julia-jump-ipopt`, with `eigenius-julia-jump-gurobi` if licensed. Each is a separate `Institution` resource (different IRI) referencing its own `JuliaEnvironment` (the right solver wired in). Their QueryClass declarations parallel each other but bind to different procedure IRIs that dispatch into the matching worker. Multiple solver-institutions coexist in the chain; users invoke the one whose IRI they want.

### 4.3 `IntervalArithmetic` — rigorous numerical bounds

`IntervalArithmetic.jl` produces *operationally* verifiable claims (mathematical bounds that hold by construction, modulo correctness of the library). The institution exposes those claims as typed resources.

#### 4.3.1 Resource classes

- **`BoundedBy`** — relates `(value, interval)` where the value's true magnitude is guaranteed to lie in the interval. Auto-validated on commit by re-running the interval extension and checking the claimed bound is at least as tight as the institution computes (or rejecting if the claimed bound is tighter than provably possible).
- **`ProvesBoundOn`** — relates `(function, domain, interval)`. The interval-extended function on the stated domain is bounded by the interval. Auto-validated.
- **`ContainsRoot`** — relates `(function, domain)` with an interval-Newton-style witness that a root exists in the domain (or, dually, that no root exists). Auto-validated.

#### 4.3.2 ExportFormats / ImportFormats

| Direction | IRI | Payload | Procedure |
|---|---|---|---|
| Export `BoundedBy` | `ef_intv_bounded_by` | `(Float, IntervalRepr)` | `urn:eigenius:intv:extract_bounded_by` |
| Export `ProvesBoundOn` | `ef_intv_proves_bound_on` | `(FunctionRepr, IntervalRepr, IntervalRepr)` | `urn:eigenius:intv:extract_proves_bound_on` |
| Import `BoundedBy` | `if_intv_bounded_by` | `(Float, IntervalRepr)` | `urn:eigenius:intv:reify_bounded_by` |
| Import `ProvesBoundOn` | `if_intv_proves_bound_on` | `(FunctionRepr, IntervalRepr, IntervalRepr)` | `urn:eigenius:intv:reify_proves_bound_on` |
| Import `ContainsRoot` | `if_intv_contains_root` | `(FunctionRepr, IntervalRepr, IntervalNewtonWitness)` | `urn:eigenius:intv:reify_contains_root` |

#### 4.3.3 QueryClasses

| QueryClass | Input | Result | Roles | Implementation |
|---|---|---|---|---|
| `qc_intv_validate_bounded_by` | `BoundedBy` | `Verdict` | `AutoOnLoad`, `Decidable` | institution-runtime. The Decidable role lets `Exp::NativeDecide(BoundedBy(v, i), _)` reduce in user programs. |
| `qc_intv_validate_proves_bound_on` | `ProvesBoundOn` | `Verdict` | `AutoOnLoad` | institution-runtime. |
| `qc_intv_validate_contains_root` | `ContainsRoot` | `Verdict` | `AutoOnLoad` | institution-runtime — replays the interval-Newton witness. |
| `qc_intv_compute_bounds` | `IntvComputeBoundsInput` (function + domain) | `ProvesBoundOn` | `OnDemand` | institution-runtime. |
| `qc_intv_verify_bound` | `BoundedBy` (claimed) | `Verdict` | `OnDemand` | institution-runtime — convenience wrapper for "is this claim *at least as tight* as the institution computes?". |
| `qc_intv_find_roots` | `IntvFindRootsInput` | `IntvRootList` | `OnDemand` | institution-runtime. |

#### 4.3.4 Epistemic note

Interval-arithmetic outputs are operationally stronger than ordinary numerical results — the bounds hold by construction. Under D14's epistemic categorisation ([D14 §7.1](d14-institution-realisation.md)) they remain *derived* (they depend on Julia and the library, not a machine-checked proof). Promotion to *verified* requires pairing with Lean — see §6.2. The Decidable role on `qc_intv_validate_bounded_by` lets a user program write a typed predicate `BoundedBy(v, [lo, hi])` and have the kernel reduce it operationally; the resulting `Refl` witness is grounded in the institution's worker, not a Lean proof, so the epistemic level is still *derived*.

### 4.4 Other Julia institutions worth considering

Each is its own design exercise; this document scopes only the three above.

- **`SciML` / `DifferentialEquations.jl`** — ODE / PDE / DAE solvers. Could be substrate components (one solve, derived output) or a real institution (the "ODE solution" fiber has structure: convergence proofs, error bounds, step refinement).
- **`Catalyst.jl`** — chemical reaction networks.
- **`Turing.jl`** — probabilistic programming. Bayesian-inference institution. Morphism types around posteriors and credible intervals are interesting and not yet thought through carefully.
- **`Manopt.jl`** — optimisation on Riemannian manifolds.
- **`HomotopyContinuation.jl`** — algebraic-geometry root finding with certified tracking.

When demand for one of these crystallises, it gets its own design doc and crate, plugging into `eigenius-julia` the same way the three reference institutions do.

## 5. Verdict shape, error taxonomy, and runtime properties per institution

D10 framed institutional contracts as `FiberReasonerContract` instances — one per institution. Under D14, those contracts collapse into the typed declarations of §4 (the resource classes, ExportFormats, ImportFormats, QueryClasses) plus the kernel-defined `Verdict` shape ([D14 §6.1](d14-institution-realisation.md)). This section names the per-institution concretisations of what D10 called "contract concerns."

### 5.1 Verdict diagnostics

Every `AutoOnLoad` and `Decidable` QueryClass returns a `Verdict`. On `Fails`, the verdict's diagnostic field carries an institution-specific reason. Representative diagnostics (illustrated for `Symbolics`; the pattern repeats per institution):

- `RuleSetUnknown` — the named rule set isn't loaded in the worker's environment.
- `ExpressionMalformed` — the input couldn't be parsed into the worker's term representation.
- `SimplificationDiverged` — the rewrite system entered a cycle or exceeded the configured step limit.
- `SymbolicTimeout` — the per-call wall-clock cap fired.
- `ReductionMismatch` — for `qc_symb_validate_reduces_to`, the worker's rewrite did not arrive at the claimed `expr2`.

`JuMP` analogues: `SolverUnavailable`, `ProblemMalformed`, `CertificateRejected`, `SolverTimeout`. `IntervalArithmetic` analogues: `IntervalRepresentationInvalid`, `BoundTooTight`, `WitnessRejected`, `IntervalTimeout`.

`Verdict::Undecidable` is used where the institution can't reach a binary verdict — e.g. JuMP returning a non-replay-able solver state, or IntervalArithmetic failing to converge under the configured rounding mode. The kernel treats `Undecidable` differently from `Fails` per [D14 §6.1](d14-institution-realisation.md): on Load it admits the resource without committing the institution to its truth; in `Exp::NativeDecide` it leaves the constraint as a passthrough neutral.

### 5.2 Runtime properties

Advisory; not declared in any single resource. Operators rely on these for capacity planning and audit reasoning.

| Property | Symbolics | JuMP | IntervalArithmetic |
|---|---|---|---|
| Determinism | Same expression + rule set + ontology layer → same result. | Same problem + solver build → same primal/dual values modulo solver-internal seed. Some solvers are bit-non-deterministic across runs; the worker records actual seed in invocation provenance. | Bit-deterministic given pinned BLAS + rounding mode. |
| Idempotence | Re-simplifying yields the same form. | Re-solving an `OptimisesTo` against the same problem yields a `Verdict::Holds` re-validation. | Re-validating bounds yields the same verdict. |
| Effects | Read-only against the chain; substrate-level network/filesystem policy applies. | Read-only. | Read-only. |
| Resource bounds | Per-call wall-clock and memory caps; cap violation → `SymbolicTimeout`. | Per-call wall-clock cap; for solvers with explicit time-limit support, the cap is forwarded to the solver. | Per-call wall-clock cap. |

These properties surface in trace metadata (the substrate's `RuntimeInvocation` provenance — see [`runtime-substrate.md`](runtime-substrate.md) §5.5) and in the verdict's auxiliary fields, not in a separate per-institution contract document.

## 6. The Lean / Julia bridge (future work)

Once both integrations are mature, a natural pattern: a Julia computation produces a *derived* result, and a Lean proof asserts a property about the *algorithm* that produced it (or an interval enclosing the expected result). Three concrete bridges are worth designing:

### 6.1 Aligned mirror packages

Lean's `EigonFFI` and Julia's `JuliaPackageMirror` for the same Eigon class need to be structurally aligned, so a claim about a `StressResult` in Lean is recognisable as a claim about the same `StressResult` Julia produced. Because both generators are deterministic and content-anchored to the same layer chain, a small "mirror equivalence" check at the institution boundary suffices: confirm the Lean mirror and the Julia mirror were generated from the same `source_layer` and that the relevant class has byte-identical structure in both.

Now that Lean's authoring side leverages the same runtime substrate as Julia ([`lean-4-as-institution.md`](lean-4-as-institution.md) §2.3), this check is mechanically simple: `JuliaPackageMirror` and `LeanPackageMirror` both extend `RuntimePackageMirror`, so structural equivalence reduces to a comparison on the parent type's `source_layer` and `mirrored_classes` properties. The bridge crate doesn't need language-specific knowledge to verify alignment.

### 6.2 Verification of `IntervalArithmetic` outputs — concrete D14 `Comorphism`

A Lean proof asserting "the function `f` has, on domain `[a, b]`, output in `[lo, hi]`" can be paired with a Julia `IntervalArithmetic` invocation producing the same bounds. Under D14, the bridge is a `Comorphism` resource ([D14 §4.5 + §5](d14-institution-realisation.md)): a triple `(s, m, t)` where `s` is an ExportFormat from the Julia interval institution, `m` is a Mini-TT Component carrying the validation logic, and `t` is an ImportFormat on the Lean institution constructing a `LeanProofTerm` whose proposition asserts the bound.

Sketch:

```
ExportFormat ef_intv_proves_bound_on   (declared by eigenius-julia-intervals)
  from_class:    ProvesBoundOn
  payload_type:  (FunctionRepr, IntervalRepr, IntervalRepr)
  procedure:     urn:eigenius:intv:extract_proves_bound_on

Component cm_intv_to_lean_obligation   (declared in the bridge layer)
  type: (FunctionRepr, IntervalRepr, IntervalRepr) ->
        LeanProofObligation
  body: <package the interval data + the matching Lean proposition shape>

ImportFormat if_lean_bound_proof   (declared by eigenius-lean)
  to_class:      LeanProofTerm
  payload_type:  LeanProofObligation
  procedure:     urn:eigenius:lean:reify_bound_proof

Comorphism ρ_intv_to_lean_bound
  export_format:  ef_intv_proves_bound_on
  transformation: cm_intv_to_lean_obligation
  import_format:  if_lean_bound_proof
  exact: false
  description: "Inexact: the Comorphism plumbs the interval data into a
                Lean proof obligation; the proof itself must still be
                supplied externally."
```

The kernel type-checks the Comorphism at commit time: `cm_intv_to_lean_obligation` must have the signature shown ([D14 §4.5](d14-institution-realisation.md)). At runtime, `Exp::InstitutionInvoke { comorphism_iri: ρ_intv_to_lean_bound, source: <ProvesBoundOn IRI> }` produces a `LeanProofTerm` resource; the `qc_proof_check` AutoOnLoad QueryClass on the Lean side fires automatically and either verifies or rejects the proof. If the proof side carries actual Lean witnessing the same bound, the resulting `LeanProofTerm` lands as *verified*; downstream consumers see the bound carries both *verified* (Lean) and *derived* (Julia interval) warrants. Agreement rules out implementation bugs the Lean abstraction missed.

The "inexactness" is honest: the comorphism does not generate the Lean proof — it plumbs the interval data into a proof obligation that some external author still has to discharge. A future "exact" variant would have `m` carry a *derivation* witnessing that the source-side claim implies the target-side claim ([D14 §5 last bullet](d14-institution-realisation.md)); that's open research, not v1 scope.

The earlier draft of this doc described a separate `JointlyBoundedBy` morphism class. Under D14 that class is unnecessary: the joint warrant lives in the *provenance* of the resulting `LeanProofTerm` (its `RuntimeInvocation` chains back through the comorphism to the `ProvesBoundOn` source) plus the kernel-tagged epistemic status. No new resource class needed.

### 6.3 Algorithm-correctness proofs about Julia code

The harder bridge. A Lean proof asserts "this Julia function, modelled as a Lean function, computes the partial sum of a series correctly." The Julia function and the Lean function are different artifacts; correspondence between them is by *human-supplied translation*, not mechanical. Useful but trust-extended; out of scope for v1 but the place this leads. Structurally it would still be a D14 `Comorphism` — `s` extracting the Julia function repr (a `JuliaScript` ExportFormat), `m` packaging the human-supplied translation as a Mini-TT term plus the matching Lean obligation, `t` constructing the resulting `LeanProofTerm`.

## 7. Implementation approach — the Julia substrate instance and per-institution crates

Per the substrate doc §3, each language crate implements `LanguageRuntime`. Per [D14 §8](d14-institution-realisation.md), each institution crate also implements the `Institution` trait. The Julia implementation:

### 7.1 `eigenius-julia` crate responsibilities

- Implements `LanguageRuntime::language_id() -> "julia"` (substrate side).
- Provides Dockerfile fragments installing `juliaup`, the pinned Julia version, the lockfile-instantiated registry packages, the user packages, and the mirror.
- Provides the worker bootstrap script (`worker.jl`) that handles RPC, mirror loading, dispatch.
- Owns the `JuliaScript`, `JuliaPackage`, `JuliaEnvironment`, `JuliaPackageMirror`, `JuliaInvocation`, `JuliaMethodSignature` resource class declarations.
- Implements `eigon-julia-gen` (or depends on a separate crate that does).
- Provides shared marshalling helpers used by per-institution crates' `extract_typed` / `reify` implementations.

`eigenius-julia` is *not* itself an institution under D14 — it's the language crate. Each per-institution crate (`eigenius-julia-symbolics`, `eigenius-julia-jump`, `eigenius-julia-intervals`) implements D14's `Institution` trait independently. They commit their own `Institution` resource, their own ExportFormats / ImportFormats / QueryClasses, and they share the `eigenius-julia` worker pool through the substrate. A single Julia worker container can serve dispatches from multiple institutions, since the procedure dispatch happens inside the worker by procedure IRI.

### 7.2 Worker bootstrap

```julia
# worker.jl — runs inside the Julia container.
using JSON3, CBOR, Sockets

function verify_environment()
    expected_digest = ENV["EIGENIUS_RUNTIME_ENV_DIGEST"]
    expected_manifest_hash = ENV["EIGENIUS_RUNTIME_ENV_MANIFEST_HASH"]
    in_image = strip(read("/etc/eigenius-runtime-env/manifest-hash", String))
    if expected_manifest_hash != in_image
        error("env-image mismatch: substrate expected $expected_manifest_hash, " *
              "image was built with $in_image. Refusing to start.")
    end
    @info "Julia worker started" digest=expected_digest
end

verify_environment()
using EigeniusMirror  # the mirror package, baked in at /mirror/

function run_script(script_source::String, entry_point::Symbol, inputs::Vector)
    # Eval script in a fresh module to scope its definitions.
    mod = Module(:InvocationScope)
    Base.include_string(mod, script_source)
    # Resolve entry point; dispatch on input mirror types.
    method = getfield(mod, entry_point)
    result = method(inputs...)
    # Record which method dispatched, for JuliaInvocation.julia_dispatch_method.
    actual_method = which(method, typeof.(inputs))
    return (result, string(actual_method))
end

# RPC loop omitted for brevity. Frames decoded with CBOR.decode, encoded with
# CBOR.encode; numerical arrays use RFC 8746 typed-array tags so FP matrices
# don't pay per-element type-tag overhead.
```

### 7.3 RPC

Per the substrate, CBOR on the wire. Julia side uses [`CBOR.jl`](https://github.com/saolsen/CBOR.jl) for marshalling. Dispatched-method recording uses `which()` to capture the actual method resolved by Julia's dispatcher.

## 8. Phased implementation plan

T-shirt sizes, ordered by dependency. The substrate's phases (substrate doc §13) run in parallel with these where the dependency permits.

### Phase A — Julia substrate proof of concept

`eigenius-julia` crate with `RunRuntimeScript` only. One persistent Julia worker per process (no pool). Inputs and outputs cross the boundary as Eigon-JSON (CBOR worker codec lands in Phase B). No mirror package — scripts work on raw JSON via `JSON3.jl`. `JuliaEnvironment` and `JuliaInvocation` resources committed; `image_digest` left empty (deployment shape (c) — Julia bundled into the orchestrator image directly). Demonstrates end-to-end: script + environment in, script runs, output committed with provenance.

**Scope:** Small. Validates the substrate shape against a real language.

### Phase B — `eigon-julia-gen` mirror generator

Deterministic generator implementation. Faithful-translation specification authored in parallel. `JuliaPackageMirror` resources committed. `CallRuntimeMethod` (Julia variant) using mirror struct dispatch. Boundary checks per substrate §7.5. CBOR on the wire (replaces the Phase A JSON bootstrap), with RFC 8746 typed-array tags for numerical arrays.

**Scope:** Medium. The trust-surface work and the spec-authoring work concentrate here.

### Phase C — Per-environment images

The substrate's image-build pipeline (substrate doc §9.2) lands here for the Julia case. Deterministic two-stage Dockerfile, build-time provenance baked in (manifest hash, mirror IRI, included packages), registry push with digest capture, `JuliaEnvironment.image_digest` populated automatically. Worker bootstrap performs the in-image-vs-env-var cross-check. Multi-environment worker pool with LRU eviction.

**Scope:** Medium-to-Large.

### Phase D — First institution: `Symbolics` / `ModelingToolkit`

`eigenius-julia-symbolics` crate, instantiating the D14 institution protocol ([D14 §3, §4, §8](d14-institution-realisation.md)): commits an `Institution` resource, the §4.1.1 resource classes, the §4.1.2 ExportFormat/ImportFormat declarations, the §4.1.3 QueryClass declarations, and a Rust `Institution` trait implementation. End-to-end demo: a notebook that loads physical-system equations, gets them simplified via an `OnDemand` `qc_symb_simplify` query (or via FIBER), then runs a numerical solve via a substrate component. The first AutoOnLoad QueryClass on `SymbolicallyReducesTo` exercises D14's commit-time validation path.

Depends on D14 milestones M1–M7 ([D14 §13.4](d14-institution-realisation.md)) and substrate Phase C.

**Scope:** Medium.

### Phase E — Second institution: `JuMP`

`eigenius-julia-jump` (per-solver registrations: HiGHS, GLPK, Ipopt). The §4.2 D14 declarations land. Solver-choice is realised by separate `Institution` resources sharing the worker infrastructure. Demo: a constrained design problem, declared in Eigon as an `OptimisationProblem`, solved by the institution via an `OnDemand` `qc_jump_solve` query, optimum committed as an `OptimisesTo` resource that the AutoOnLoad `qc_jump_validate_optimum` re-checks before admission.

**Scope:** Medium.

### Phase F — Third institution: `IntervalArithmetic` + numerical hardening

`eigenius-julia-intervals`. Strict-determinism mode (BLAS pinning, FMA off, refusal to run on non-conforming hosts). Cross-host reproducibility verification tooling.

**Scope:** Open-ended, driven by deployment needs.

### Phase G — Lean / Julia bridge

The mirror-equivalence check (§6.1). Joint `EigonFFI` + `JuliaPackageMirror` consistency tests. Worked example: a Julia FEA stress analysis paired with a Lean proof of the analysis pipeline's correctness.

**Scope:** Medium. Mostly the cross-integration work; both substrates are by then mature.

## 9. Open questions

The substrate's open questions (substrate doc §14) apply directly. Julia-specific additions:

1. **Scope of `eigon-julia-gen`.** Mirror every Eigon class on each generator run, or only the classes used by registered scripts and packages? Scoped is faster to generate and ship; comprehensive is easier for users and for cross-team reuse.

2. **Random seeds in stochastic Julia code.** Julia's `Random` module is the default, but many scientific packages have their own RNG handling (`StableRNGs.jl`, `Distributions.jl`, `Turing.jl`'s sampler-specific RNGs). A `JuliaInvocation.random_seed` field is too coarse if the script uses multiple RNGs with different states. Worth designing a `JuliaInvocation.rng_states: List<{name, seed, algorithm}>` extension.

3. **Multiple-dispatch ambiguity.** Julia rejects calls when no most-specific method exists. The substrate maps this to `DispatchAmbiguity` (substrate doc §11.1). But ambiguity can also arise *between* the user's script and a downstream package update — the same script that resolved fine in env v1 might be ambiguous in env v2. The boundary check could pre-resolve the dispatched method at script-publish time and record it; rejection then surfaces at publish, not at run. Worth deciding before Phase B.

4. **Compilation / precompilation policy.** Julia's first-call-after-load compilation is famously slow for some workloads. The image-build pipeline runs `Pkg.precompile()` so worker spawn doesn't pay it, but invocations that exercise method bodies not hit by precompilation still see latency. Accept and document, or invest in `PrecompileTools.jl`-driven workload coverage during build?

5. **Solver choice for `JuMP`.** The default solver registration question: HiGHS (open-source, broadly capable), GLPK (open-source, lighter), Ipopt (NLP only), or Gurobi (proprietary, fastest but license-locked). Probably HiGHS as the default; deployments register others as needed.

6. **Error preservation for Symbolics expressions.** When `SimplificationDiverged` fires (the rewrite system entered a cycle or didn't terminate), what does the institution return? The intermediate expression after a configurable step limit, the original expression with a flag, or a structured error? Affects how downstream consumers handle partial simplification.

7. **Interval-arithmetic determinism.** Different rounding modes can produce different bounds (still rigorous, but tighter or looser). The substrate's image pin captures the rounding mode by pinning the Julia + library versions, but this is worth documenting explicitly so users don't expect bit-identical bounds across IntervalArithmetic library versions.

---

*This outline complements [`runtime-substrate.md`](runtime-substrate.md) and [`d14-institution-realisation.md`](d14-institution-realisation.md). The substrate doc covers the language-agnostic machinery; D14 covers the institution protocol; this doc covers the Julia-specific layer plus the per-institution declarations that make Julia interesting beyond "another runtime." With D14 in place, no per-institution contract document is needed — the typed declarations and trait implementation are the contract. The next deliverables are the faithful-translation specification for `eigon-julia-gen` and the per-institution worker bootstraps, alongside the implementation work.*

---

## Appendix A: Julia ecosystem context

### A.1 Tooling baked into Julia images

- **`juliaup`** — the canonical Julia version manager. The build pipeline uses it to install the exact pinned Julia version per `JuliaEnvironment` image.
- **`Pkg.jl`** — the standard package manager. `Pkg.instantiate()` materialises a `Manifest.toml` into a working depot. Run inside the image build; the precompiled depot is part of the image layer.
- **`PrecompileTools.jl`** — proactive precompilation. Run during image build so first-call latency at worker spawn is bounded by image-pull + JIT-warmup, not by package precompilation.
- **`CBOR.jl`** — CBOR support; in-flight RPC marshalling per substrate doc §8.1. Uses RFC 8746 typed-array tags for large numerical arrays.
- **`JSON3.jl`** — JSON support; used for the Phase A bootstrap before CBOR worker codecs land.
- **`Distributed.jl` / `Malt.jl`** — process-pool primitives. The Julia worker's intra-container patterns borrow from these.

### A.2 Reasoning libraries — wrapping priority

- **`Symbolics.jl` + `ModelingToolkit.jl`** — Phase D. Output: `SymbolicallyReducesTo`, `Substitutes`, `SimplifiesTo` morphisms.
- **`JuMP`** — Phase E. Output: `OptimisesTo`, `Infeasible`, `BoundedBy`.
- **`IntervalArithmetic.jl`** — Phase F. Output: `BoundedBy(value, interval)`, `ProvesBoundOn(function, domain, interval)`.
- **`SciML` / `DifferentialEquations.jl`** — likely fourth, when domain demand exists.
- **`Catalyst.jl`** — domain-specific institution.
- **`Turing.jl`** — probabilistic programming. Bayesian-inference institution.

### A.3 Cross-language considerations

- **Calling Python from Julia** (`PyCall.jl`, `PythonCall.jl`) — works, but introduces Python's runtime into the trust surface. Not used in the v1 substrate; Python integration gets its own substrate when scoped (substrate doc Phase D).
- **Calling C / C++ from Julia** — first-class via `ccall`. The substrate doesn't restrict this; institution registrations declare their C dependencies in the `Manifest.toml` chain, which already pins them by hash.
- **Calling Julia from elsewhere** — `libjulia` is callable from C and has a stable ABI. Not used by the integration (the substrate hosts Julia workers; the orchestrator dispatches into them).

This appendix should be revisited annually, or when any of the surveyed projects undergoes a major release.
