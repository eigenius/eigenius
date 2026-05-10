# Tutorial: the Catalyst institution and the kinase mechanism

This walkthrough wires the Catalyst institution end-to-end, using the classical Michaelis-Menten + competitive-inhibition reaction network — the *mechanistic* model under a kinase IC₅₀ assay — as the worked example. By the end you will have:

- Defined chain shapes for `ReactionNetwork` and `ConservationLaw`.
- Generated a Julia mirror that maps both onto typed Julia structs.
- Built the env image with the `EigeniusCatalyst` handler package + a precompiled `Catalyst.jl`.
- Committed the kinase reaction network to the chain.
- Committed three `ConservationLaw` claims (enzyme, substrate-or-product, inhibitor); the AutoOnLoad gate re-derived each from `Catalyst.conservationlaws(rn)` and produced three `Holds` verdicts.

The script form lives at [`demo/catalyst/run.sh`](../../../../demo/catalyst/run.sh) — `just demo-catalyst` runs it end-to-end. Read this if you want to understand what each step proves and where the broader kinase-modelling story fits in.

If you haven't seen [the intervals tutorial](intervals-institution-tutorial.md) yet, read that first — it covers the substrate plumbing (mirror, env build, AutoOnLoad dispatch) at a slower pace. This tutorial assumes those mechanics. The [Symbolics tutorial](symbolics-institution-tutorial.md) covers the chain-side typed-formula machinery from D32; Catalyst doesn't yet consume FormulaTerms (it carries network sources as Julia macro strings), so D32 is background rather than load-bearing here.

## What's different about Catalyst

Where the intervals institution gates a flat numerical resource and Symbolics gates expression trees, Catalyst gates **structural invariants of reaction networks**. A `ConservationLaw` claim is a vector of integers — and the question "is this vector a conservation law of this network" is a *linear-algebra row-span check* against the network's stoichiometry matrix, computed by `Catalyst.conservationlaws(rn)`. There's no heuristic involved: the answer is `Holds` or `Fails`. (No `Undecidable` in v1 — see [the verdict-policy note](#verdict-policy-no-undecidable-in-v1) below.)

The institutional value is what this gives downstream consumers:

- **Chain-typed conservation laws** become indexable. EigenQL queries can ask "which conservation laws hold in network N?" or "across all networks committed to layer L, which share a conservation law with coefficient pattern P?".
- **Reaction networks become first-class chain resources**, available as inputs to comorphisms. The `Catalyst → DiffEq` comorphism (D27 §4.4.4, still ahead) takes a `ReactionNetwork` and produces an `OdeProblem` for time-course simulation. The `Catalyst → Symbolics` comorphism (via ModelingToolkit) takes the same network and produces an `ODESystem` for symbolic manipulation. Both depend on `ReactionNetwork` being a typed chain shape — which is what this v1 lands.
- **The kinase assay's IC₅₀ measurements gain a mechanistic anchor.** The kinase-institutions notebook ([`notebooks/examples/kinase-institutions.json`](../../../../notebooks/examples/kinase-institutions.json)) is currently flat — IC₅₀ values for each (compound, target, protocol) row, no model behind them. With Catalyst on the chain, the *enzyme-kinetic mechanism* underneath each measurement can be a chain-committed network that downstream comorphisms can solve, fit, or compare. v1 doesn't wire that pipeline end-to-end (DiffEq isn't here yet), but it lays the network-and-conservation-law foundation that makes it possible.

## The kinase mechanism

The classical kinase-with-competitive-inhibition mechanism:

```
E + S  ⇌  ES  →  E + P    (Michaelis-Menten catalysis)
E + I  ⇌  EI              (competitive inhibition)
```

Six species — enzyme `E`, substrate `S`, enzyme-substrate complex `ES`, product `P`, inhibitor `I`, enzyme-inhibitor complex `EI`. Five rate parameters. Three structural conservation laws drop out of the stoichiometry:

| Conservation law | Coefficient vector (E, S, ES, P, I, EI) | Meaning |
|---|---|---|
| Enzyme | `[1, 0, 1, 0, 0, 1]` | `[E] + [ES] + [EI] = E_total` |
| Substrate-or-product | `[0, 1, 1, 1, 0, 0]` | `[S] + [ES] + [P] = S_total` |
| Inhibitor | `[0, 0, 0, 0, 1, 1]` | `[I] + [EI] = I_total` |

These aren't computed by the demo or the handler — they're *claims* the user (or in production, an authoring agent) writes down. The Catalyst institution then re-derives them from the network and confirms they're valid. Three commits, three verdicts, all `Holds`.

If you committed a fourth claim with coefficients that *aren't* in the conservation matrix's row span — say `[1, 1, 0, 0, 0, 0]`, claiming `[E] + [S] = const`, which the catalysis reaction `ES → E + P` violates — the gate would return `Fails` and reject the load. The chain stays consistent: every committed `ConservationLaw` is a verified structural invariant.

## Prerequisites

- The compose stack running: `EIGENIUS_MOCK_LLM=true docker compose up -d`.
- A reachable Docker daemon on the host. (See [the intervals tutorial](intervals-institution-tutorial.md#prerequisites) for the substrate-depot bind-mount detail.)
- `jq`.
- The workspace built once.
- **Patience for the cold env build.** Catalyst.jl pulls ModelingToolkit, SymbolicUtils, DiffEqBase, and a long tail of SciML dependencies; first-run `Pkg.precompile` takes ~10 minutes. Subsequent rebuilds hit buildah's layer cache and finish in seconds.

The institution sources used throughout live at [`julia/institutions/catalyst/`](../../../../julia/institutions/catalyst/):

```
julia/institutions/catalyst/
├── declarations/
│   ├── catalyst-ontology.eigon.json     # ReactionNetwork + ConservationLaw
│   │                                    # classes + their properties
│   └── catalyst-institution.eigon.json  # Institution + RuntimeMethodSignature
│                                        # + AutoOnLoad QueryClass
└── EigeniusCatalyst/
    ├── Project.toml                     # dep on Catalyst.jl + LinearAlgebra
    │                                    # + EigeniusMirror
    └── src/EigeniusCatalyst.jl          # validate_conservation_law handler
```

## Step 1 — Load the Catalyst ontology

```bash
eigenius --endpoint http://localhost:50051 \
    load julia/institutions/catalyst/declarations/catalyst-ontology.eigon.json
```

### What just happened

The ontology declares two classes:

| Class | Required properties | Role |
|---|---|---|
| `ReactionNetwork` | `network_source`, `species_declared`, `parameters_declared` | Chain carrier for a Catalyst.jl `ReactionSystem`. |
| `ConservationLaw` | `network`, `coefficients` | Claim: the coefficient vector is a left-nullspace row of the network. |

The ones worth pausing on:

- **`network_source: string, max_length: 16384`** — the verbatim `@reaction_network begin … end` macro text. Bounded length (16k) keeps the chain shape from absorbing arbitrary Julia programs; the handler additionally refuses sources that don't parse to a single `@reaction_network` macro call (see [§ Step 4](#step-4--read-the-handler) below).
- **`species_declared: value_array<string>`** — the canonical species ordering Catalyst's `species(rn)` returns (first-appearance order across the reactions). This is what `coefficients` indexes positionally; without a canonical ordering, two equally-valid orderings of the same network would produce different conservation-law coefficient vectors and the chain would lose substitutivity.
- **`coefficients: value_array<integer>`** — a flat integer vector. The Nth entry is the multiplier for the Nth species. The validator rejects mismatched lengths as `Fails`.

## Steps 2–6 — Mirror, env image, env Resource, install institution

Steps 2-6 follow the same shape as the [intervals](intervals-institution-tutorial.md#step-2--anchor-the-mirror-to-a-layer) and [symbolics](symbolics-institution-tutorial.md#steps-26--mirror-env-image-env-resource-institution) tutorials. Three Catalyst-specific notes:

### The mirror filter seeds two classes

```bash
eigenius mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["ReactionNetwork", "ConservationLaw"]
              RETURN [] { iri: ?iri }' \
    --language julia \
    --output /tmp/catalyst-mirror \
    --json
```

Closure walking pulls in the property definitions transitively. The mirror generator emits `struct ReactionNetwork` (with `network_source::String`, `species_declared::Vector{String}`, `parameters_declared::Vector{String}`) and `struct ConservationLaw` (with `network::ReactionNetwork`, `coefficients::Vector{Int64}`). Plus the standard `decode_*` / `encode_*` codecs and the `_eigenius_decoders` registry.

### The env build is slow

Cold runs take ~10 minutes — Catalyst.jl pulls ModelingToolkit + SymbolicUtils + DiffEqBase + RuntimeGeneratedFunctions + a long SciML dep tail, all of which `Pkg.precompile` runs against. The image gets cached by buildah on subsequent builds. If you're iterating on the handler code, the rebuild is fast because only the `EigeniusCatalyst` layer changes.

### The institution declares one QueryClass

Where Symbolics declared six QueryClasses across three dispatch roles, Catalyst v1 declares one — `conservation_law_validity`, AutoOnLoad. Future expansion (D27 §4.4): `qc_cat_check_deficiency` (Decidable), `qc_cat_compute_steady_states` (OnDemand), the `Catalyst → DiffEq` comorphism's QueryClasses.

## Step 4 — Read the handler

The handler at [`EigeniusCatalyst.jl`](../../../../julia/institutions/catalyst/EigeniusCatalyst/src/EigeniusCatalyst.jl) is small. The three pieces worth reading:

### Network parsing

```julia
function parse_network(source::AbstractString)
    expr = Meta.parse(source)
    if !(expr isa Expr && expr.head === :macrocall && expr.args[1] === Symbol("@reaction_network"))
        error("EigeniusCatalyst: network_source must be a single `@reaction_network` macro invocation")
    end
    return Core.eval(@__MODULE__, expr)
end
```

`Meta.parse` produces a Julia expression tree from the source string. The defensive check confirms the top-level form is exactly `@reaction_network begin … end` — anything else (a top-level `include`, a function definition, multiple statements) is rejected before Catalyst's macro machinery sees it. Then `Core.eval` expands the macro in the handler module's scope, which has `using Catalyst` so the macro's bindings resolve. The result is a fully-constructed `ReactionSystem`.

This is the only place in the institution where untrusted-ish input gets `eval`'d. The narrowness of the parse check — exactly one macro call, exactly the `@reaction_network` macro — is the security discipline; it doesn't make the surface immune to malicious sources, but it makes the threat surface explicit.

### The row-span check

```julia
function in_row_span(v::AbstractVector{<:Integer}, M::AbstractMatrix{<:Integer})::Bool
    if size(M, 1) == 0
        return all(==(0), v)
    end
    Mf = Float64.(M)
    vf = Float64.(v)
    Mext = vcat(Mf, vf')
    return rank(Mf) == rank(Mext)
end
```

A vector lies in the row span of a matrix iff appending it as a new row doesn't increase the rank. Implemented as a Float64 SVD-rank computation — sufficient for the small integer matrices Catalyst's conservation-law machinery produces (typical reaction networks have <20 species and <10 conservation laws). For pathological cases with very large integer entries or near-singular conditions, exact-arithmetic rank (over `Rational{BigInt}` with row reduction) would be more robust; v1 accepts the float-precision tradeoff for the simplicity, and notes the v2 path in the source comments.

### The handler

```julia
function validate_conservation_law(c::EigeniusMirror.ConservationLaw)
    rn = parse_network(c.network.network_source)
    M = Catalyst.conservationlaws(rn)

    if length(c.coefficients) != size(M, 2)
        return _verdict("Fails")
    end

    if in_row_span(Int.(c.coefficients), M)
        return _verdict("Holds")
    else
        return _verdict("Fails")
    end
end
```

Three branches: structural-mismatch `Fails`, in-row-span `Holds`, not-in-row-span `Fails`. No `Undecidable` — see the next subsection.

### Verdict policy: no `Undecidable` in v1

The intervals institution returns `Undecidable` when interval arithmetic can't decide containment (a value lands exactly on a bound that has multiple Float64 representations). The Symbolics institution returns `Undecidable` because `Symbolics.simplify` is heuristic — failure to match doesn't refute. **Catalyst's conservation-law check is neither.** The conservation matrix is exact (integer-valued); the row-span check is a structural-rank question; either the claimed vector is in the span or it isn't. `Holds` and `Fails` cover the cases.

`Undecidable` stays reserved for v2 cases where it might genuinely apply — e.g. a structurally-simplified network where some species are eliminated and the validator can't tell whether the claim refers to the original or simplified species set. Until that case lands, the institution doesn't return it.

## Step 5 — Build the environment image

(See [the intervals tutorial](intervals-institution-tutorial.md#step-5--build-the-environment-image) for the substrate-side mechanics.) The Catalyst-specific note: this is the slow one. ~10 min cold. Plan accordingly.

## Step 6 — Commit the env Resource

(See [the intervals tutorial](intervals-institution-tutorial.md#step-6--commit-the-environment-resource).)

## Step 7 — Install the institution

Three resources go on the chain in one commit: the `Institution` itself, the `RuntimeMethodSignature` for `validate_conservation_law`, and the AutoOnLoad `QueryClass` `conservation_law_validity`.

```bash
eigenius institution install \
    --definition julia/institutions/catalyst/declarations/catalyst-institution.eigon.json
```

After this commit, the kernel's `auto_on_load_by_class` index has an entry: `ConservationLaw` → `conservation_law_validity`. Every future `ConservationLaw` commit fires the gate.

## Step 8 — Commit the kinase reaction network

```bash
cat > /tmp/network.json <<'JSON'
[{
  "@id": "urn:eigenius:demo:catalyst:network:kinase",
  "core:is_a": ["urn:eigenius:catalyst:ReactionNetwork"],
  "core:short_name": "kinase_with_competitive_inhibition",
  "catalyst:network_source":
    "@reaction_network begin\n    k_on_S, E + S --> ES\n    k_off_S, ES --> E + S\n    k_cat, ES --> E + P\n    k_on_I, E + I --> EI\n    k_off_I, EI --> E + I\nend",
  "catalyst:species_declared": ["E", "S", "ES", "P", "I", "EI"],
  "catalyst:parameters_declared": ["k_on_S", "k_off_S", "k_cat", "k_on_I", "k_off_I"]
}]
JSON
eigenius load /tmp/network.json
```

### What just happened

A `ReactionNetwork` resource was committed. **No AutoOnLoad gate fired** — the institution declares no `query_class: ReactionNetwork`. The network is data; only claims *about* networks (currently `ConservationLaw`, future `SteadyState` / `DeficiencyZero` / etc.) trigger gates.

The species ordering deserves attention. Catalyst's `species(rn)` returns species in *first-appearance order across the reactions*:

- R1 `E + S --> ES`: introduces `E`, `S`, `ES`.
- R2 `ES --> E + S`: nothing new.
- R3 `ES --> E + P`: introduces `P`.
- R4 `E + I --> EI`: introduces `I`, `EI`.
- R5 `EI --> E + I`: nothing new.

So the canonical order is `[E, S, ES, P, I, EI]` — and that's exactly what `species_declared` carries. The conservation-law coefficient vectors in the next step index into this order positionally.

## Step 9 — Commit three ConservationLaw claims

```bash
cat > /tmp/laws.json <<'JSON'
[
  {
    "@id": "urn:eigenius:demo:catalyst:law:enzyme_conservation",
    "core:is_a": ["urn:eigenius:catalyst:ConservationLaw"],
    "core:short_name": "enzyme_conservation",
    "catalyst:network": "urn:eigenius:demo:catalyst:network:kinase",
    "catalyst:coefficients": [1, 0, 1, 0, 0, 1]
  },
  {
    "@id": "urn:eigenius:demo:catalyst:law:substrate_product_conservation",
    "core:is_a": ["urn:eigenius:catalyst:ConservationLaw"],
    "core:short_name": "substrate_product_conservation",
    "catalyst:network": "urn:eigenius:demo:catalyst:network:kinase",
    "catalyst:coefficients": [0, 1, 1, 1, 0, 0]
  },
  {
    "@id": "urn:eigenius:demo:catalyst:law:inhibitor_conservation",
    "core:is_a": ["urn:eigenius:catalyst:ConservationLaw"],
    "core:short_name": "inhibitor_conservation",
    "catalyst:network": "urn:eigenius:demo:catalyst:network:kinase",
    "catalyst:coefficients": [0, 0, 0, 0, 1, 1]
  }
]
JSON
eigenius load /tmp/laws.json
```

### What just happened

Three commits, three AutoOnLoad firings. For each:

1. The kernel's commit pipeline finds `conservation_law_validity` in `auto_on_load_by_class[ConservationLaw]`.
2. It dispatches `validate_conservation_law` against the orchestrator with the `ConservationLaw` mirror struct serialised to Eigon-CBOR. The struct embeds the full `ReactionNetwork` (the `network` property is typed `core:resource, class_types: [ReactionNetwork]`, so the kernel's IRI-dereference pass has already embedded the chain-committed network into the synthetic input).
3. The worker's mirror decodes the input, hands it to `validate_conservation_law(c::ConservationLaw)`.
4. The handler parses `c.network.network_source` to rebuild the `ReactionSystem`, calls `Catalyst.conservationlaws(rn)` to get the conservation matrix, and row-span-checks `c.coefficients` against it.
5. All three claims are valid conservation laws of the kinase network — three `Holds` verdicts come back.
6. The kernel commits each `ConservationLaw` along with its `Verdict + RuntimeInvocation` audit anchor.

Reading from the network itself: the conservation matrix `M = Catalyst.conservationlaws(rn)` for this network has rank 3 (six species, three independent laws). Each claim's coefficient vector is one of the three rows up to row reduction; the rank check `rank([M; v']) == rank(M)` holds for each.

If you committed a malformed claim — `[1, 1, 0, 0, 0, 0]`, claiming `E + S = const` — the catalysis reaction `ES → E + P` violates it (the reaction consumes one ES but produces one E and one P, so `E + S` doesn't stay constant: `S` doesn't change but `E` increases by 1). The rank check would refuse, the gate would return `Fails`, and the kernel would reject the load. The chain stays consistent: every persisted `ConservationLaw` is verified.

## Step 10 — Inspect the verdicts

```bash
eigenius query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) {
        "urn:eigenius:core:ctor_name": ?ctor
     } RETURN [] { verdict: ?v, ctor: ?ctor }'
```

Three rows: `ctor = "Holds"` for each of the enzyme / substrate-product / inhibitor verdicts. Each Verdict back-references its `RuntimeInvocation` (timing, image digest, numerical metadata).

## What now lives on the chain

After running the demo, the chain carries:

| Resource | IRI | Source |
|---|---|---|
| `ReactionNetwork` class + 3 properties | `urn:eigenius:catalyst:ReactionNetwork` etc. | step 1 |
| `ConservationLaw` class + 2 properties | `urn:eigenius:catalyst:ConservationLaw` etc. | step 1 |
| `RuntimePackageMirror` | `urn:eigenius:runtime:mirror:julia:<hex>` | step 3 |
| `RuntimeEnvironment` | `urn:eigenius:catalyst:env:v1` | step 6 |
| `Institution` | `urn:eigenius:institutions:catalyst` | step 7 |
| `RuntimeMethodSignature` | `urn:eigenius:catalyst:signatures:validate_conservation_law` | step 7 |
| `QueryClass` | `urn:eigenius:catalyst:query_classes:conservation_law_validity` | step 7 |
| Kinase `ReactionNetwork` | `urn:eigenius:demo:catalyst:network:kinase` | step 8 |
| 3 `ConservationLaw` claims | `urn:eigenius:demo:catalyst:law:*` | step 9 |
| 3 `Verdict`s | `urn:eigenius:invocation:<uuid>:verdict` | step 9 |
| 3 `RuntimeInvocation`s | `urn:eigenius:invocation:<uuid>` | step 9 |

The kinase mechanism is now a chain-committed citizen: queryable, comorphism-addressable, and structurally validated.

## Where this is heading

Catalyst v1's `ReactionNetwork` + `ConservationLaw` is the foundation. The downstream story stacks on top:

### The `Catalyst → DiffEq` comorphism (shipped, Phase 19h.1)

The `ReactionNetwork` chain shape exists precisely so that downstream institutions (DiffEq, Symbolics, JuMP) can consume it through typed cross-institution comorphisms. The first comorphism — `Catalyst → DiffEq` — is now wired:

- A `CatalystToOdeInput { network, initial_conditions, parameter_values, time_span_start, time_span_end }` composite class on the Catalyst side bundles everything needed to compile a network to a solvable problem.
- An `ExportFormat ef_cat_to_ode_input` (Catalyst-side) and an `ImportFormat if_diffeq_problem` (DiffEq-side) declare the typed boundary — both with `payload_type = diffeq:OdeProblem` (the same FormulaTerm-typed shape DiffEq's institution gates).
- A chain-committed `Comorphism(ef_cat_to_ode_input, m_id_ode_problem, if_diffeq_problem)` triple at [`julia/comorphisms/catalyst-to-diffeq.eigon.json`](../../../../julia/comorphisms/catalyst-to-diffeq.eigon.json) makes the typed contract first-class. The transformation `m_id_ode_problem` is the identity Lambda on `OdeProblem` — both ends speak the same shape, so the typed middle is a no-op (same pattern as the Symbolics → IntervalArithmetic identity comorphism).
- The operational backing is an OnDemand `qc_cat_to_ode` QueryClass invokable via FIBER. The Catalyst handler's `compile_to_ode(input::CatalystToOdeInput)` walks `Catalyst.netstoichmat(rn) * Catalyst.oderatelaw.(reactions(rn))` to get the symbolic per-species RHS, translates each via `num_to_formula` to a chain-typed `FormulaTerm`, and packs them into an `OdeProblem` mirror struct DiffEq can integrate.

In other words: with both institutions on the chain, the kinase mechanism flows end-to-end as **`ReactionNetwork → (qc_cat_to_ode FIBER) → OdeProblem (FormulaTerm RHS) → (DiffEq AutoOnLoad) → OdeSolution`** — a fully chain-typed pipeline from biochemical mechanism to integrated trajectory. See [the DiffEq tutorial](diffeq-institution-tutorial.md) for the receiving side.

### Catalyst v2 (independently shippable)

- `DeficiencyZero` / `DeficiencyOne` claim classes + AutoOnLoad gates over `Catalyst.deficiency(rn)`.
- `qc_cat_check_deficiency` Decidable QueryClass — exposes deficiency as a typed predicate user programs branch on.
- `SteadyState` claim + AutoOnLoad gate (needs HomotopyContinuation.jl or NonlinearSolve.jl in the env image).

### The kinase IC₅₀ pipeline (the real downstream value)

With Catalyst → DiffEq in place, the chain has the missing primitive for mechanistic kinase modelling. The full pipeline:

- A new `KinaseAssayMechanism` chain class linking `(ReactionNetwork, parameter_assignments, IC50_measurement)` so the screening notebook's flat `AssayResult(compound, target, protocol, ic50_nm)` rows carry a *mechanistic anchor* — "this IC₅₀ was measured under network N with parameters fitted to ⟨k_on_S, k_off_S, k_cat, k_on_I, k_off_I⟩ via assay protocol P."
- The Catalyst → DiffEq comorphism produces the `OdeProblem`; DiffEq integrates; a parameter sweep over inhibitor concentration produces a numerical IC₅₀ from the simulated time courses.
- A comorphism into JuMP takes `(network, observed_IC50)` and produces a parameter-fit problem; the fitted parameters get committed back as `KinaseAssayMechanism.parameter_assignments`.
- A `DiffEq → IntervalArithmetic` comorphism (the third in the chain) walks the FormulaTerm RHS under interval semantics + the integrated trajectory's time grid to produce rigorous bounds — the assay's 95% CI becomes a *mathematically-bounded* confidence interval rather than a statistical one.

That's where the kinase-institutions notebook ([`notebooks/examples/kinase-institutions.json`](../../../../notebooks/examples/kinase-institutions.json)) upgrades from "flat dataset of measurements" to "mechanistic claims with end-to-end interval-bounded provenance." The Catalyst institution + the comorphism wired in this session is the first two foundation stones.

## Common failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `network_source must be a single \`@reaction_network\` macro invocation` | Source string contains anything besides `@reaction_network begin … end` — extra statements, wrong macro, syntax error. | Re-format the source so the entire string is a single `@reaction_network ... end`. |
| `ConservationLaw … Fails` on a claim you believe should hold | Coefficient ordering doesn't match the declared `species_declared`, OR the claim is actually wrong (the catalysis reaction violates it). | Print `species(rn)` in a Julia REPL, line the coefficients up against that order, and double-check the stoichiometry by hand. |
| `length(c.coefficients) != size(M, 2)` from the handler | Coefficient vector length doesn't match the network's species count. | The vector must have one entry per species, including species that don't participate in the conservation law (those entries are `0`). |
| The env build hangs on `Pkg.precompile` for >10 minutes | Catalyst.jl's dep tree is genuinely large (MTK + SymbolicUtils + SciML); first-time precompile is slow but not stuck. | Wait. If it's still going at 30 minutes, check the orchestrator container's stdout for actual errors. |

For substrate-level failure modes (`seed manifest drift`, depot bind-mount issues, manifest-hash mismatch), see [the intervals tutorial](intervals-institution-tutorial.md#common-failure-modes).
