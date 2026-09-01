# 2. Shared payload languages

The first and cheapest layer of composition is agreeing on a shared *data
shape*. When two institutions both consume the same typed payload, the
comorphism between them declares an identity middle and the reading code on
each side is written once. When they don't, every consumer needs a reader for
every producer's shape.

This chapter establishes three claims, all grounded in
[`formulas:FormulaTerm`](../formula/README.md) as the v1 example: a shared
payload is a coordination mechanism, not a domain vocabulary; with one in
place a comorphism's declared middle becomes the identity, and the
translation work relocates to the boundary procedure feeding it; and the
principle generalises beyond formulas.

Everything below is a structural claim about the shipped declarations and
handler source, checked against `julia/institutions/*/declarations/` and
`julia/comorphisms/`. Nothing in the repository *measures* the payoff: no
code, test, ontology or design document counts adapters or enumerates
institution pairs. Read the numbers here as an inventory, not as a
benchmark.

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
re-execution.

A **shared payload language** changes where that code has to live. Both
institutions agree on a single typed value shape — for v1 numerical work,
that's [`formulas:FormulaTerm`](../formula/README.md). Each institution
exposes a boundary that *speaks the shared shape* — Symbolics' `ExportFormat`
extracts a FormulaTerm out of its `SymbolicExpression`, IntervalArithmetic's
`ImportFormat` wraps a FormulaTerm into its `IntervalFunction`. The bridge
between them is a short declaration: a `Comorphism` whose `transformation`
is the *identity* function on FormulaTerm (chapter 3 covers this in detail).

Two different savings get conflated here, and only one of them is visible in
the shipped code. State them separately.

**The saving the code shows: interpreters, not translators.** An institution
that consumes FormulaTerm needs exactly one reader for it, no matter how many
peers it eventually exchanges terms with. Four of the five v1 Julia
institutions have such a reader, and each is a six-method walk over the
mirror structs plus a dictionary from operator IRI to a Julia function
([§2.4](#2-4-what-the-shared-payload-buys-interpreters-not-translators)).
Under a pairwise scheme, the same four consumers would each need a
translator per ordered pair they actually exchanged terms over. That
substitution — one interpreter per *consuming institution* in place of one
translator per *ordered pair* — is the structural saving, and it is legible
in the source.

**The saving the code does not show: fewer comorphism-side translations.**
The identity middle does not make the translation disappear; it relocates it
into the export procedure on the source side. That procedure is reusable in
principle — any comorphism targeting the same payload can take it — but on
the present chain each of the three export procedures serves exactly one
comorphism, so the count of translations written has not yet fallen. Three
comorphisms exist among the twenty ordered pairs five institutions admit,
and no test enumerates pairs.

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
institution that consumes it does so as a peer; nobody owns it. **Four** of
the five v1 Julia institutions declare a FormulaTerm-typed chain property —
five such properties between them:

| Institution | Property | Declared on | Declaring file |
|---|---|---|---|
| Symbolics | `symbolics:term` | `SymbolicExpression` | `symbolics-ontology.eigon.json` |
| IntervalArithmetic | `intervals:term` | `IntervalFunction` | `intervals-ontology.eigon.json` |
| DiffEq | `diffeq:term` | `RhsComponent` (one per element of `OdeProblem.rhs`) | `diffeq-ontology.eigon.json` |
| JuMP-HiGHS | `jump:objective` | `OptimisationProblem` | `jump-ontology.eigon.json` |
| JuMP-HiGHS | `jump:lhs` | `Constraint` | `jump-ontology.eigon.json` |
| Catalyst | *(none)* | — | — |

Each of the five carries `core:data_type: core:inductive` and
`core:class_types: ["urn:eigenius:formulas:FormulaTerm"]` — the exact pair
[Rule 17](../formula/04-operator-catalog.md#4-3-the-app-spine-arity-check)
fires on.

**Catalyst is the exception, and it is worth stating plainly.** Catalyst
declares no FormulaTerm-typed property at all; the only occurrence of
`FormulaTerm` anywhere in its ontology is one sentence of prose. Its
`ReactionNetwork` carries the network as a `@reaction_network` source
*string* plus the declared species and parameter orderings, and it reaches
the shared shape only inside its own Julia handler, through a bespoke
`Symbolics.Num`-to-`FormulaTerm` walker with an explicit failure path for
operators it cannot encode. `OdeProblem.rhs` — sometimes attributed to
Catalyst — is declared by **DiffEq**, not Catalyst; Catalyst's
`compile_to_ode` procedure *produces* one, which is a different relation.
So the shared-payload argument holds for four of the five institutions, and
for the fifth the per-institution glue the argument promises to remove is
still there.

None of the four *owns* FormulaTerm. All of them *use* it.

For the FormulaTerm shape itself — the six constructors, the operator
catalog, the validator's inductive-value rule — see the
[formula language guide](../formula/README.md). This chapter is about its
role as a coordination mechanism.

## 2.3. Five institutions, one payload — the kinase setup at a glance

The kinase notebook's setup script registers five institutions and three
comorphisms in one go (cells 1–3 of
[`kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh)).
Three edges among the twenty ordered pairs five institutions admit. The
structural fact worth pausing on is *not* that every cross-institution edge
is FormulaTerm-shaped — only one of the three is. It is that every edge's
declared middle is an identity, and that FormulaTerm is what the composite
payloads on the other two edges are built out of.

```
                 ┌──────────────────────────┐
                 │     formulas:FormulaTerm │
                 │  (kernel bootstrap layer)│
                 └────────────┬─────────────┘
                              │ shared payload
        ┌─────────────┬───────┴───────┬─────────────┐
        ▼             ▼               ▼             ▼
  ┌──────────┐  ┌──────────┐    ┌──────────┐  ┌──────────┐   ┌──────────┐
  │ Symbolics│  │Intervals │    │  DiffEq  │  │   JuMP   │   │ Catalyst │
  └────┬─────┘  └─────▲────┘    └────▲─────┘  └─────▲────┘   └─────┬────┘
       │              │              │              │              │
       │ FormulaTerm  │              │  OdeProblem  │              │
       └──────────────┘              └──────────────┼──────────────┘
       (identity on FormulaTerm,     (identity on OdeProblem,
        symbolics_to_intervals,       catalyst_to_diffeq, exact:false —
        exact:true)                   compilation lives in Catalyst's
                                      ef_cat_to_ode_input export)
       │                                            │
       └──────── OptimisationProblem ───────────────┘
       (identity on OptimisationProblem, symbolics_to_jump, exact:false —
        the framing lives in Symbolics' ef_symb_to_jump_input export)
```

Catalyst is drawn off to the side deliberately: it is a comorphism *source*
without being a FormulaTerm declarer. It sits on the diagram because its
export procedure emits FormulaTerm-typed `RhsComponent`s into a
DiffEq-declared `OdeProblem`, not because it declares a slot of its own.

All three comorphisms declare an **identity transformation** in the middle,
but at three *different* types. Only `symbolics_to_intervals` is the
identity on `FormulaTerm` itself, and it is the only one of the three marked
`exact: true`. `catalyst_to_diffeq` is the identity on `diffeq:OdeProblem`
and `symbolics_to_jump` the identity on `jump:OptimisationProblem` — both
institution-owned composite classes, both `exact: false`.

The two composite cases are not structural transformations that happen to be
declared as identities. The structural work has been *relocated* to the
export procedure on the source side, and the declarations say so in as many
words: `catalyst_to_diffeq`'s description states that "the actual
compilation work happens inside the ExportFormat's `procedure`", and
`symbolics_to_jump`'s that the source-side export "already produces a
fully-formed OptimisationProblem". Chapter 3 unpacks where each piece of
work ended up.

## 2.4. What the shared payload buys: interpreters, not translators

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
institutions share the typed payload language, the comorphism's declared
middle is the identity.

Note the scope of that claim. It is about the *declared middle*. It does not
say the translation went away, and on the present chain the translation has
not gone away — see [§2.4b](#2-4b-what-the-shared-payload-did-not-buy).

### The saving that is legible in the source

Look one level below the comorphisms. Four of the five institutions consume
`FormulaTerm` by walking it, and each walker has the same shape: six Julia
methods, one per constructor, dispatched on the mirror struct type, plus a
dictionary from operator IRI to a Julia function.

| Institution | Walker | Target type | Operators |
|---|---|---|---|
| Symbolics | `formula_to_num` | `Symbolics.Num` | 13 |
| IntervalArithmetic | `formula_to_interval` | interval | 13 |
| DiffEq | `formula_to_value` | `Float64` | 13 |
| JuMP-HiGHS | `formula_to_jump` | `AffExpr` / `QuadExpr` | 13 |

None of the four is longer than sixty lines, and all four are keyed by the
same chain-committed operator IRIs. **This is the shared payload's payoff in
the only form the code supports: one interpreter per *consuming institution*,
where a pairwise scheme between the same four consumers would need one
translator per *ordered pair* they actually exchanged terms over.**

That substitution is what makes a fifth or sixth numerical institution cheap
to add: it brings one walker with it and can then read terms from every
institution already on the chain, rather than negotiating a format with each
of them. Nothing in the repository measures this — no test enumerates pairs
and no design document counts adapters — so treat it as a structural
property of the source, not as a benchmarked result.

Three smaller consequences follow from the same shape:

- **Cross-institution dispatch is free at the wire.** When a
  `SymbolicExpression` gets reified as an `IntervalFunction` via the
  comorphism, no payload conversion happens. The chain bytes that encoded
  the original term encode the reified term; only the containing class
  changes. (This holds for `symbolics_to_intervals`, the one identity on
  FormulaTerm; the other two carry a composite around the term.)
- **Handlers don't repeat decoder logic.** The mirror generator emits one
  `decode_FormulaTerm` per institution's mirror package — but they're
  identical (auto-generated from the same chain inductive). The handlers
  consume `FormulaTerm` mirror structs directly via Julia multiple dispatch,
  without per-institution decoder differences.
- **The validator catches arity errors uniformly.** A 3-argument App-spine
  against `add`'s 2-argument signature gets rejected the same way whether it
  landed via Symbolics, JuMP, or hand-authored Eigon-JSON.

One seam the walkers do *not* close: the operator catalog and the
interpreters are separately maintained. Four of the catalog's seventeen
function operators — `eq`, `lt`, `le`, `derivative` — appear in no
interpreter's map. A term using one commits cleanly (Rule 17 checks arity and
nothing else) and fails at dispatch. Nothing checks that the two vocabularies
agree.

## 2.4b. What the shared payload did *not* buy

Two things the argument in its stronger forms predicts, which the shipped
code does not show.

**The comorphism-side translation was relocated, not removed.** A
comorphism's declared triple resolves to two Julia procedures plus a term:
the export format's `procedure`, the middle, and the import format's
`procedure`. Five of the six shipped procedures are a single statement — two
of them literally `return problem`, which the declarations justify as
keeping the kernel's dispatch path uniform rather than special-casing a
no-op. The sixth, Catalyst's `compile_to_ode`, is where the Catalyst → DiffEq
translation actually lives: it parses the `@reaction_network` source,
computes `netstoichmat(rn) * oderatelaw.(reactions(rn))`, and walks each
resulting `Symbolics.Num` into a `FormulaTerm`. `symbolics_to_intervals` is
the one comorphism where export, middle *and* import are each one statement.

An export boundary is reusable in principle. On the present chain each of the
three export procedures serves exactly one comorphism, so nothing has yet
been reused and the number of translations written has not fallen.

**Where the argument predicted a shared encoder, the code was copied.**
Translating *into* `FormulaTerm` is the other direction. Symbolics needs a
`Symbolics.Num`-to-`FormulaTerm` encoder for `simplify_expression`; Catalyst
needs the same encoder for `compile_to_ode`. Both packages carry one, and
after stripping comments, docstrings and the module name from the error
messages the two implementations are character-identical. The duplication is
deliberate and documented — an in-source `TODO(common-package)` in
[`EigeniusCatalyst.jl`](../../../julia/institutions/catalyst/EigeniusCatalyst/src/EigeniusCatalyst.jl)
records that sharing would mean either Catalyst depending on Symbolics
("non-obvious") or adding a Symbolics dependency to the common package,
which changes the dependency picture for every mirror consumer.

Both objections are real. What the shared payload language did about them is
narrower than the argument promises: it fixed the *shape* both encoders must
produce, so the two copies cannot silently diverge in what they emit. It did
not remove the second copy. That is exactly the per-institution glue the
argument is supposed to eliminate, surviving in the one place where two
institutions needed the same code.

## 2.4a. A second shared payload landed: `eigentt:TypeExpr` propositions (D47)

`FormulaTerm` covers the numerical institutions. The reasoning + statistics stack (D52 + D39) sits on a *different* shared payload: chain-mirrored EigenTT type expressions, declared at `eigentt:TypeExpr` per [D47](../../../design/d47-chain-mirrored-eigentt-type-fragment.md). Same general mechanism — a chain-resident inductive type that multiple institutions consume directly — but a different semantic domain (typed propositions and dependent types, not numerical expressions) and a different bridge mechanism (the [D49 chain-witness index](../platform/justification-logic/README.md#the-d49-witness-index-how-the-kernel-admits-grounding-witnesses), not a comorphism extract-transform-reify pipeline).

The shape:

```esl
data eigentt:TypeExpr : Type 0 {
    ConstRef(core:string),                  // IRI of a class, axiom, definition or inductive
    App(eigentt:TypeExpr, eigentt:TypeExpr), // application
    Pi(eigentt:TypeExpr, eigentt:TypeExpr),  // dependent function type
    LitString(core:string),                  // string literal as a Prop argument
    LitInt(core:integer),
    Sort(core:Level),                        // universe level — a core:Level value, not an
                                             // integer, since eigenius#188 made levels
                                             // polymorphic (Zero / Succ / Max / IMax / Param)
    // ... 20 constructors in all; see the core ontology for the full set
}
```

A chain-resident value of `eigentt:TypeExpr` IS a typed proposition (when it lives in `Prop` per [D46](../../../design/d46-prop-universe-and-proof-irrelevance.md)) or a type expression. The author surface is [`type_expr(...)`](../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions) — the syntactic counterpart of `formula(...)` for the proposition language:

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

| Institution | What it reads `eigentt:TypeExpr` for |
|---|---|
| **D52 statistics** ([tutorial](../platform/statistics-institution/README.md)) | The `StatisticalAnalysisPlan`'s `null_hypothesis` / `alternative_hypothesis` / `canonical_proposition` slots carry chain-mirrored propositions. The §7.4 epistemic-scope check walks the proposition's head predicate to look up its `is_a` scope markers. |
| **D39 reasoning** ([tutorial](../platform/justification-logic/README.md)) | The `justification:Conclusion`'s `proposition` slot. The certificate's `justification:Certificate(j, P)` indices read it. The grounding constructors (`declared`/`observed`/`derived`/`verified`) hash it to compute the witness-index key. |
| **D49 chain-witness index** ([§6.4a](../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state)) | Reads `canonical_proposition` from every chain-resident resource carrying one, together with the `prov` trace attesting how it came to exist, and computes a SHA-256 hash to key the witness-admission table. |
| **Lean institution** ([tutorial](../platform/lean-institution/README.md)) | The `lean_to_reasoning` comorphism reifies a Lean proof's proposition as a `justification:VerifiedPropositionView` with a `canonical_proposition` slot — same chain shape, written by the comorphism instead of by the original author. |

### The bridge mechanism is different

For `FormulaTerm`, institutions coordinate through declared **comorphisms** — chain-resident bridges that translate one institution's view of a `FormulaTerm` value into another's, with the kernel statically type-checking the alignment ([chapter 3](03-comorphisms.md)). The transformation is *active*: one institution's runtime is invoked, output is reified back into the chain.

For `eigentt:TypeExpr`, institutions coordinate through the **witness index** ([§6.4a](../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state)). Each institution that emits a chain-resident resource with a `canonical_proposition` slot, alongside the `prov` trace attesting how it came to exist, automatically populates the per-layer witness index at construction. Each institution that consumes a proposition (notably D39, but in principle any institution that wants to admit a `ChainWitness` predicate at type-check time) reads from the same index. The composition is *passive* — D39 doesn't call D52; it just reads what D52 emitted.

This is the load-bearing structural difference between the two composition shapes. Comorphisms are explicit translation handlers; witness-index composition is implicit through a shared chain artifact shape. Both work; which one applies depends on whether the downstream institution needs the input *value translated* (comorphism, `FormulaTerm` shape) or just *cited as evidence* (witness index, `eigentt:TypeExpr` shape).

### Identity-comorphism collapse, witness-index edition

The `FormulaTerm` story includes the identity middle: when both institutions speak the same payload, the comorphism's declared transformation step is a no-op ([§2.4](#2-4-what-the-shared-payload-buys-interpreters-not-translators)) — though the translation it does not do is done by the export procedure feeding it ([§2.4b](#2-4b-what-the-shared-payload-did-not-buy)). The witness-index analog: when two institutions share the `eigentt:TypeExpr` proposition shape and the `canonical_proposition` slot, *no bridge code at all is required*. The producer institution emits the resource with `canonical_proposition` set; the consumer institution looks up the proposition by hash in the witness index. There is no comorphism to declare, no transformation to write — the shape itself is the protocol.

The drug-screening fixture at [`kernel/tests/fixtures/drug_screening.esl`](../../../kernel/tests/fixtures/drug_screening.esl) exercises this: the `claim_eig0291_lowic50` StatisticalAnalysisPlan's `canonical_proposition = HasLowIC50("urn:...:EIG_0291")` becomes available to the downstream `concl_eig0291_strong` justification:Conclusion's `derived(claim_iri, HasLowIC50(...), ...)` certificate constructor *without any bridge code on either side* — D52 emits the proposition into a chain slot D39 already reads from. See [chapter 7](07-stats-and-reasoning-walkthrough.md) for the full walkthrough.

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
   happens. Note how the shipped declaration resolves this — it does *not*
   put the compilation in the comorphism's middle. It puts a composite
   payload (`diffeq:OdeProblem`) on both sides, makes the middle the
   identity on that composite, and names the compilation as the source
   institution's `ExportFormat` procedure. The work is still visible and
   still attributable; it is attributed to Catalyst rather than to the
   bridge.

When you reach for a shared payload, you're buying *coordination*. What you
are not buying is the disappearance of the translation: it moves to whichever
boundary procedure is named as its owner. The trade-off lives where it
should — in the comorphism declaration and the ExportFormat it names
([chapter 3](03-comorphisms.md)).

## 2.6. What other shared payloads might look like

`FormulaTerm` and `eigentt:TypeExpr` are v1's two shared payloads —
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

The one-interpreter-per-consumer property documented in [§2.4](#2-4-what-the-shared-payload-buys-interpreters-not-translators)
and the zero-bridge-code property documented in [§2.4a](#2-4a-a-second-shared-payload-landed-coreeigentttype-propositions-d47)
(shared chain slot for the witness-index shape) both transfer to any
future shared payload — once the inductive is on the chain at a peer
namespace, every institution that consumes it composes either through
identity-middled comorphisms (if the runtime needs to read the value) or
through the appropriate witness/admission mechanism (if the value is being
cited as evidence rather than processed). The caveats in
[§2.4b](#2-4b-what-the-shared-payload-did-not-buy) transfer too: expect the
translation to move to the boundary procedures rather than to vanish, and
expect a second consumer of the same *encoding* direction to need explicit
packaging if it is not to be copied.

---

Next: **[3. Comorphisms — bridges between domains →](03-comorphisms.md)**
