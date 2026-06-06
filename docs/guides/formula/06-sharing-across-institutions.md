# 6. Sharing across institutions

The structural payoff of a chain-resident, typed expression-tree language
is that **multiple institutions can speak it at once**. This chapter walks
through what that means in practice for the v1 Julia institutions, why
comorphisms between them are nearly trivial, and where the abstraction
actually pays off.

## 6.1. Five institutions, one payload language

The v1 platform ships five Julia-backed institutions
([platform §11](../platform/11-runtime-substrate.md), tutorials under
[`platform/julia-institutions/`](../platform/julia-institutions/)). Each
declares chain shapes that carry FormulaTerm-typed properties:

| Institution | Class with a FormulaTerm field | Field name |
|---|---|---|
| Symbolics | `SymbolicExpression` | `term` |
| IntervalArithmetic | `IntervalFunction` | `term` |
| Catalyst | (network compiles to) `OdeProblem.rhs[i]` | `term` (per `RhsComponent`) |
| DiffEq | `OdeProblem.rhs[i]` | `term` (per `RhsComponent`) |
| JuMP-HiGHS | `OptimisationProblem.objective`, `Constraint.lhs` | direct FormulaTerm |

The key thing is that **none** of those classes own the FormulaTerm
shape. They all reference it as `core:inductive` typed by
`urn:eigenius:formulas:FormulaTerm` — declared once, in the kernel
bootstrap, peer to nothing. Authoring a `SymbolicExpression` and
authoring a `Constraint.lhs` use the same six-constructor encoding from
[chapter 3](03-eigon-json-embedding.md), the same `formula(...)` ESL
sublanguage from [chapter 5](05-esl-sublanguage.md), the same operator
catalog from [chapter 4](04-operator-catalog.md).

## 6.2. Identity comorphisms

When two institutions share a payload language, the comorphism between
them collapses to the **identity function on the payload**. The chain
shape of a comorphism has three pieces (D14 §4):

- An `ExportFormat` on the source side that extracts a typed payload
  from a source-class instance.
- A `transformation` Component that operates on the payload.
- An `ImportFormat` on the target side that constructs a target-class
  instance from a typed payload.

For two FormulaTerm-speaking institutions, the `transformation` is
literally `Lam(t: FormulaTerm. Var(t))` — the identity. The
ExportFormat unwraps a `SymbolicExpression`'s `term` field; the
ImportFormat wraps the same FormulaTerm as an `IntervalFunction.term`
(or similar). No translation work, because both endpoints already
agree on the bytes.

The comorphism declaration (from
[`julia/comorphisms/symbolics-to-intervals.eigon.json`](../../../julia/comorphisms/symbolics-to-intervals.eigon.json)):

```json
{
  "@id": "urn:eigenius:comorphisms:symbolics_to_intervals",
  "core:is_a": ["institution:Comorphism"],
  "institution:export_format": "urn:eigenius:symbolics:formats:ef_symb_expr",
  "institution:transformation": "urn:eigenius:comorphisms:symbolics_to_intervals:m_id_formula_term",
  "institution:import_format": "urn:eigenius:intervals:formats:if_intv_function",
  "institution:exact": true
}
```

`exact: true` is the tell — bit-for-bit payload preservation, no
semantic loss. D32 §6.2 makes this the load-bearing claim for FormulaTerm:
when two institutions share the typed payload language, the comorphism
between them collapses to identity.

## 6.3. The structural payoff

What this buys at scale, from author and operator perspectives:

- **Comorphism declarations are tiny.** A new bridge between two
  FormulaTerm institutions is a six-line JSON resource — the
  ExportFormat, the ImportFormat, and a comorphism declaration linking
  them with `m_id_formula_term`. No transformation code to write,
  test, or audit. (For non-identity transformations, the `transformation`
  Component is a EigenTT term — also chain-resident — and the kernel
  type-checks the signature alignment.)
- **Cross-institution dispatch is free at the wire.** When a
  `SymbolicExpression` gets reified as an `IntervalFunction` via the
  comorphism, no payload conversion happens. The chain bytes that
  encoded the original term encode the reified term; only the
  containing class changes (`SymbolicExpression` → `IntervalFunction`).
- **Handlers don't repeat decoder logic.** The mirror generator
  emits one `decode_FormulaTerm` per institution's mirror package —
  but they're identical (auto-generated from the same chain
  inductive). The handlers consume `FormulaTerm` mirror structs
  directly via Julia multiple dispatch, without per-institution
  decoder differences.
- **Validator catches arity errors uniformly.** A 3-argument App-spine
  against `add`'s 2-argument signature gets rejected the same way
  whether it landed via Symbolics, JuMP, or hand-authored Eigon-JSON.

## 6.4. The kinase notebook's three comorphisms

Three comorphisms register in the kinase-institutions setup:

| Comorphism | Source class | Target class | Transformation | Exact |
|---|---|---|---|---|
| `symbolics_to_intervals` | `SymbolicExpression` | `IntervalFunction` | identity on `FormulaTerm` | ✓ |
| `catalyst_to_diffeq` | `CatalystToOdeInput` | `OdeProblem` | identity on `OdeProblem` | ✗ (mass-action ODE compilation faithful only to the deterministic limit) |
| `symbolics_to_jump` | `SymbolicsToJuMPInput` | `OptimisationProblem` | identity on `OptimisationProblem` | ✗ (the framing imposes structure not in the source SymbolicExpression) |

Two of three are identity-on-FormulaTerm directly. The third
(`catalyst_to_diffeq`) involves more structural work — Catalyst's reaction
network compiles into multiple FormulaTerm RHS components — but each
RHS is itself a FormulaTerm, so the *FormulaTerm-shaped portions* of
the work are still identity transforms; only the surrounding structure
differs.

`exact` flags the satisfaction-condition fidelity (D14 §4.5): an exact
comorphism preserves truth of sentences across the translation; an
inexact one is sound but loses some semantic detail. In the
Catalyst → DiffEq case, the inexactness is the deterministic-limit
approximation of mass-action kinetics, not anything specific to the
formula language.

## 6.5. Where this leads

Two near-term and one medium-term direction the architecture supports:

- **Sibling solvers in JuMP.** The `OptimisationProblem` shape with a
  FormulaTerm objective + FormulaTerm constraints is solver-agnostic.
  Sibling institutions for `JuMP+Ipopt` (general nonlinear),
  `JuMP+GLPK` (LP/MILP), `JuMP+Gurobi` (commercial) plug in by reusing
  the same ontology and walker, swapping only the `Project.toml` deps
  and the `Model(SolverFoo.Optimizer)` line. See the
  [JuMP-HiGHS tutorial](../platform/julia-institutions/jump-highs-institution-tutorial.md#per-solver-institution-siblings).
- **More numerical institutions.** Anything with a typed expression
  payload — symbolic computer-algebra systems (SageMath, Maxima),
  rigorous numerics libraries (Arb, MPFR), neural-network
  symbolic-execution tools — slots in by following the substrate
  install flow from [chapter 11](../platform/11-runtime-substrate.md)
  and consuming `FormulaTerm` directly.
- **Logical clauses crossing institutions.** Under the Curry–Howard
  reading from [chapter 2 §2.2](02-mini-tt-fragment.md#22-why-pi-and-lam-are-chain-resident),
  a typed predicate (a `Pi`-shaped value with a propositional return
  type) can travel through a comorphism the same way an arithmetic
  expression does. The medium-term path opens cross-institution
  reasoning beyond pure numerical work — a Symbolics-authored
  side-condition can be transferred to IntervalArithmetic for a
  rigorous bound, or to JuMP for an optimisation reformulation. The
  v1 institutions don't yet exploit this fully, but the chain shapes
  are in place.

## 6.6. Cross-references

- [Symbolics tutorial](../platform/julia-institutions/symbolics-institution-tutorial.md)
  — most thorough worked example of FormulaTerm end-to-end (validator,
  AutoOnLoad gates, OnDemand FIBER, Decidable predicates).
- [Catalyst tutorial](../platform/julia-institutions/catalyst-institution-tutorial.md)
  + [DiffEq tutorial](../platform/julia-institutions/diffeq-institution-tutorial.md)
  — Catalyst → DiffEq comorphism in detail.
- [JuMP-HiGHS tutorial](../platform/julia-institutions/jump-highs-institution-tutorial.md)
  — smart-pow walker, LP/QP encoding via FormulaTerm.
- [Symbolics → IntervalArithmetic comorphism](../../../julia/comorphisms/symbolics-to-intervals.eigon.json)
  — chain-resident form of the identity comorphism.

---

Next: **[7. Common failure modes →](07-failure-modes.md)**
