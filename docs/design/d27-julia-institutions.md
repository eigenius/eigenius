# Julia Institutions

**Status:** Implemented (Phase 19; five reference institutions live — Symbolics, IntervalArithmetic, Catalyst, DiffEq, JuMP — with Catalyst→DiffEq and Symbolics→JuMP comorphisms)
**Scope:** What it takes to bring Julia up as the first concrete instance of the [runtime substrate](d26-runtime-substrate.md), and to register specific Julia libraries as Eigenius institutions on top of it under the [D14 institution protocol](d14-institution-realisation.md). Covers the Julia-specific resource subclasses, the `eigon-julia-gen` mirror generator, five reference institutions (`Symbolics` / `ModelingToolkit`, `JuMP`, `IntervalArithmetic`, `Catalyst`, `DiffEq` ODEs), and the future Lean / Julia bridge.
**Related:** [`d14-institution-realisation.md`](d14-institution-realisation.md) (the institution protocol — typed declarations, trait surface, dispatch model, Verdict, Comorphism shape — that each Julia institution instantiates), [`d26-runtime-substrate.md`](d26-runtime-substrate.md) (the language-agnostic substrate this layers on), [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) (the proof-bearing institution the Julia integration eventually pairs with), `boundary-contracts.md` (meta-spec context — under D14 the per-institution `BoundaryContract` collapses into typed declarations + Verdict; see §5)

## 1. Purpose and scope

The runtime substrate ([`d26-runtime-substrate.md`](d26-runtime-substrate.md)) is what makes Julia code executable inside Eigenius with full provenance — `RunRuntimeScript` and `CallRuntimeMethod` components, content-addressed `RuntimeScript` and `RuntimePackage` resources, OCI-image-pinned `RuntimeEnvironment` resources, all the boundary-check and worker-pool machinery. That gets Julia onto the platform.

This document covers what comes after: **what makes Julia interesting beyond "another runtime."** Specifically:

- The Julia-specific subclasses of the substrate's parent resource classes (`JuliaScript extends RuntimeScript`, etc.).
- The `eigon-julia-gen` mirror generator and its faithful-translation specification.
- Five reference institutions wrapping Julia libraries that implement formal reasoning systems with their own fibers:
  - **`Symbolics` / `ModelingToolkit`** — symbolic algebra, equation simplification, substitution.
  - **`JuMP`** — optimisation with solver-side certificates.
  - **`IntervalArithmetic`** — rigorous numerical bounds.
  - **`Catalyst`** — chemical reaction networks (conservation laws, deficiency theorems, mass-action / jump kinetics).
  - **`DiffEq`** — ODE solving with integration certificates (v1 scope: ODEs only).
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

Under D14, every institution registers by committing five kinds of typed Resources to the layer chain: an `Institution` (identity + runtime kind), `ExportFormat`s (typed extractions of class instances into EigenTT payloads), `ImportFormat`s (typed constructors of class instances from EigenTT payloads), `QueryClass`es (typed functions in the institution's fibre with `dispatch_role` of `OnDemand` / `AutoOnLoad` / `Decidable` and a result class — `Verdict` for the gate-on-commit and decide-procedure roles), and `Comorphism`s (triples `(s, m, t)` where `s` is an ExportFormat, `m` is a EigenTT Component, and `t` is an ImportFormat — the cross-institution bridge). The institution implements an `Institution` Rust trait with three methods: `extract_typed`, `reify`, and an optional `query`. Each Julia institution this doc describes — `Symbolics`/`ModelingToolkit`, `JuMP`, `IntervalArithmetic` — is its own D14 institution: one `Institution` resource per crate, its own typed declarations, its own trait implementation. They share the substrate's authoring-side machinery (image-pinned environment, mirror generator, worker pool) but they are independent reasoning systems at the institution-protocol level.

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

The "generator" is **substrate Rust code**, not a separate CLI binary. It runs as part of the substrate's image-build pipeline (D26 §9.2): when `build_environment_image` walks the chain to assemble the `JuliaEnvironment`'s image, a generator pass walks the relevant ontology classes and emits the Julia source mirror. The output gets committed as a `JuliaPackageMirror` resource (content-addressed) and baked into the env image. Earlier drafts of D27 framed the generator as `eigon-julia-gen`, an externally-runnable tool — that framing is misleading. Auditability comes from the *deterministic output spec* ([D29](d29-eigon-julia-mirror-spec.md)), not from the generator being an out-of-process binary; anyone with the substrate source + the layer chain can re-derive byte-identical mirror source. There is no v1 use case for invoking the generator outside the image-build pipeline.

The substrate POC and the mirror generator ship together as a single milestone (Phase 19a in [implementation-plan.md](implementation-plan.md)) because the worker's dispatch contract is shaped by whether mirrors exist — separating them would force two passes over the worker-side dispatch logic, and the interdependency is tight enough that addressing them in one milestone is cleaner than the original 19a/19b split.

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

Five Julia libraries that wrap as Eigenius institutions cleanly under D14 (Symbolics/MTK, JuMP, IntervalArithmetic, Catalyst, DiffEq). Each is its own crate (`eigenius-julia-symbolics`, `eigenius-julia-jump`, `eigenius-julia-intervals`, `eigenius-julia-catalyst`, `eigenius-julia-diffeq`) depending on `eigenius-julia` (the language crate, which depends on `eigenius-runtime-substrate`). Each crate ships:

1. An `Institution` resource declaring its IRI and its `External` runtime kind (it dispatches into a substrate-hosted Julia worker rather than running in-process).
2. A set of **resource classes** representing intra-fibre structure (the relations / typed claims the institution contributes).
3. **`ExportFormat`** declarations turning input class instances into EigenTT payloads the institution's worker consumes.
4. **`ImportFormat`** declarations constructing result-class instances from worker-produced EigenTT payloads.
5. **`QueryClass`** declarations with appropriate `dispatch_role`s — `AutoOnLoad` for relations the institution validates on commit, `OnDemand` for queries fired from EigenQL FIBER, `Decidable` for predicates referenced from `Exp::NativeDecide`.
6. Optionally **`Comorphism`** declarations bridging into other institutions (Julia ↔ Lean, Julia ↔ Julia, Julia → external).
7. A Rust `Institution` trait implementation. `extract_typed` / `reify` marshal between Eigon resources and CBOR-encoded EigenTT payloads on the substrate RPC; `query` dispatches procedure IRIs to the worker over substrate RPC.

### 4.0 IRI conventions

Every Phase-19 institution lives under the substrate-prefixed namespace `urn:eigenius:julia:<inst>:`. The institution *is* its Julia implementation under D14 — a hypothetical Python competitor implementing the same fibre would be a *different* institution with a different IRI (e.g. `urn:eigenius:python:sympy:`), composed via Comorphism, not by sharing a fibre namespace. The substrate prefix makes the implementation origin auditable from the IRI alone.

Within each institution, the local-name structure is:

| Kind | IRI form | Example |
|---|---|---|
| Institution itself | `urn:eigenius:julia:<inst>` | `urn:eigenius:julia:intervals` |
| Resource class | `urn:eigenius:julia:<inst>:<ClassName>` | `urn:eigenius:julia:intervals:BoundedBy` |
| Property | `urn:eigenius:julia:<inst>:<property_name>` | `urn:eigenius:julia:intervals:lower` |
| ExportFormat | `urn:eigenius:julia:<inst>:ef_<name>` | `urn:eigenius:julia:intervals:ef_bounded_by` |
| ImportFormat | `urn:eigenius:julia:<inst>:if_<name>` | `urn:eigenius:julia:intervals:if_bounded_by` |
| QueryClass | `urn:eigenius:julia:<inst>:qc_<name>` | `urn:eigenius:julia:intervals:qc_validate_bounded_by` |
| Worker procedure | `urn:eigenius:julia:<inst>:proc:<verb>` | `urn:eigenius:julia:intervals:proc:validate_bounded_by` |

The §4.1–§4.5 tables below use the bare `ef_` / `if_` / `qc_` short names for readability; the canonical IRIs follow this convention.

### 4.0.1 Payload-type simplification for v1

D14's `payload_type` is meant to be a EigenTT type — a primitive (`core:float`), an InductiveType (`Verdict`, a tuple-shaped inductive), or a class IRI (whose dependent-record type is induced from `requires`). The §4.x tables below describe payloads as tuples (`(Float, IntervalRepr)`, `(SymbolicTerm, SymbolicTerm, RuleSetIri)`) for narrative clarity. In v1 the registration IRIs use the resource's own class as the payload type — `extract_typed` is structurally identity, returning the resource's CBOR-encoded shape verbatim, and the language-side handler decomposes into library-native values. Once Phase-11b inductive types are in widespread use across the chain, the typed-tuple payloads become declarable as InductiveTypes and the registrations can be tightened. The kernel does not require the v1 simplification — it just keeps initial registrations from forcing tuple-shaped inductives ahead of demand.

### 4.0.2 Library version pins and verified API surface

The §4.1–§4.5 vocabulary is verified against installed Julia packages, not training-data recall. The probing script lives at [`julia/research/introspect-libraries.jl`](../../julia/research/introspect-libraries.jl) and writes a survey to [`api-survey.md`](../../julia/research/api-survey.md) in the same directory; refresh both whenever a Julia institution's pinned versions move.

**Versions verified as of 2026-05-03**: IntervalArithmetic 1.0.8, IntervalRootFinding 0.6.3, Symbolics 7.21.0, ModelingToolkit 11.24.1 (note v11 — `states` was renamed to `unknowns` back in v9), SymbolicUtils 4.25.2, JuMP 1.30.1, MathOptInterface 1.51.0, Catalyst 16.1.1, OrdinaryDiffEq 6.111.0, SciMLBase 2.155.1.

**Cross-cutting findings** (those that affect more than one institution):

- **`@variables` is exported by Symbolics, JuMP, MTK, Catalyst, and ModelingToolkitBase** — five-way conflict. Per-institution Julia handler code must qualify the macro by module. Worker-side dispatch needs to be written assuming the worker has all of these in scope.
- **`@parameters` is NOT in Symbolics** — it's MTK / ModelingToolkitBase / Catalyst. So "parameter" vocabulary belongs to the MTK institution, not the Symbolics one. The §4.1 (Symbolics) declarations should not reach for it.
- **`OrdinaryDiffEq`, not the `DifferentialEquations` umbrella, is the right `using` for the DiffEq institution image**. The umbrella pulls SDE/DAE/jump dependencies the v1 ODE-only institution doesn't need.

### 4.1 `Symbolics` / `ModelingToolkit` — symbolic algebra and equation simplification

The fibre of symbolic expressions has structure: equivalences modulo a rule set, substitutions, simplifications to canonical forms. The library implements the rule set; the institution wraps the dispatch.

#### 4.1.1 Resource classes (sentences in this fibre)

- **`SymbolicExpression`** — a symbolic-algebra expression resource. Carries the expression's typed term (a EigenTT inductive shape representing the expression tree) and the rule set IRI it belongs to.
- **`SymbolicallyReducesTo`** — relates two `SymbolicExpression` resources under a rule set. Carries `expr1`, `expr2`, `rule_set`. The kernel auto-validates this on commit (§4.1.3 below).
- **`Substitutes`** — relates `(expr, var, value, result)`. Auto-validated on commit.
- **`SimplifiesTo`** — relates `(expr, simplified_form, rule_set)`. Auto-validated by re-running `simplify` and confirming the claimed simplified form. **`simplified_form`, not `normal_form`** — Symbolics 7's `simplify` is *heuristic*, not normalising; calling it twice on the same expression need not converge to a unique canonical form, and `simplify(a) == simplify(b)` does not decide algebraic equivalence. The institution must not promise normal forms.
- **`SatisfiesEquation`** — relates `(expr_lhs, expr_rhs, rule_set)`. "Both sides reduce to the same simplified form" is the default-rewrite check; the institution returns `Verdict::Holds` on convergence, `Verdict::Undecidable` on disagreement (since `simplify` is heuristic). Hard equivalence over polynomial fragments goes through `Symbolics.groebner_basis` / `polynomial_coeffs`, returning `Holds`/`Fails` cleanly.

**Verified API note (Symbolics 7.21 / SymbolicUtils 4.25 / MTK 11.24 / Latexify 0.16, 2026-05-03)**: `Num`, `Equation` (with `(:lhs, :rhs)` fields, `~` infix constructor), `simplify`, `expand`, `substitute`, `get_variables`, `derivative`, `jacobian`, `hessian`, `gradient`, `polynomial_coeffs`, `groebner_basis`, `Differential`, `expand_derivatives` are all in Symbolics. **`@parameters` is in MTK / ModelingToolkitBase / Catalyst, NOT Symbolics** — parameter declaration belongs to the MTK part of the institution. **`SymbolicUtils.Pow` does NOT exist** in v4 (the `BasicSymbolic` union folded it into other cases); the SymbolicTerm EigenTT inductive should mirror `(Sym, Term, Add, Mul, Const)` plus power-as-application, not `(Sym, Term, Add, Mul, Pow, …)`. `RuleSet` and `@rule` exist but are *power-user extensibility hooks*, not a stable named catalog the institution can treat as IRI-able rule sets — `rule_set` IRIs in the resource shapes above must be interpreted as "the institution's pinned rewriter configuration" (a registration-time parameter), not per-resource discriminators.

#### 4.1.2 ExportFormats / ImportFormats

| Direction | IRI | Payload type | Procedure |
|---|---|---|---|
| Export from `SymbolicExpression` | `ef_symb_expr` | `SymbolicTerm` (EigenTT inductive) | `urn:eigenius:symbolics:extract_expr` |
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

The fibre has *transitivity* and *equivalence* structure that substrate components alone don't expose. A `SymbolicallyReducesTo` resource is a typed relation that the institution validates (the AutoOnLoad QueryClass), discovers (an OnDemand "find common form" query could return candidate reduction pairs), and that downstream programs can reason about transitively. Substrate `RunRuntimeScript` would just run a script and produce a result — it would not give that result the typed-relation status that lets EigenQL FIBER traverse a chain of `SymbolicallyReducesTo` resources or that lets EigenTT's `NativeDecide` reduce a propositional equivalence.

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

- **`BoundedBy`** — relates `(value, interval)`. v1 semantics: the institution validates `interval.inf ≤ value ≤ interval.sup` on commit (the typing/sanity use, which is what grounds the kinase IC50-with-CI columns). The full "value's true magnitude is guaranteed to lie in the interval" reading depends on a `derivation` linking `value` to a function the institution can interval-extend; that's added when Phase 19d (Symbolics) lands and `function` becomes typed.
- **`ProvesBoundOn`** — relates `(function, domain, interval)`. The interval extension of `function` over `domain` is bounded by `interval`. Auto-validated by re-running the institution's interval extension and confirming `interval` is at least as tight as what the institution computes (rejecting if the claim is tighter than provably possible). v1: `function` is carried as `function_source` (a Julia source string, anonymous-function literal `c -> ...`); 19d tightens this to a typed `SymbolicExpression` reference.
- **`ContainsRoot`** — relates `(function, domain)` with an interval-Newton-style witness that a root exists in the domain (or, dually, that no root exists). Auto-validated.

**Verified API note (IntervalArithmetic 1.0.8 / IntervalRootFinding 0.6.3, 2026-05-03)**: endpoint accessors are **`inf(x)` / `sup(x)`** (the `Interval` ontology class uses these as property names). `lower`/`upper`/`infimum`/`supremum`/`lo`/`hi` are NOT defined in IA 1.0; `bounds(x)` returns the pair as a tuple. Construction is `interval(lo, hi)` (lowercase, validated); `Interval(lo, hi)` errors — only single-arg `Interval(::Real)` exists. The `..` infix syntax lives in IntervalSets, not IntervalArithmetic. **Decoration** (`com` / `dac` / `def` / `trv` / `ill` and the `_NG` Not-Guaranteed flag) is real and printed; v1 uses common-decoration intervals exclusively and does not surface decoration as an ontology property. **`ContainsRoot` belongs to IntervalRootFinding.jl**, not IntervalArithmetic — `roots`, `Krawczyk`, `Newton`, `Bisection`, `Root`, `RootProblem`, `root_status` are exported there. The institution image must include both packages.

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

### 4.4 `Catalyst` — chemical reaction networks

Catalyst is structurally interesting for an institution because reaction networks aren't just symbolic ODEs — the *network* has its own algebraic structure: stoichiometry, conservation laws, deficiency theorems, mass-action vs. propensity-based kinetics. That structure is the institution's fibre. Promoted to a first-class reference institution because the life-science use cases (PK / PD, signaling pathways, metabolic networks) lean on it heavily.

#### 4.4.1 Resource classes

- **`ReactionNetwork`** — typed network: species, reactions, rate laws, parameters.
- **`ConservationLaw`** — typed linear invariant on species (a vector in the left-nullspace of the stoichiometry matrix). Auto-validated on commit by recomputing the conservation matrix and checking the claimed law lies in its row span.
- **`SteadyState`** — relates `(network, parameter_assignment, species_concentrations)`. Auto-validated on commit by re-solving the steady-state system at the given parameters.
- **`MassActionKinetics`** / **`JumpProcessSemantics`** — discriminator-style markers for the *compilation path*, not network properties. A `ReactionNetwork` is a single object; the discriminator selects whether it gets compiled to `ODEProblem` (mass-action ODEs) or `JumpProblem` (stochastic kinetics) — affects which D14 ImportFormat / Comorphism to fire, not the network's identity.
- **`DeficiencyZero`** / **`DeficiencyOne`** — claims that the network's structural deficiency equals 0 / 1. Auto-validated by calling `Catalyst.deficiency(rn)` (returns `Int`) and comparing. Catalyst does NOT export theorem-named entry points (`deficiencyzerotheorem` / `deficiencyonetheorem`); the validation is a numeric comparison, not a theorem check. `Catalyst.isweaklyreversible(rn)` and `Catalyst.iscomplexbalanced(rn)` are also exported and could underpin secondary AutoOnLoad classes if motivated.

**Verified API note (Catalyst 16.1.1 / MTK 11.24.1, 2026-05-03)**: `@reaction_network` returns a `ReactionSystem`. Exported and confirmed: `species`, `parameters`, `reactions`, `equations`, `unknowns`, `netstoichmat`, `substoichmat`, `prodstoichmat`, **`conservationlaws`** (returns `Matrix{Int64}`), `conservedequations`, `conservationlaw_constants`, `complexstoichmat`, `reactioncomplexes`, `deficiency`, `isweaklyreversible`, `iscomplexbalanced`. Catalyst pulls in 426 names from MTK + extensions.

**Catalyst → ODE / SDE / Jump compilation pipeline** (probe results in [`julia/research/catalyst-ode-probe.md`](../../julia/research/catalyst-ode-probe.md), confirmed against an SciML expert):

- `convert(ODESystem, rn)` is **dead** in Catalyst 16 / MTK 11 (the `ModelingToolkitBase.IntermediateDeprecationSystem` layer).
- The canonical replacement family is **`ode_model(rn)` / `jump_model(rn)` / `sde_model(rn)`** — explicit model constructors that produce symbolic `ODESystem` / `JumpSystem` / `SDESystem` objects respectively, performing rate-law generation and combinatoric handling. This is the entry point for any Catalyst → MTK / Catalyst → Symbolics Comorphism that needs the symbolic system.
- For direct compilation to a solvable problem: **`ODEProblem(rn, u0_map, tspan, p_map)`** with **map-form** `u0` and `p`: `[species_sym => value, ...]` and `[param_sym => value, ...]`. Positional-vector form errors with `BoundsError` — this is a deliberate consequence of MTK 11's lazy indexing on hierarchical systems, not a bug. The Catalyst → DiffEq Comorphism uses this map-form path direct to `OdeProblem`, no `ODESystem` intermediate needed.
- **`complete(rn)`** is required best-practice before any model conversion or Problem construction (incomplete systems are treated as open / hierarchical and trigger warnings or errors). Standard pipeline: `rn = complete(flatten(rn))` then `ode_model(rn)` or `ODEProblem(rn, u0_map, …)`.
- Other useful Catalyst 16 exports: `oderatelaw` (symbolic rate law for a single reaction; useful diagnostic), `balance_system` (conservation laws + reduced-rank system; alternative to the `conservationlaws(rn)` matrix path the institution currently uses), `make_si_ode` (Species-Indexed ODE — sparse-solver-optimised, niche), `ss_ode_model` (steady-state ODE for `NonlinearProblem`; relevant for the institution's steady-state work), `symbolic_solve_ode` (analytic solver for simple linear-rate systems; not needed v1).
- **Structural simplification policy for v1**: do NOT call `structural_simplify` at the institution boundary by default. Keep the user-named species and parameters as the system's variables, so `species_declared` aligns with the state vector and downstream `SteadyState` claims map back cleanly. v2 may add an opt-in `structurally_simplified: bool` flag on `ReactionNetwork` or the produced `OdeProblem`, with the institution exposing eliminated quantities via `observed(sys)`.
- **`JumpProblem(rn, u0_map, tspan, p_map)` and `SDEProblem(rn, u0_map, tspan, p_map)`** are the v2-scope direct entry points for stochastic mass-action and Chemical Langevin compilation; same map-form discipline.

#### 4.4.2 ExportFormats / ImportFormats

| Direction | IRI | Payload type | Procedure |
|---|---|---|---|
| Export from `ReactionNetwork` | `ef_cat_network` | `CatalystNetworkRepr` | `urn:eigenius:catalyst:extract_network` |
| Export from `ConservationLaw` | `ef_cat_conservation_law` | `(NetworkRepr, Vec<Coef>)` | `urn:eigenius:catalyst:extract_conservation_law` |
| Import to `SteadyState` | `if_cat_steady_state` | `(NetworkRepr, Params, Vec<Float>)` | `urn:eigenius:catalyst:reify_steady_state` |
| Import to `OdeSystemRepr` (handoff to DiffEq) | `if_cat_to_ode` | `OdeSystemRepr` | `urn:eigenius:catalyst:reify_ode` |

#### 4.4.3 QueryClasses

| QueryClass | `query_class` | `result_class` | `dispatch_role` | Implementation |
|---|---|---|---|---|
| `qc_cat_validate_conservation_law` | `ConservationLaw` | `Verdict` | `AutoOnLoad` | institution-runtime — re-derives the network's conservation matrix and checks the claimed law. |
| `qc_cat_validate_steady_state` | `SteadyState` | `Verdict` | `AutoOnLoad` | institution-runtime — re-solves at the given parameters. |
| `qc_cat_compute_steady_states` | `CatalystSteadyStateInput` | `SteadyStateSet` | `OnDemand` | institution-runtime. |
| `qc_cat_to_ode` | `ReactionNetwork` | `OdeSystemRepr` | `OnDemand` | institution-runtime — the bridge into the DiffEq institution (§4.5). |
| `qc_cat_check_deficiency` | `ReactionNetwork` | `Verdict` | `OnDemand`, `Decidable` | institution-runtime. |
| `qc_cat_extract_invariants` | `ReactionNetwork` | `ConservationLawSet` | `OnDemand` | institution-runtime. |

#### 4.4.4 Comorphism into Symbolics / ModelingToolkit

Catalyst is built on top of ModelingToolkit; a `ReactionNetwork → ODESystem` translation is a built-in capability of the library. Under D14 we declare this as a typed `Comorphism` resource so the cross-fibre move is a tracked translation rather than an ad-hoc Julia call. Sketch:

```
Comorphism ρ_catalyst_to_mtk
  export_format:  ef_cat_network
  transformation: cm_catalyst_network_to_mtk_ode  (Julia component)
  import_format:  if_mtk_ode_system
  exact: true
  description: "Mass-action-kinetics translation from a reaction network
                to a Modelling-Toolkit ODE system."
```

#### 4.4.5 Why this is an institution

The fibre has structural invariants — linear conservation laws are independent of parameters; deficiency is a topological invariant of the network; steady-state existence has structural sufficient conditions — that EigenQL `FIBER` queries can traverse. Substrate components alone would just return numbers; the institutional shape is what lets a downstream program reason about "all networks satisfying conservation law L" or "all deficiency-zero networks in this layer."

### 4.5 `DifferentialEquations.jl` — ODE solving

The "ODE solution" fibre has structure: convergence proofs, error bounds, step refinement, sensitivity analysis. Promoted to a first-class reference institution because life-science PK / mechanistic modelling cannot be served by substrate components alone — solutions need to be *typed claims* ("this trajectory is the solution to that compartmental model"), not just byte streams.

**v1 scope: ODEs only.** SDEs, DAEs, DDEs, jump processes, and hybrid systems are deferred to follow-on work; the full DiffEq surface is enormous and the bread-and-butter life-science modelling (PK compartmental models, mechanistic dose-response, steady-state cell-cycle) is ~95% deterministic ODE. SDE / jump-process kinetics warrant a separate institution when low-copy-number stochastic kinetics are the consumer.

#### 4.5.1 Resource classes

- **`OdeProblem`** — typed problem: function, initial conditions, time span, parameters, optional Jacobian. *Renamed from D27's earlier `OdeSystem`*: in the SciML ecosystem `ODESystem` is the symbolic MTK abstraction; `ODEProblem` is the concrete solvable thing — `OdeSystem` would collide with MTK semantics once the Symbolics/MTK institution is wired up.
- **`OdeSolution`** — relates `(problem, parameters, initial_conditions, time_span, integrator, trajectory)`. The trajectory is content-addressed; the integrator records algorithm, abstol, reltol, step strategy.
- **`ReproducibleIntegration`** — relates `(solution, algorithm, abstol, reltol, trajectory_hash)`. *Renamed from `IntegrationCertificate`*: DiffEq's adaptive tolerances are heuristic local-truncation bounds, not rigorous global enclosures. "Certificate" oversells what the institution can deliver. Auto-validated by re-solving with the same `(alg, abstol, reltol)` against a host with matching `numerical_metadata` and confirming the trajectory hash matches. The `IntegrationCertificate` / `ValidatedIntegration` IRIs stay reserved for a future TaylorModels-backed institution that produces actual interval-rigorous enclosures.
- **`BoundedError`** — *removed from this institution*: not native to DiffEq's vocabulary; rigorous norm bounds belong to the IntervalArithmetic / TaylorModels institutions. The `sol.errors` field DiffEq exposes is only populated when the problem carries an analytic solution (`u_analytic`), which is a niche case.
- **`ParameterFit`** — *moved to the JuMP / Optimization institution scope*: in 2026 SciML, parameter fitting is `Optimization.jl` building a loss from `solve(remake(prob; p=θ))` rather than `DiffEqParamEstim` (stagnant). The ontology class lives at the cross-institution boundary; DiffEq exports the `OdeProblem` and consumes back fitted parameters via comorphism.

**Verified API note (OrdinaryDiffEq 6.111 / SciMLBase 2.155, 2026-05-03)**: `ODEProblem`, `ODESolution`, `SteadyStateProblem`, `EnsembleProblem`, `remake`, `successful_retcode` are in SciMLBase. `Tsit5`, `Vern9`, `Rosenbrock23`, `Rodas5`, `Rodas5P`, `QNDF`, `FBDF`, `AutoTsit5`, `AutoVern9` are in OrdinaryDiffEq. **`ReturnCode.Success` is the success enum** — the full set is `(Default, Success, Failure, Terminated, MaxIters, DtNaN, MaxNumSub, DtLessThanMin, Unstable, InitialFailure, ConvergenceFailure, ExactSolutionLeft, ExactSolutionRight, FloatingPointLimit, Infeasible, MaxTime, InternalLineSearchFailed, ShrinkThresholdExceeded, Stalled, StalledSuccess, InternalLinearSolveFailed, APosterioriSafetyFailure)`. `ODESolution` fields: `(:u, :u_analytic, :errors, :t, :k, :discretes, :prob, :alg, :interp, :dense, :tslocation, :stats, :alg_choice, :retcode, :resid, :original, :saved_subsystem)`. **For trajectory content-addressing**, hash `(t, u, k, alg, alg_choice)` together — `k` is the per-segment interpolation coefficients; hashing only `(t,u)` loses dense-output reproducibility. The institution should `using OrdinaryDiffEq` (or specific sub-packages like `OrdinaryDiffEqTsit5`), NOT the heavier `using DifferentialEquations` umbrella.

#### 4.5.2 ExportFormats / ImportFormats

| Direction | IRI | Payload type | Procedure |
|---|---|---|---|
| Export from `OdeSystem` | `ef_diffeq_system` | `OdeSystemRepr` | `urn:eigenius:diffeq:extract_system` |
| Import to `OdeSolution` | `if_diffeq_solution` | `(OdeSystemRepr, Params, ICs, TimeSpan, Trajectory, IntegratorMeta)` | `urn:eigenius:diffeq:reify_solution` |
| Import to `IntegrationCertificate` | `if_diffeq_certificate` | `(SolutionRef, Tolerance, ErrorBound)` | `urn:eigenius:diffeq:reify_certificate` |
| Import to `ProvesBoundOn` (handoff to IntervalArithmetic) | `if_diffeq_to_interval_bound` | `(FunctionRepr, IntervalRepr, IntervalRepr)` | `urn:eigenius:diffeq:reify_to_interval_bound` |

#### 4.5.3 QueryClasses

| QueryClass | `query_class` | `result_class` | `dispatch_role` | Implementation |
|---|---|---|---|---|
| `qc_diffeq_validate_solution` | `OdeSolution` | `Verdict` | `AutoOnLoad` | institution-runtime — re-integrates and confirms within tolerance. |
| `qc_diffeq_validate_certificate` | `IntegrationCertificate` | `Verdict` | `AutoOnLoad` | institution-runtime. |
| `qc_diffeq_solve` | `DiffEqSolveInput` (system + params + ic + time span) | `OdeSolution` | `OnDemand` | institution-runtime. |
| `qc_diffeq_steady_state` | `OdeSystem` (with parameters) | `SteadyState` | `OnDemand` | institution-runtime. |
| `qc_diffeq_continuation` | `DiffEqContinuationInput` | `ParameterSweepSet` | `OnDemand` | institution-runtime — parameter sweep with bifurcation tracking. |
| `qc_diffeq_sensitivity` | `OdeSolution` | `SensitivityReport` | `OnDemand` | institution-runtime. |

#### 4.5.4 Comorphisms in and out

- **From `Catalyst` (§4.4):** `ReactionNetwork → OdeSystem` via `ρ_catalyst_to_mtk` (and `ρ_mtk_to_diffeq`), or directly via a Catalyst → DiffEq comorphism if Catalyst's mass-action kinetics translate to a DiffEq problem without a MTK round-trip.
- **From `Symbolics`/`ModelingToolkit` (§4.1):** `ODESystem → OdeSystem` is a direct compilation step in the libraries; under D14 it's a typed Comorphism.
- **To `IntervalArithmetic` (§4.3):** given an `OdeSolution` plus an interval-extension of the vector field, produce a `ProvesBoundOn` resource. This is the bridge for *operationally verified* PK / signaling predictions: bounds that hold by construction, not just by tolerance.

#### 4.5.5 Why an institution and not just a substrate component

A substrate component runs an integrator and returns a trajectory. An institution makes the *trajectory* a typed claim ("this is the solution to that system at those parameters within that tolerance"), validatable on commit, queryable through FIBER, composable with other institutions via Comorphism. The integration-certificate pattern (a re-checkable bound) is operationally similar to JuMP's certificate model — it's why the fibre has structure rather than just numbers.

### 4.6 Other Julia institutions worth considering

Each is its own design exercise; this document scopes only the five in §4.1–4.5.

- **SDE / DAE / jump-process subsets of `DifferentialEquations.jl`** — first follow-on to §4.5 once stochastic kinetics or differential-algebraic systems land as a domain need.
- **`Turing.jl`** — probabilistic programming. Bayesian-inference institution. Morphism types around posteriors and credible intervals are interesting and not yet thought through carefully.
- **`Manopt.jl`** — optimisation on Riemannian manifolds.
- **`HomotopyContinuation.jl`** — algebraic-geometry root finding with certified tracking.

When demand for one of these crystallises, it gets its own design doc and crate, plugging into `eigenius-julia` the same way the five reference institutions do.

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

These properties surface in trace metadata (the substrate's `RuntimeInvocation` provenance — see [`d26-runtime-substrate.md`](d26-runtime-substrate.md) §5.5) and in the verdict's auxiliary fields, not in a separate per-institution contract document.

## 6. The Lean / Julia bridge (future work)

Once both integrations are mature, a natural pattern: a Julia computation produces a *derived* result, and a Lean proof asserts a property about the *algorithm* that produced it (or an interval enclosing the expected result). Three concrete bridges are worth designing:

### 6.1 Aligned mirror packages

Lean's `EigonFFI` and Julia's `JuliaPackageMirror` for the same Eigon class need to be structurally aligned, so a claim about a `StressResult` in Lean is recognisable as a claim about the same `StressResult` Julia produced. Because both generators are deterministic and content-anchored to the same layer chain, a small "mirror equivalence" check at the institution boundary suffices: confirm the Lean mirror and the Julia mirror were generated from the same `source_layer` and that the relevant class has byte-identical structure in both.

Now that Lean's authoring side leverages the same runtime substrate as Julia ([`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) §2.3), this check is mechanically simple: `JuliaPackageMirror` and `LeanPackageMirror` both extend `RuntimePackageMirror`, so structural equivalence reduces to a comparison on the parent type's `source_layer` and `mirrored_classes` properties. The bridge crate doesn't need language-specific knowledge to verify alignment.

### 6.2 Verification of `IntervalArithmetic` outputs — concrete D14 `Comorphism`

A Lean proof asserting "the function `f` has, on domain `[a, b]`, output in `[lo, hi]`" can be paired with a Julia `IntervalArithmetic` invocation producing the same bounds. Under D14, the bridge is a `Comorphism` resource ([D14 §4.5 + §5](d14-institution-realisation.md)): a triple `(s, m, t)` where `s` is an ExportFormat from the Julia interval institution, `m` is a EigenTT Component carrying the validation logic, and `t` is an ImportFormat on the Lean institution constructing a `LeanProofTerm` whose proposition asserts the bound.

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

The harder bridge. A Lean proof asserts "this Julia function, modelled as a Lean function, computes the partial sum of a series correctly." The Julia function and the Lean function are different artifacts; correspondence between them is by *human-supplied translation*, not mechanical. Useful but trust-extended; out of scope for v1 but the place this leads. Structurally it would still be a D14 `Comorphism` — `s` extracting the Julia function repr (a `JuliaScript` ExportFormat), `m` packaging the human-supplied translation as a EigenTT term plus the matching Lean obligation, `t` constructing the resulting `LeanProofTerm`.

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

Per the substrate, CBOR on the wire. Julia side uses [`CBOR.jl`](https://github.com/JuliaIO/CBOR.jl) for marshalling. Dispatched-method recording uses `which()` to capture the actual method resolved by Julia's dispatcher.

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

### Phase E — Second institution: `IntervalArithmetic` + numerical hardening

`eigenius-julia-intervals`. *Reordered ahead of JuMP* — no solver-dependency surface, kinase CI columns map directly onto `BoundedBy`, and the Decidable role is the most novel piece of D14 runtime mechanics, so we want it exercised early. Strict-determinism mode (BLAS pinning, FMA off, refusal to run on non-conforming hosts). Cross-host reproducibility verification tooling.

**Scope:** Open-ended, driven by deployment needs.

### Phase F — Third institution: `JuMP`

`eigenius-julia-jump` (per-solver registrations: HiGHS, GLPK, Ipopt). The §4.2 D14 declarations land. Solver-choice is realised by separate `Institution` resources sharing the worker infrastructure. Demo: a constrained design problem, declared in Eigon as an `OptimisationProblem`, solved by the institution via an `OnDemand` `qc_jump_solve` query, optimum committed as an `OptimisesTo` resource that the AutoOnLoad `qc_jump_validate_optimum` re-checks before admission.

**Scope:** Medium.

### Phase G — Fourth institution: `DifferentialEquations.jl` — ODEs only

`eigenius-julia-diffeq`. *Reordered ahead of Catalyst* — Catalyst's `qc_to_ode` Comorphism has nowhere to land if DiffEq isn't ready first; with this ordering, 19g ships using hand-written compartmental ODEs (PK two-compartment is well-defined without Catalyst). **v1 scope: ODEs only**; SDEs / DAEs / jump processes are deferred follow-ons. Declarations from §4.5 (verified note): `OdeProblem` (renamed from `OdeSystem` to avoid MTK collision), `OdeSolution`, `OdeSteadyState`, plus `ReproducibleIntegration` framing for AutoOnLoad re-validation; `IntegrationCertificate` / `BoundedError` IRIs reserved for a future TaylorModels-backed institution. Comorphism out to Phase E (DiffEq → IntervalArithmetic) for operationally-verified bounds. Demo: a one-compartment PK clearance model integrated, validated on commit, optionally bounded via interval extension.

**Scope:** Medium-to-Large.

### Phase H — Fifth institution: `Catalyst`

`eigenius-julia-catalyst`. Promoted to a first-class reference institution because life-science PK / signaling pathways / metabolic networks lean on reaction-network modelling heavily. Declarations from §4.4: `ReactionNetwork`, `ConservationLaw`, `SteadyState`, `DeficiencyZero` / `DeficiencyOne`, `WeaklyReversible` / `ComplexBalanced`. AutoOnLoad QueryClasses validate conservation laws and steady states on commit. The Catalyst → DiffEq Comorphism — the load-bearing handoff — lands here using the symbolic-keyed map form `ODEProblem(rn, [sp => v, ...], tspan, [p => v, ...])` (the Catalyst-ODE probe verified this is the working entry point in Catalyst 16.1.1; the older `convert(ODESystem, rn)` is broken). Demo: a notebook that declares a reaction network, derives conservation laws via `qc_cat_extract_invariants`, computes a steady state, validates it on commit, and hands off to the DiffEq institution (already shipped in Phase G) for time-domain integration.

**Scope:** Medium.

### Phase I — Lean / Julia bridge

The mirror-equivalence check (§6.1). Joint `EigonFFI` + `JuliaPackageMirror` consistency tests. Worked example: a Julia FEA stress analysis (or PK trajectory) paired with a Lean proof of the analysis pipeline's correctness or of the trajectory's interval bounds.

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

8. **Comorphism transformation as EigenTT term** *(resolved per pre-existing design guidance — captured in D14 §4.5 + §5)*. The transformation slot in a Comorphism resource is a *EigenTT expression* (typed term), not an opaque `program:Component` IRI. The natural shape is `program:Lambda` whose body is a typed expression — pure transformations bottom at composed primitive forms (e.g. `λ Δg. exp(-Δg / RT) * 1e9` for Arrhenius, fully transparent); institution-runtime transformations bottom at a `program:Component` reference (Component-as-expression-form, dispatched through the substrate to the institution's worker). Kernel evaluation handles both via the existing Component-evaluation path. The Julia comorphism declarations in [`ontologies/julia/comorphisms/comorphisms.json`](../../ontologies/julia/comorphisms/comorphisms.json) follow this pattern: `transform_catalyst_to_ode_problem` and `transform_diffeq_to_interval_bound` are `program:Lambda` resources whose bodies Apply institution-routed Components (`compile_to_ode_problem`, `bound_trajectory`) to the bound parameter. **Cleanup TODO**: [`ontologies/examples/d14-dock-assay/dock-assay.json`](../../ontologies/examples/d14-dock-assay/dock-assay.json) still declares `cm_arrhenius` as `is_a program:Component` — the Arrhenius transformation is a *Lambda* (`λ Δg. exp(-Δg / RT) * 1e9`), not an opaque Component, and the demo file should be rewritten accordingly. Same for any other examples that drift from the EigenTT-term shape. The kernel-side enforcement (rejecting Comorphism resources whose `transformation` is `is_a program:Component` rather than an expression form) is a structural-validation rule that lands as the institution-ontology property declaration tightens.

---

*This outline complements [`d26-runtime-substrate.md`](d26-runtime-substrate.md) and [`d14-institution-realisation.md`](d14-institution-realisation.md). The substrate doc covers the language-agnostic machinery; D14 covers the institution protocol; this doc covers the Julia-specific layer plus the per-institution declarations that make Julia interesting beyond "another runtime." With D14 in place, no per-institution contract document is needed — the typed declarations and trait implementation are the contract. The next deliverables are the faithful-translation specification for `eigon-julia-gen` and the per-institution worker bootstraps, alongside the implementation work.*

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
- **`Catalyst.jl`** — Phase G. Output: `ConservationLaw`, `SteadyState`, `DeficiencyZero`/`DeficiencyOne` morphisms; comorphism into Symbolics/MTK and (via §4.5) into DiffEq.
- **`DifferentialEquations.jl` (ODEs only)** — Phase H. Output: `OdeSolution`, `IntegrationCertificate`, `BoundedError`; comorphism into IntervalArithmetic for *operationally verified* PK / signaling bounds.
- **SDE / DAE / jump-process subsets of `DifferentialEquations.jl`** — follow-on; lands when stochastic or differential-algebraic kinetics become a domain need.
- **`Turing.jl`** — probabilistic programming. Bayesian-inference institution; deferred until a posterior-over-PK-parameters use case asks for it.

### A.3 Cross-language considerations

- **Calling Python from Julia** (`PyCall.jl`, `PythonCall.jl`) — works, but introduces Python's runtime into the trust surface. Not used in the v1 substrate; Python integration gets its own substrate when scoped (substrate doc Phase D).
- **Calling C / C++ from Julia** — first-class via `ccall`. The substrate doesn't restrict this; institution registrations declare their C dependencies in the `Manifest.toml` chain, which already pins them by hash.
- **Calling Julia from elsewhere** — `libjulia` is callable from C and has a stable ABI. Not used by the integration (the substrate hosts Julia workers; the orchestrator dispatches into them).

This appendix should be revisited annually, or when any of the surveyed projects undergoes a major release.
