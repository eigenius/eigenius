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

## 2.4a. A second shared payload landed: `core:EigenTTType` propositions (D47)

`FormulaTerm` covers the numerical institutions. The reasoning + statistics stack (D52 + D39) sits on a *different* shared payload: chain-mirrored EigenTT type expressions, declared at `core:EigenTTType` per [D47](../../../design/d47-chain-mirrored-eigentt-type-fragment.md). Same general mechanism — a chain-resident inductive type that multiple institutions consume directly — but a different semantic domain (typed propositions and dependent types, not numerical expressions) and a different bridge mechanism (the [D49 chain-witness index](../platform/reasoning-institution/README.md#the-d49-witness-index-how-the-kernel-admits-grounding-witnesses), not a comorphism extract-transform-reify pipeline).

The shape:

```esl
data core:EigenTTType : Type 0 {
    ConstRef(core:string),                  // qualified name of a class or primitive
    App(core:EigenTTType, core:EigenTTType), // application
    Pi(core:EigenTTType, core:EigenTTType),  // dependent function type
    LitString(core:string),                  // string literal as a Prop argument
    LitInt(core:integer),
    Sort(core:integer),                      // universe level
    // ...
}
```

A chain-resident value of `core:EigenTTType` IS a typed proposition (when it lives in `Prop` per [D46](../../../design/d46-prop-universe-and-proof-irrelevance.md)) or a type expression. The author surface is [`type_expr(...)`](../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions) — the syntactic counterpart of `formula(...)` for the proposition language:

```esl
reflection:canonical_proposition = type_expr(
    screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
);
```

This lowers to a `Value::Json` carrying the tagged-dict tree:

```json
{
  "ctor": "App",
  "args": [
    {"ctor": "ConstRef", "args": ["urn:eigenius:demo:screen:HasLowIC50"]},
    {"ctor": "LitString", "args": ["urn:eigenius:demo:screen:EIG_0291"]}
  ]
}
```

### Which institutions consume it

| Institution | What it reads `core:EigenTTType` for |
|---|---|
| **D52 statistics** ([tutorial](../platform/statistics-institution/README.md)) | The `StatisticalAnalysisPlan`'s `null_hypothesis` / `alternative_hypothesis` / `canonical_proposition` slots carry chain-mirrored propositions. The §7.4 epistemic-scope check walks the proposition's head predicate to look up its `is_a` scope markers. |
| **D39 reasoning** ([tutorial](../platform/reasoning-institution/README.md)) | The `ReasoningSentence`'s `proposition` slot. The certificate's `JustifiedBy(j, P)` indices read it. The grounding constructors (`declared`/`observed`/`derived`/`verified`) hash it to compute the witness-index key. |
| **D49 chain-witness index** ([§6.4a](../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state)) | Reads `canonical_proposition` from every chain-resident `DeclaredResource` / `ObservedResource` / `DerivedResource` / `VerifiedResource` and computes a SHA-256 hash to key the witness-admission table. |
| **Lean institution** ([tutorial](../platform/lean-institution/README.md)) | The `lean_to_reasoning` comorphism reifies a Lean proof's proposition as a `reasoning:VerifiedPropositionView` with a `canonical_proposition` slot — same chain shape, written by the comorphism instead of by the original author. |

### The bridge mechanism is different

For `FormulaTerm`, institutions coordinate through declared **comorphisms** — chain-resident bridges that translate one institution's view of a `FormulaTerm` value into another's, with the kernel statically type-checking the alignment ([chapter 3](03-comorphisms.md)). The transformation is *active*: one institution's runtime is invoked, output is reified back into the chain.

For `core:EigenTTType`, institutions coordinate through the **witness index** ([§6.4a](../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state)). Each institution that emits a chain-resident `DerivedResource` (or `ObservedResource` / `DeclaredResource` / `VerifiedResource`) with a `canonical_proposition` slot automatically populates the per-layer witness index at construction. Each institution that consumes a proposition (notably D39, but in principle any institution that wants to admit a `ChainWitness` predicate at type-check time) reads from the same index. The composition is *passive* — D39 doesn't call D52; it just reads what D52 emitted.

This is the load-bearing structural difference between the two composition shapes. Comorphisms are explicit translation handlers; witness-index composition is implicit through a shared chain artifact shape. Both work; which one applies depends on whether the downstream institution needs the input *value translated* (comorphism, `FormulaTerm` shape) or just *cited as evidence* (witness index, `core:EigenTTType` shape).

### Identity-comorphism collapse, witness-index edition

The `FormulaTerm` story includes the identity-comorphism collapse: when both institutions speak the same payload, the comorphism's transformation step is a no-op and the bridge collapses to a pure dispatch route ([§2.4](#2-4-identity-comorphism-collapse-the-structural-payoff)). The witness-index analog: when two institutions share the `core:EigenTTType` proposition shape and the `canonical_proposition` slot, *no bridge code at all is required*. The producer institution emits the resource with `canonical_proposition` set; the consumer institution looks up the proposition by hash in the witness index. There is no comorphism to declare, no transformation to write — the shape itself is the protocol.

The drug-screening fixture at [`crates/eigenius-reasoning/tests/fixtures/drug_screening.esl`](../../../crates/eigenius-reasoning/tests/fixtures/drug_screening.esl) exercises this: the `claim_eig0291_lowic50` StatisticalAnalysisPlan's `canonical_proposition = HasLowIC50("urn:...:EIG_0291")` becomes available to the downstream `concl_eig0291_strong` ReasoningSentence's `derived(claim_iri, HasLowIC50(...), ...)` certificate constructor *without any bridge code on either side* — D52 emits the proposition into a chain slot D39 already reads from. See [chapter 7](07-stats-and-reasoning-walkthrough.md) for the full walkthrough.

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

`FormulaTerm` and `core:EigenTTType` are v1's two shared payloads —
the first coordinates numerical institutions via comorphisms ([§2.2-§2.4](#2-2-formulaterm-as-a-coordination-mechanism)),
the second coordinates statistics + reasoning via the witness index
([§2.4a](#2-4a-a-second-shared-payload-landed-coreeigentttype-propositions-d47)).
The shared-payload principle generalises further: any chain-mirrored
inductive type at a peer namespace can play the same role for a third
or fourth institution family. One near-term candidate the platform's
structure makes natural:

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

The structural payoff documented in [§2.4](#2-4-identity-comorphism-collapse-the-structural-payoff)
(identity collapse for the comorphism shape) and the equivalent
zero-bridge-code property documented in [§2.4a](#2-4a-a-second-shared-payload-landed-coreeigentttype-propositions-d47)
(shared chain slot for the witness-index shape) both transfer to any
future shared payload — once the inductive is on the chain at a peer
namespace, every institution that consumes it composes either through
identity comorphisms (if the runtime needs to read the value) or through
the appropriate witness/admission mechanism (if the value is being cited
as evidence rather than processed).

---

Next: **[3. Comorphisms — bridges between domains →](03-comorphisms.md)**
