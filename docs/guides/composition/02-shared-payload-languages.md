# 2. Shared payload languages

The first and cheapest layer of composition is agreeing on a shared *data
shape*. When two institutions both consume the same typed payload, bridging
them via a comorphism becomes nearly trivial. When they don't, every
comorphism has to do real translation work, and the cost compounds with each
new institution that joins the conversation.

This chapter establishes three claims, all grounded in
[`formulas:FormulaTerm`](../formula/README.md) as the v1 example: a shared
payload is a coordination mechanism, not a domain vocabulary; with one in
place comorphisms collapse to identity (or near-identity) transformations;
and the principle generalises beyond formulas.

## 2.1. Why payload-shape agreement matters

Consider two institutions that need to exchange numerical expressions —
Symbolics (symbolic algebra) and IntervalArithmetic (rigorous bounds). Without
a shared payload language, each has its own AST: Symbolics carries
`SymbolicUtils.BasicSymbolic` trees, IntervalArithmetic carries closures over
intervals. A bridge between them must:

1. Read the source AST out of the Symbolics-side resource.
2. Translate node-by-node into the target AST shape.
3. Write the target AST into an IntervalArithmetic-side resource.

That's three pieces of code per direction, and the code lives outside the
chain — not inspectable, not type-checked, not reproducible without
re-execution. Worse, when a third institution joins (say, JuMP), the
combinatorics get expensive: N institutions need O(N²) bridges in principle,
each with its own translation code.

A **shared payload language** flips the cost. Both institutions agree on a
single typed value shape — for v1 numerical work, that's
[`formulas:FormulaTerm`](../formula/README.md). Each institution exposes a
boundary that *speaks the shared shape* — Symbolics' `ExportFormat` extracts
a FormulaTerm out of its `SymbolicExpression`, IntervalArithmetic's
`ImportFormat` wraps a FormulaTerm into its `IntervalFunction`. The bridge
between them is a one-line declaration: a `Comorphism` whose `transformation`
is the *identity* function on FormulaTerm (chapter 3 covers this in detail).

The structural saving: O(N) boundaries instead of O(N²) bridges, each
boundary reusable across every comorphism that targets the institution.

## 2.2. `FormulaTerm` as a coordination mechanism

The first thing to internalise about `formulas:FormulaTerm` is that it
**doesn't belong to any one institution**. It lives at
`urn:eigenius:formulas:` — peer to `urn:eigenius:core:`, `urn:eigenius:program:`,
`urn:eigenius:reflection:`, `urn:eigenius:institution:`, and
`urn:eigenius:notebook:` — the kernel bootstrap layers loaded on every chain.
Every chain has FormulaTerm available from the moment it starts.

This namespacing is deliberate. If FormulaTerm lived under
`urn:eigenius:symbolics:` (where the Symbolics-the-institution declares its
domain shapes), Catalyst couldn't speak it without reaching into Symbolics's
namespace; and a Symbolics → JuMP comorphism would have to *convert* the
expression tree from Symbolics's vocabulary into JuMP's. Since both
institutions actually speak the same expression-tree shape, the conversion
is unnecessary work — but only if neither owns the shape.

Living at `urn:eigenius:formulas:` makes FormulaTerm a *shared asset*. Every
institution that consumes it does so as a peer; nobody owns it. The five v1
Julia institutions all consume the same FormulaTerm shape:

| Institution | Class with a FormulaTerm field | Field name |
|---|---|---|
| Symbolics | `SymbolicExpression` | `term` |
| IntervalArithmetic | `IntervalFunction` | `term` |
| Catalyst | `OdeProblem.rhs[i]` (per `RhsComponent`) | `term` |
| DiffEq | `OdeProblem.rhs[i]` (per `RhsComponent`) | `term` |
| JuMP-HiGHS | `OptimisationProblem.objective`, `Constraint.lhs` | direct FormulaTerm |

None of them *owns* FormulaTerm. All of them *use* it.

For the FormulaTerm shape itself — the six constructors, the operator
catalog, the validator's inductive-value rule — see the
[formula language guide](../formula/README.md). This chapter is about its
role as a coordination mechanism.

## 2.3. Five institutions, one payload — the kinase setup at a glance

The kinase notebook's setup script registers five institutions and three
comorphisms in one go (cells 1–3 of
[`kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh)).
The structural fact worth pausing on: every cross-institution edge is
FormulaTerm-shaped.

```
                 ┌─────────────────────────┐
                 │     formulas:FormulaTerm │
                 │  (kernel bootstrap layer)│
                 └────────────┬─────────────┘
                              │ shared payload
        ┌─────────────┬───────┼───────┬─────────────┐
        ▼             ▼       ▼       ▼             ▼
  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐
  │ Symbolics│  │Intervals │  │ Catalyst │  │  DiffEq  │  │   JuMP   │
  └────┬─────┘  └─────▲────┘  └─────┬────┘  └────▲─────┘  └─────▲────┘
       │              │             │            │              │
       │ FormulaTerm  │ FormulaTerm │ OdeProblem │              │
       └──────────────┘             └────────────┘              │
       (identity comorphism         (structural — but the       │
        symbolics_to_intervals)      OdeProblem's RhsComponents  │
                                     each carry FormulaTerm)     │
       │                                                         │
       └──────────────── FormulaTerm ────────────────────────────┘
       (identity comorphism symbolics_to_jump)
```

Two of the three comorphisms have **identity transformations** in the middle —
the FormulaTerm comes out of the source institution and goes straight into
the target institution unchanged. The third (`catalyst_to_diffeq`) has a
*structural* transformation that compiles a reaction network into an ODE
right-hand side — but the result *itself* is FormulaTerm-shaped, so the
DiffEq side reads it the same way. Chapter 3 unpacks the structural case.

## 2.4. Identity-comorphism collapse: the structural payoff

When both endpoints of a comorphism share the payload language, the
comorphism's `transformation` Component collapses to the identity function:

```
Lambda(t: FormulaTerm. Var(t))
```

The chain bytes flow through unchanged. From
[`julia/comorphisms/symbolics-to-intervals.eigon.json`](../../../julia/comorphisms/symbolics-to-intervals.eigon.json):

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

`exact: true` is the tell — bit-for-bit payload preservation, no semantic
loss. This isn't an optimisation hack; it's the load-bearing claim of
[D32 §6.2](../../design/d32-chain-mirrored-mini-tt-inductives.md): when two
institutions share the typed payload language, the comorphism between them
collapses to identity.

The structural payoff at scale:

- **Comorphism declarations are tiny.** A new bridge between two
  FormulaTerm-speaking institutions is a six-line JSON resource — the
  ExportFormat, the ImportFormat, and a Comorphism declaration linking them
  via a shared `m_id_formula_term`. No transformation code to write, test, or
  audit.
- **Cross-institution dispatch is free at the wire.** When a
  `SymbolicExpression` gets reified as an `IntervalFunction` via the
  comorphism, no payload conversion happens. The chain bytes that encoded
  the original term encode the reified term; only the containing class
  changes.
- **Handlers don't repeat decoder logic.** The mirror generator emits one
  `decode_FormulaTerm` per institution's mirror package — but they're
  identical (auto-generated from the same chain inductive). The handlers
  consume `FormulaTerm` mirror structs directly via Julia multiple dispatch,
  without per-institution decoder differences.
- **The validator catches arity errors uniformly.** A 3-argument App-spine
  against `add`'s 2-argument signature gets rejected the same way whether it
  landed via Symbolics, JuMP, or hand-authored Eigon-JSON.

## 2.5. When *not* to share a payload

Shared payloads are not free. They require all participating institutions to
agree on the encoding up front. When domains genuinely diverge, forcing a
shared payload is harmful — you end up with an awkwardly general type that
nobody finds natural, and every institution writes ad-hoc adapters from the
shared shape into its preferred form.

Three signals that a shared payload is the wrong choice:

1. **The natural types differ in cardinality.** If institution A operates
   on infinite streams and institution B operates on finite tuples, no
   single payload language captures both without lossy conversion in one
   direction.
2. **The semantic invariants differ.** FormulaTerm is well-typed because
   every operator has a chain-resident signature. If institution A wants
   total functions and institution B wants partial functions with explicit
   error sentinels, a shared "function" payload would force one of them to
   work against the grain of its semantics.
3. **The transformation between domains is structurally interesting on its
   own.** Catalyst → DiffEq is the worked example: compiling a reaction
   network into an ODE right-hand side is a real transformation that
   deserves its own representation. Forcing both sides to share an
   intermediate "graph + system" payload would obscure where the work
   happens.

When you reach for a shared payload, you're buying *coordination*. When you
reach for a structural comorphism, you're buying *honesty about the
translation*. The trade-off lives where it should: in the comorphism
declaration ([chapter 3](03-comorphisms.md)).

## 2.6. What other shared payloads might look like

`FormulaTerm` is v1's only shared payload. The principle generalises: any
chain-mirrored inductive type can play the same role. Two near-term
candidates the platform's structure makes natural:

- **A shared planning-tree language.** An inductive type whose constructors
  describe a planned action (`Procure`, `Move`, `Transform`, `Sell`,
  `Hedge`, `Reschedule`, `Reroute`) plus binders for control flow
  (`Sequence`, `Parallel`, `IfElse`, `Forall`). A scenario, a process model,
  a simulation trajectory, and a solver plan would all be values in this
  language. Cross-institution comorphisms for "the simulation institution's
  view of a Q3 plan" → "the routing institution's view of the same plan"
  collapse to identity the way the Symbolics → IntervalArithmetic comorphism
  does. The
  [enterprise supply-chain scenario note](../../notes/enterprise-supply-chain-scenario.md)
  explores this shape in a non-science domain.

- **A shared logical-clause language.** An inductive type for typed
  propositions and conditions (`Forall`, `Exists`, `Implies`, `BoundedBy`,
  `EquivalentTo`) over enterprise quantities. Because the kernel's Mini-TT
  carries `Pi` and `Lam` natively (the binders that make FormulaTerm do
  double duty as a logical language under
  [Curry-Howard](../formula/02-mini-tt-fragment.md#22-why-pi-and-lam-are-chain-resident)),
  a covenant condition like "for all rolling 12-month windows,
  debt-service-coverage ratio ≥ 1.25" is a typed value the receiving
  institution can introspect, simplify, and discharge.

Neither of these has been built yet. The point is that the *machinery* for
shared-payload composition is generic — once an inductive type is on the
chain at a peer namespace, every institution that consumes it can compose
through identity comorphisms, and the structural payoff in §2.4 transfers.

---

Next: **[3. Comorphisms — bridges between domains →](03-comorphisms.md)**
