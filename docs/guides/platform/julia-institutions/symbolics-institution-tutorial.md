# Tutorial: typed formulas across institutions, end-to-end

This walkthrough exercises the Symbolics institution and, through it, the chain-shared formula language [D32](../../../design/d32-chain-mirrored-mini-tt-inductives.md) prescribes. By the end you will have:

- Committed a typed inductive expression tree on the chain and watched the validator type-check it against a constructor schema and a typed operator catalog.
- Watched four kinds of chain-committable claims (`SimplifiesTo`, `Substitutes`, `SatisfiesEquation`, `SymbolicallyReducesTo`) fire AutoOnLoad gates that re-run Symbolics' simplifier and produce a `Verdict` per claim.
- Dispatched an OnDemand QueryClass via FIBER and a Decidable QueryClass via EigenQL, both passing chain-committed `SymbolicExpression`s by IRI and seeing them land on the worker as fully-decoded mirror structs.

The script form of this tutorial lives at [`demo/symbolics/run.sh`](../../../../demo/symbolics/run.sh) — `./demo/symbolics/run.sh` runs the same sequence non-interactively. Read this if you want to understand *why* the chain shapes look the way they do and *what D32 buys you* once they're in place.

If you haven't seen [the intervals tutorial](intervals-institution-tutorial.md) yet, read that first — it covers the substrate plumbing (mirror, env build, AutoOnLoad dispatch) at a slower pace. This tutorial assumes those mechanics and focuses on what makes the Symbolics institution structurally different.

## What's different about Symbolics

Every claim and QueryClass in the Symbolics institution operates on *expression trees* — `SymbolicExpression`, `SymbolicEquation`, `SimplifiesTo`, `Substitutes`, `SatisfiesEquation`, `SymbolicallyReducesTo` all carry `FormulaTerm`-typed payloads. A `SymbolicExpression` carries a `term: FormulaTerm` field whose value is a recursive tree like

```
App(App(OpRef("formulas:ops:add"), Var("x")), LitFloat(0.0))
```

i.e. `x + 0`. That's a typed inductive value, validated at commit time against a chain-declared constructor schema and a chain-declared operator catalog, mirrored to a Julia abstract-type-plus-per-ctor-struct hierarchy that the worker's handler dispatches on naturally.

The IntervalArithmetic institution has the *same* FormulaTerm machinery available — its `qc_compute_bounds` OnDemand QueryClass takes a `BoundsRequest` carrying a SymbolicExpression, and its `IntervalFunction` class wraps a FormulaTerm too (see [the intervals tutorial step 10](intervals-institution-tutorial.md#step-10--beyond-the-autoonload-gate)). The difference is one of *scope*: in intervals the FormulaTerm-consuming surface is a step beyond the basic AutoOnLoad gate over flat `BoundedBy(value, lower, upper)` triples; in Symbolics, FormulaTerm is the *whole institution* — every chain-committable claim is a statement about expression trees. That's also why Symbolics is the natural place to ground the D32 walkthrough: it's where the typed-formula machinery is load-bearing top-to-bottom rather than scoped to one OnDemand path.

[D32](../../../design/d32-chain-mirrored-mini-tt-inductives.md) is the design doc that makes that work. Its three load-bearing claims:

1. **`FormulaTerm` is a fragment of EigenTT `Exp`.** Constructors mirror `Exp::Var`, `Exp::App`, `Exp::Lam`, `Exp::Pi` one-for-one (D32 §4.1). The chain doesn't need a separate term language; it lifts a chosen subset of the kernel's existing `Exp` onto the chain via the `core:InductiveType` surface that the chain already had (D32 §3.1).
2. **`FormulaTerm` is shared across every numerical institution** — it lives at `urn:eigenius:formulas:`, *not* at `urn:eigenius:symbolics:`. A `Comorphism` Symbolics → IntervalArithmetic carries the *identity* function on `FormulaTerm` as its typed middle (D32 §6.2). The cross-institution claim is operationally proven by [`crates/eigenius-julia/tests/cross_institution_probe.rs`](../../../../crates/eigenius-julia/tests/cross_institution_probe.rs) — and chain-formalised by the comorphism resources at [`julia/comorphisms/symbolics-to-intervals.eigon.json`](../../../../julia/comorphisms/symbolics-to-intervals.eigon.json).
3. **Operators carry on-chain EigenTT type signatures** (D32 §5.2). `formulas:ops:add` declares `Real → Real → Real` as a chain-committed `FormulaTerm`. The validator rank-checks every `App` invocation in a `FormulaTerm` value against the spine of `Pi` binders in the operator's signature (D32 §5.4). Mismatched arity is rejected at commit, not at dispatch.

Everything else in this tutorial is downstream of those three claims.

## How `FormulaTerm` values are encoded on the chain

Before any of the JSON in the upcoming steps will make sense, you need the encoding rule (D32 §3.7). Every `FormulaTerm` value is a tagged-dict tree:

```json
{ "ctor": "<CtorName>", "args": [ <arg₀>, <arg₁>, ... ] }
```

`<CtorName>` is one of `Var`, `LitFloat`, `OpRef`, `App`, `Lam`, `Pi` — the six ctors of the `formulas:FormulaTerm` `InductiveType`. `args` is an ordered list whose contents match the ctor's `arg_types` declared in [`formulas-ontology.json`](../../../../ontologies/formulas/formulas-ontology.json):

| Ctor | `args` shape |
|---|---|
| `Var(name)` | `["x"]` — one string |
| `LitFloat(value)` | `[2.5]` — one float |
| `OpRef(iri)` | `["urn:eigenius:formulas:ops:add"]` — one IRI string |
| `App(head, arg)` | `[<head FormulaTerm>, <arg FormulaTerm>]` — two recursive nodes |
| `Lam(name, ty, body)` | `[<string>, <FormulaTerm>, <FormulaTerm>]` |
| `Pi(name, ty, body)` | `[<string>, <FormulaTerm>, <FormulaTerm>]` |

**Multi-arg operators are curried via left-spined `App`s.** `f(a, b)` is `App(App(OpRef("f"), a), b)`; `g(a, b, c)` is `App(App(App(OpRef("g"), a), b), c)`. The chain doesn't have a variadic `App` — it mirrors EigenTT's binary application discipline directly (D32 §4.1, §2.2).

For example, `(x + 0) * 1` expands fully to:

```json
{ "ctor": "App",
  "args": [
    { "ctor": "App",
      "args": [
        { "ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"] },
        { "ctor": "App",
          "args": [
            { "ctor": "App",
              "args": [
                { "ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"] },
                { "ctor": "Var", "args": ["x"] }
              ] },
            { "ctor": "LitFloat", "args": [0.0] }
          ] }
      ] },
    { "ctor": "LitFloat", "args": [1.0] }
  ]
}
```

Reading from outermost-in: top-level `App(_, LitFloat(1.0))` is `_ * 1`; the `_` is `App(OpRef(mul), App(App(OpRef(add), Var(x)), LitFloat(0.0)))` which curries to `mul(add(x, 0))` — i.e. `(x + 0) * 1`.

### Notation in this tutorial

To keep the JSON readable, this document uses the shorthand

```
"symbolics:term": { ...FormulaTerm for <surface notation>... }
```

as a placeholder for the full nested expansion. **It's purely editorial — neither the chain nor the demo accepts that literal text.** [Step 7](#step-7--first-autoonload-simplifiesto-says-x0-simplifies-to-0) shows one full expansion inline so you have a concrete reference; later steps elide their `term` payloads. The exact JSON every step actually loads lives in [`demo/symbolics/run.sh`](../../../../demo/symbolics/run.sh).

## Prerequisites

- The compose stack running: `EIGENIUS_MOCK_LLM=true docker compose up -d`.
- A reachable Docker daemon on the host. (See [the intervals tutorial](intervals-institution-tutorial.md#prerequisites) for the substrate-depot bind-mount detail — same setup applies.)
- `jq` for parsing JSON output.
- The workspace built once.

The institution sources used throughout live at [`julia/institutions/symbolics/`](../../../../julia/institutions/symbolics/):

```
julia/institutions/symbolics/
├── declarations/
│   ├── symbolics-ontology.eigon.json     # SymbolicExpression, claim classes,
│   │                                     # input classes for QueryClasses,
│   │                                     # SymbolicEquation + VariableBinding,
│   │                                     # ReductionStrategy inductive
│   └── symbolics-institution.eigon.json  # Institution / RuntimeMethodSignatures
│                                         # / QueryClasses (AutoOnLoad x4 +
│                                         #  OnDemand x1 + Decidable x1) /
│                                         # ExportFormat
└── EigeniusSymbolics/
    ├── Project.toml                      # Symbolics + SymbolicUtils + EigeniusMirror
    └── src/EigeniusSymbolics.jl          # 5 handler functions covering all 6
                                          # QueryClasses
```

The shared formula language lives separately in [`ontologies/formulas/formulas-ontology.json`](../../../../ontologies/formulas/formulas-ontology.json) and is part of the kernel bootstrap — it's already on every chain by the time you start.

## Step 1 — Load the Symbolics ontology

```bash
eigenius --endpoint http://localhost:50051 \
    load julia/institutions/symbolics/declarations/symbolics-ontology.eigon.json
```

### What just happened

The Symbolics ontology declares the institution's chain-committable surface — both the resource classes you can author claims of and the input classes for the OnDemand / Decidable QueryClasses. The full inventory:

| Resource | Kind | Role |
|---|---|---|
| `SymbolicExpression` | Class | Carries a `term: FormulaTerm`. The thing every Symbolics-touching claim is *about*. |
| `SimplifiesTo` | Class | Claim: `simplify(expr) == claimed`. Validated at commit by `simplifies_to_validity`. |
| `SymbolicEquation` | Class | An equation `lhs = rhs` between two `SymbolicExpression`s. No truth value on its own. |
| `VariableBinding` | Class | Substitution pair `(variable, expression)`. |
| `SatisfiesEquation` | Class | Claim: under the bindings, the equation holds algebraically. Validated at commit. |
| `Substitutes` | Class | Claim: substituting `variable` with `replacement` in `target` yields `result`. Validated at commit. |
| `ReductionStrategy` | InductiveType | Two ctors `Simplify` / `Expand` — typed strategy parameter for `SymbolicallyReducesTo`. |
| `SymbolicallyReducesTo` | Class | Claim: under the named strategy, source reduces to result. Validated at commit. |
| `SimplifyRequest` | Class | Composite input class for the OnDemand `qc_symb_simplify`. |
| `EquivalenceCheck` | Class | Composite input class for the Decidable `qc_symb_check_equivalence`. |

The validator checks each at commit. Worth pausing on `SymbolicExpression.term`:

```json
{
  "@id": "urn:eigenius:symbolics:term",
  "core:is_a": ["core:Property"],
  "core:data_type": "core:inductive",
  "core:class_types": ["urn:eigenius:formulas:FormulaTerm"]
}
```

The `data_type: core:inductive` plus `class_types: [formulas:FormulaTerm]` is what triggers the validator's inductive-value rule (D32 §3.5): when a `SymbolicExpression` lands carrying a `term` field, the validator walks the tagged-dict tree, looks up `formulas:FormulaTerm` on the chain, finds the six ctors (`Var`, `LitFloat`, `OpRef`, `App`, `Lam`, `Pi`), and recursively type-checks every node against the matching ctor's `arg_types`. Bad ctor names, bad argument types, and missing required arguments all reject at commit.

The `formulas:FormulaTerm` declaration itself is already on the chain — the kernel's bootstrap embeds [`ontologies/formulas/formulas-ontology.json`](../../../../ontologies/formulas/formulas-ontology.json) as a layer below `notebook`, so every chain has it. You're committing the *Symbolics-specific* surface on top of it.

## Steps 2–6 — Mirror, env image, env Resource, institution

Steps 2-6 (resolve head layer, generate mirror, build env image, commit env Resource, install institution) follow the same shape as the intervals tutorial. Three differences worth flagging:

### The mirror filter is broader

```bash
eigenius --endpoint http://localhost:50051 mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["SimplifiesTo", "SimplifyRequest",
                              "EquivalenceCheck", "Substitutes",
                              "SatisfiesEquation", "SymbolicallyReducesTo"]
              RETURN [] { iri: ?iri }' \
    --language julia \
    --output /tmp/symbolics-mirror \
    --json
```

The intervals demo seeded one class. This one seeds six — every top-level claim/input class. Closure walking pulls in the rest:

- `SymbolicExpression` (referenced by every claim's typed property)
- `SymbolicEquation`, `VariableBinding` (referenced by `SatisfiesEquation`)
- `FormulaTerm` (referenced by `SymbolicExpression.term`)
- `ReductionStrategy` and its two ctors (referenced by `SymbolicallyReducesTo.strategy`)
- The full `Operator` catalog (`add`, `sub`, `mul`, `div`, `pow`, `neg`, `exp`, `log`, `sin`, `cos`, `tan`, `sqrt`, `abs`, `eq`, `lt`, `le`, `derivative`)

The mirror generator (D32 §3.6) walks every reachable `Class` *and* `InductiveType` reference, emitting:

- `struct SymbolicExpression`, `struct SimplifiesTo`, … for each Class
- `abstract type FormulaTerm` plus six concrete structs (`FormulaTerm_Var`, `FormulaTerm_LitFloat`, `FormulaTerm_OpRef`, `FormulaTerm_App`, `FormulaTerm_Lam`, `FormulaTerm_Pi`) for each ctor of the FormulaTerm inductive
- `abstract type ReductionStrategy` plus `ReductionStrategy_Simplify`, `ReductionStrategy_Expand`
- `decode_FormulaTerm(d::AbstractDict)` that dispatches on `d["ctor"]` and recursively decodes `d["args"]` against the matching ctor's typed slots
- Symmetric `encode_FormulaTerm(t)` that produces the chain-shaped `{"ctor": "...", "args": [...]}` dict

That last pair — `decode_FormulaTerm` / `encode_FormulaTerm` — is what makes typed Julia dispatch on `FormulaTerm` values *trivial*. The Symbolics handler writes `formula_to_num(t::FormulaTerm_App)` and Julia's multiple dispatch routes to it.

### The handler does both directions

The intervals handler decodes `BoundedBy → Interval`, returns a Verdict (a Dict). The Symbolics handler also goes the other way for OnDemand: it decodes `FormulaTerm → Symbolics.Num`, runs `Symbolics.simplify`, then encodes the result *back* into a `FormulaTerm`. The `num_to_formula` direction lives at [`EigeniusSymbolics.jl`](../../../../julia/institutions/symbolics/EigeniusSymbolics/src/EigeniusSymbolics.jl); it walks the underlying `SymbolicUtils.BasicSymbolic` (Sym / Term / Number cases) and emits chain-shaped `FormulaTerm` ctors via the mirror's per-ctor structs. Adding a new operator requires entries in both directions: `_OP_FN` (decode) and `_FN_TO_IRI` (encode).

### The institution declares more dispatch roles

Where intervals declared one `QueryClass` (`bounded_by_validity`, AutoOnLoad), Symbolics declares six:

| QueryClass | Dispatch role | Input | Output |
|---|---|---|---|
| `simplifies_to_validity` | AutoOnLoad | `SimplifiesTo` | `Verdict` |
| `substitutes_validity` | AutoOnLoad | `Substitutes` | `Verdict` |
| `satisfies_equation_validity` | AutoOnLoad | `SatisfiesEquation` | `Verdict` |
| `symbolically_reduces_to_validity` | AutoOnLoad | `SymbolicallyReducesTo` | `Verdict` |
| `qc_symb_simplify` | OnDemand | `SimplifyRequest` | `SymbolicExpression` |
| `qc_symb_check_equivalence` | Decidable | `EquivalenceCheck` | `Verdict` |

Three different dispatch roles, all on one institution. Worth understanding what that means before walking through the demo's invocations:

- **`AutoOnLoad`** ([D14 §6.1](../../../design/d14-institution-realisation.md#6-verdicts)) — fires automatically every time a resource of the bound class is committed. The kernel runs the gate; on `Holds` the commit goes through with a `Verdict + RuntimeInvocation` audit anchor; on `Fails` the commit is rejected; on `Undecidable` the commit goes through but the verdict carries no domain commitment.
- **`OnDemand`** — fired explicitly via EigenQL FIBER (D2 v2 §3.5). The user writes a query that calls into the institution; the handler returns a typed result that flows into the query result set. Doesn't gate commits.
- **`Decidable`** — referenced from `Exp::NativeDecide` (D14 §6.2). User programs (and EigenQL `WHERE` predicates) call the QueryClass like a function; on `Holds` the constraint reduces to `Refl`, on `Fails` to a failing neutral, on `Undecidable` to a passthrough neutral. The kernel evaluates it during type-check reduction.

The demo exercises all three.

The env-build step takes 5+ minutes cold (Symbolics.jl is heavy — pulls SymbolicUtils, RuntimeGeneratedFunctions, etc., plus precompiles all of them). Subsequent runs hit buildah's layer cache.

## Step 7 — First AutoOnLoad: `SimplifiesTo` says `x*0` simplifies to `0`

```bash
eigenius load <<<'[
  {
    "@id": "urn:eigenius:demo:symbolics:claim:x_times_0_simplifies_to_zero",
    "core:is_a": ["urn:eigenius:symbolics:SimplifiesTo"],
    "core:short_name": "x_times_0_simplifies_to_zero",
    "symbolics:expr": {
      "core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "core:short_name": "x_times_0",
      "symbolics:term": {
        "ctor": "App",
        "args": [
          {"ctor": "App", "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
            {"ctor": "Var", "args": ["x"]}
          ]},
          {"ctor": "LitFloat", "args": [0.0]}
        ]
      }
    },
    "symbolics:simplified": {
      "core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
      "core:short_name": "zero",
      "symbolics:term": {"ctor": "LitFloat", "args": [0.0]}
    }
  }
]'
```

### What just happened

This is the first `FormulaTerm` value to land on the chain in this demo, so it's worth walking through what the validator is doing under the hood (D32 §3.5):

1. Parse the JSON, recognise the outer resource's `is_a: SimplifiesTo`, look up `SimplifiesTo` on the chain.
2. `SimplifiesTo` requires `expr` and `simplified`. Both are typed `core:resource, class_types: [SymbolicExpression]` — recurse into both.
3. Each embedded `SymbolicExpression` requires `term`. `term` is typed `core:inductive, class_types: [FormulaTerm]` — trigger the inductive-value rule:
   1. Read the value's `ctor` field (e.g. `"App"`).
   2. Look up `formulas:FormulaTerm` on the chain, find the ctor `App` with `arg_types: [head: FormulaTerm, arg: FormulaTerm]`.
   3. Recurse into `args[0]` (the `head` slot) — also expects a FormulaTerm — and `args[1]` (the `arg` slot).
   4. Bottom out at primitives: `LitFloat(0.0)` checks the `value: float` arg type; `OpRef("urn:eigenius:formulas:ops:mul")` checks the `iri: string` arg type, also resolves the IRI to a chain-committed `Operator` resource so we know `mul` exists.
4. The `App`-spine rank check (D32 §5.4): the validator walks the leftmost `App` chain, finds `OpRef("formulas:ops:mul")` at the bottom, reads `mul`'s `operator_signature` (which is itself a `FormulaTerm` — `Pi(_, Real, Pi(_, Real, Real))`, dogfooding the type language), counts the spine arguments (two), and walks down the `Pi` chain matching each. Two args, two `Pi` binders before bottoming out at `Real` — checks. If you'd written `App(App(App(OpRef("mul"), x), y), z)` (three args to a binary operator) the validator would reject with `OperatorArityMismatch`.

After the value validates, the chain commit pipeline fires the `simplifies_to_validity` AutoOnLoad gate. The kernel:

1. Looks up `simplifies_to_validity` in its `auto_on_load_by_class` index.
2. Issues a `DispatchExternal` gRPC call to the orchestrator with `(env_iri, image_digest, "validate_simplifies_to", signature_iri, [serialized SimplifiesTo CBOR])`.
3. The Julia worker decodes the input via `decode_SimplifiesTo`, which transitively decodes `expr.term` and `simplified.term` via `decode_FormulaTerm`. Each FormulaTerm becomes a typed Julia tree of `FormulaTerm_App`, `FormulaTerm_OpRef`, `FormulaTerm_Var`, `FormulaTerm_LitFloat` structs.
4. The handler's `validate_simplifies_to` translates each FormulaTerm into a `Symbolics.Num` via `formula_to_num` (multiple dispatch on the ctor structs), runs `Symbolics.simplify` on `expr_num`, and compares structurally against `claimed_num` via `isequal`.
5. `simplify(x*0)` returns `0`; `isequal(0, 0)` holds; the handler returns `_verdict("Holds")`.
6. The kernel commits the original `SimplifiesTo` resource, plus a `Verdict + RuntimeInvocation` audit anchor.

If you change the claim to assert `x*0` simplifies to `1`, you'd get `Undecidable` — Symbolics' simplifier is heuristic ([D27 §4.1.1](../../../design/d27-julia-institutions.md)), so a representational mismatch doesn't refute the claim, and `Fails` is reserved for a future strict-decision path.

## Step 8 — Inspect the Verdict

```bash
eigenius query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) {
        "urn:eigenius:core:ctor_name": ?ctor
     } RETURN [] { verdict: ?v, ctor: ?ctor }'
```

You'll see one row: `ctor = "Holds"`, `verdict = urn:eigenius:invocation:<uuid>:verdict`. Inspecting it directly (`eigenius inspect <verdict-iri>`) shows the back-reference to the `RuntimeInvocation` (timing, image digest, numerical metadata).

## Step 9 — Commit input expression for OnDemand FIBER

The next step uses `qc_symb_simplify` (an OnDemand QueryClass) via FIBER. FIBER takes a single typed input resource and returns a single typed output. We pre-commit the expression to simplify so the FIBER call can pass it by IRI:

```bash
eigenius load <<<'[
  {
    "@id": "urn:eigenius:demo:symbolics:expr:x_plus_0_times_1",
    "core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
    "core:short_name": "x_plus_0_times_1",
    "symbolics:term": { ...FormulaTerm for (x + 0) * 1... }
  }
]'
```

A standalone `SymbolicExpression` doesn't fire any AutoOnLoad gate — there's no `query_class: SymbolicExpression` declared. It just lands on the chain.

## Step 10 — OnDemand FIBER: `qc_symb_simplify`

```bash
eigenius query \
    'USING INSTITUTION "urn:eigenius:institutions:symbolics" AS cap
     FIBER cap:qc_symb_simplify {
       expr: "urn:eigenius:demo:symbolics:expr:x_plus_0_times_1"
     } AS ?simplified
     RETURN [] { result: ?simplified, term: ?simplified.term }'
```

### What just happened

A FIBER query packs its parameters into the QueryClass's typed input class. Here `qc_symb_simplify` declares `query_class: SimplifyRequest`, and `SimplifyRequest` requires one user-supplied property: `expr: SymbolicExpression`.

The kernel's FIBER eval path (which mirrors the same plumbing the Decidable path uses, see Step 16):

1. Resolves `cap:qc_symb_simplify` to the `qc_symb_simplify` QueryClass entry, confirms its dispatch role includes `OnDemand`.
2. Builds a synthetic `SimplifyRequest` input resource. The `expr: "urn:..."` parameter is an IRI string; the kernel's IRI-dereference pass (in `embed_typed_resource_arg`) sees that `SimplifyRequest.expr` declares `data_type: core:resource, class_types: [SymbolicExpression]`, looks up the IRI on the layer, and *embeds* the chain-committed SymbolicExpression into the synthetic input. So the input the worker sees is a fully-typed `SimplifyRequest{expr: SymbolicExpression{term: FormulaTerm{...}}}`, not a bare IRI string.
3. Dispatches `simplify_expression(req::SimplifyRequest)` on the worker. The handler decodes `req.expr.term` to a `Symbolics.Num`, runs `Symbolics.simplify`, encodes the result back to a `FormulaTerm` via `num_to_formula`, and returns a fresh `SymbolicExpression` mirror struct.
4. The worker's encoder pipeline walks the returned `SymbolicExpression`, calls `encode_SymbolicExpression` which calls `encode_FormulaTerm` on the term, and produces a CBOR map matching the chain's typed shape.
5. The kernel parses the response, stamps a synthetic IRI on it (the `?simplified` binding), and threads it into the rest of the query so `?simplified.term` projects to the simplified form.

`(x + 0) * 1` simplifies to `x` under Symbolics. The output's `term` is `Var("x")`, encoded as `{"ctor": "Var", "args": ["x"]}`.

### Why this works without per-institution glue

The FIBER input IRI got dereferenced to an embedded SymbolicExpression. The handler's signature `simplify_expression(req::SimplifyRequest)` took the typed mirror struct directly. Nowhere in this round-trip did anyone parse JSON, walk a property map, or manually decode a FormulaTerm — every hop was driven by typed dispatch on chain-mirrored structs. That's the whole D29 / D32 mechanism.

## Steps 11–13 — Three more AutoOnLoad gates

These three extend the chain-committed-claim surface beyond `SimplifiesTo`. Each follows the same pattern: commit a claim, the AutoOnLoad gate fires `Symbolics.<operation>`, a Verdict lands.

### Step 11 — `Substitutes`: `x ↦ 0` in `x + 1` yields `1`

```json
{
  "core:is_a": ["urn:eigenius:symbolics:Substitutes"],
  "symbolics:target": { ...x + 1... },
  "symbolics:variable": "x",
  "symbolics:replacement": { ...LitFloat 0... },
  "symbolics:result": { ...LitFloat 1... }
}
```

The handler `validate_substitutes` runs `Symbolics.substitute(target_num, x => 0)`, simplifies the result and the claim, compares via `isequal`. `0 + 1` simplifies to `1`; matches; Holds.

### Step 12 — `SatisfiesEquation`: `x + 0 = x` (empty bindings)

```json
{
  "core:is_a": ["urn:eigenius:symbolics:SatisfiesEquation"],
  "symbolics:equation": {
    "core:is_a": ["urn:eigenius:symbolics:SymbolicEquation"],
    "symbolics:lhs": { ...x + 0... },
    "symbolics:rhs": { ...x... }
  },
  "symbolics:bindings": []
}
```

`SatisfiesEquation` is more interesting than the others because `bindings` is a list of typed substitution pairs (`VariableBinding { variable, expression }`). An empty list claims unconditional algebraic equivalence; a non-empty list claims "under these substitutions the equation holds." The handler builds a `Symbolics.substitute` map from the bindings, applies it to both sides, simplifies the residue `lhs_subbed - rhs_subbed`, and checks `iszero`.

This particular claim has empty bindings, which collapses to "is `simplify(x + 0 - x)` zero?" — `Symbolics.simplify` cancels the residue, `iszero` holds, Verdict is Holds.

### Step 13 — `SymbolicallyReducesTo` under `Expand`: `2*(x+1)` → `2*x + 2`

```json
{
  "core:is_a": ["urn:eigenius:symbolics:SymbolicallyReducesTo"],
  "symbolics:expr": { ...2*(x+1)... },
  "symbolics:strategy": { "ctor": "Expand", "args": [] },
  "symbolics:result": { ...2*x + 2... }
}
```

This is the strategy-parametric reduction claim. `ReductionStrategy` is a chain-declared `InductiveType` with two ctors (`Simplify`, `Expand`). The validator's inductive-value rule type-checks the strategy field at commit; the handler dispatches on the strategy via Julia method dispatch:

```julia
apply_strategy(::ReductionStrategy_Simplify, expr) = Symbolics.simplify(expr)
apply_strategy(::ReductionStrategy_Expand, expr) = Symbolics.expand(expr)
```

`Symbolics.expand(2*(x+1))` distributes to `2x + 2`; matches the claim; Holds.

The strategy-parametric design means adding a new strategy is one line of ontology (a new ctor on `ReductionStrategy`) plus one line of Julia (a new `apply_strategy` method). No conditional dispatch in the handler, no per-strategy branch in the validator.

### What's interesting about each

These three claim types are all conceptually different windows onto the same Symbolics machinery:

- **`SimplifiesTo`**: "the rewriter normalises `expr` to `simplified`."
- **`Substitutes`**: "`target[variable ↦ replacement]` equals `result`."
- **`SatisfiesEquation`**: "under the bindings, `lhs ≡ rhs` algebraically."
- **`SymbolicallyReducesTo`**: "the named strategy reduces `expr` to `result`."

You could collapse them all into one giant `SymbolicClaim` with a strategy enum and optional substitution bindings. But each has a sharper semantic: a `Substitutes` claim *is* a substitution, not a generalised reduction; a `SatisfiesEquation` claim has an honest equation as a first-class chain resource that other claims can reference. Keeping them distinct keeps each commit's *meaning* clear, even if they all dispatch through the same `Symbolics` library underneath.

## Step 14 — Verdict round-up

```bash
eigenius query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) {
        "urn:eigenius:core:ctor_name": ?ctor
     } RETURN [] { verdict: ?v, ctor: ?ctor }'
```

Four AutoOnLoad-fired Verdicts now exist on the chain (one per claim from steps 7, 11, 12, 13). Each carries a back-reference to the `RuntimeInvocation` that produced it — same audit closure as the intervals tutorial, just four claims deep.

## Step 15 — Pre-commit lhs/rhs for the Decidable check

```bash
eigenius load <<<'[
  {"@id": "urn:eigenius:demo:symbolics:expr:lhs_x_plus_0", ...x + 0...},
  {"@id": "urn:eigenius:demo:symbolics:expr:rhs_x", ...x...}
]'
```

Two SymbolicExpressions. Same pattern as Step 9 — pre-commit the inputs so the next step can reference them by IRI.

## Step 16 — Decidable EigenQL: `qc_symb_check_equivalence`

```bash
eigenius query \
    'USING INSTITUTION "urn:eigenius:institutions:symbolics" AS cap
     RETURN [] {
       verdict: cap:qc_symb_check_equivalence(
         "urn:eigenius:demo:symbolics:expr:lhs_x_plus_0",
         "urn:eigenius:demo:symbolics:expr:rhs_x"
       )
     }'
```

### What just happened

This is the third dispatch role at work. The Decidable surface ([D14 §6.2](../../../design/d14-institution-realisation.md#6-verdicts)) treats a QueryClass as a *function call* in EigenQL expression position. `cap:qc_symb_check_equivalence(?lhs_iri, ?rhs_iri)` parses as a function-call expression; the kernel's expression evaluator recognises that the qualified function name resolves to a Decidable-role QueryClass and dispatches it.

The pipeline is structurally identical to the OnDemand FIBER one — and shares the same kernel helper [`institution::marshal::marshal_decidable_input`](../../../../kernel/src/institution/marshal.rs):

1. The kernel resolves `cap:qc_symb_check_equivalence` to its QueryClass entry, confirms `Decidable` is one of its dispatch roles.
2. Reads the input class (`EquivalenceCheck`) and its `requires: [lhs, rhs]` (excluding kernel-managed `is_a` / `short_name`).
3. The two positional args are IRI strings. For each, the kernel notices the corresponding property declares `data_type: core:resource, class_types: [SymbolicExpression]`, looks the IRI up on the layer, and embeds the chain-committed SymbolicExpression into the synthetic input.
4. The synthetic `EquivalenceCheck{lhs: SymbolicExpression{...}, rhs: SymbolicExpression{...}}` flows to the worker. `decode_EquivalenceCheck` produces a typed mirror struct; the handler's `check_equivalence(check::EquivalenceCheck)` simplifies both sides and returns `Holds` on `isequal`.
5. The Verdict comes back as a `Value::Embedded`; the EigenQL query result row carries it under `verdict`.

`simplify(x + 0)` and `simplify(x)` both produce `x`; `isequal` holds; the row contains a Verdict whose `ctor_name` is `Holds`.

### Why Decidable is structurally different from OnDemand

Both end up calling `Institution::query` with a typed input, both return a typed output. The differences are at the *user-facing surface*:

- A FIBER OnDemand call is a **clause** in EigenQL — `FIBER cap:qc { params } AS ?var` — that adds a new variable binding to subsequent clauses. The result is a typed resource that downstream patterns can decompose.
- A Decidable call is an **expression** in EigenQL — `cap:qc(args)` — that returns a Verdict value usable in `WHERE` filters (via postfix Verdict predicates `:Holds` / `:Fails` / `:Undecidable`) or in `RETURN` projections.

But the deeper distinction (D14 §6.2): Decidable is the role `Exp::NativeDecide` reduces. User-authored ESL programs can write predicates like `decide cap:are_eq(x, y)`; the kernel's evaluator reaches the predicate during type-check normalisation, dispatches it through the same `try_d14_decide` path, and reduces the surrounding term according to the verdict. `Holds` reduces the constraint to `Refl`; `Fails` produces a failing neutral; `Undecidable` leaves it as a passthrough. That's how Decidable predicates participate in *type-checking*, not just in queries.

## What now lives on the chain

After the full demo runs, the following resources have been committed (all reachable via `eigenius inspect <iri>`):

| Resource | Source |
|---|---|
| Symbolics ontology classes (10 of them) | step 1 |
| `RuntimePackageMirror` | step 3 |
| `RuntimeEnvironment` (`urn:eigenius:symbolics:env:v1`) | step 5 |
| `Institution`, 6 RuntimeMethodSignatures, 6 QueryClasses, ExportFormat | step 6 |
| `SimplifiesTo` claim (`x*0 → 0`) + Verdict + RuntimeInvocation | step 7 |
| `SymbolicExpression` (`(x + 0) * 1`) | step 9 |
| `Substitutes` claim + Verdict | step 11 |
| `SatisfiesEquation` claim + Verdict | step 12 |
| `SymbolicallyReducesTo` claim + Verdict | step 13 |
| Two `SymbolicExpression`s (`x + 0`, `x`) | step 15 |

The OnDemand and Decidable invocations (steps 10 and 16) don't commit anything — both surfaces are read-only with respect to the chain ([D14 §6.2](../../../design/d14-institution-realisation.md#6-verdicts)). Their audit trail rides on the EigenQL query trace, not on the chain.

## What this exercises about D32

Everything in this demo is downstream of the three D32 claims listed at the top:

- **`FormulaTerm` is a fragment of EigenTT.** Every `term` field on every `SymbolicExpression` is a typed inductive value validated against the `core:InductiveType` schema D32 §3 specifies. Steps 7, 11, 12, 13 commit four different shapes of FormulaTerm and each is type-checked at commit.
- **`FormulaTerm` is shared across institutions.** The same FormulaTerm shape that this demo's Symbolics handler decodes is also what the IntervalArithmetic handler in the cross-institution probe ([`crates/eigenius-julia/tests/cross_institution_probe.rs`](../../../../crates/eigenius-julia/tests/cross_institution_probe.rs)) decodes — different institutions, identical chain-side payload. Step 9's pre-committed SymbolicExpression could have been authored in a Catalyst or DiffEq context with the same wire shape.
- **Operators carry typed signatures.** Every `OpRef` in every FormulaTerm in this demo got rank-checked against an on-chain `Operator.operator_signature` at commit time (D32 §5.4). Authoring `App(App(OpRef("formulas:ops:add"), Var("x")), LitFloat(0.0), LitFloat(1.0))` — an extra arg — gets rejected before the worker ever sees it.

The runtime mechanics — three dispatch roles, six QueryClasses, four claim types — are the surface. The chain shapes underneath (typed inductive values, typed operator signatures, mirror-driven Julia dispatch) are what makes them composable across institutions.

## Common failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `expected one of: Var, LitFloat, OpRef, App, Lam, Pi; got: Foo` at commit | A FormulaTerm value carries a ctor name that's not declared on the chain. Misspelling or stale ontology. | Match the ctor name exactly to one declared on `formulas:FormulaTerm`. |
| `OperatorArityMismatch: formulas:ops:mul: expected 2 args, got 3` | An `App` spine has more arguments than the operator's signature has `Pi` binders. | Either refactor the call (multi-arg ops are curried, not n-ary) or check the operator's signature in `ontologies/formulas/formulas-ontology.json`. |
| `unknown operator urn:eigenius:formulas:ops:foo` from the worker | Handler's `_OP_FN` map (decode side) or `_FN_TO_IRI` map (encode side) doesn't have an entry for the operator. | Add entries in both directions in `EigeniusSymbolics.jl`, rebuild the env image. |
| `MethodError: no method matching encode_FormulaTerm(::ActuallyComplex)` | `num_to_formula` hit a Symbolics term shape it doesn't know how to translate. | Extend `num_to_formula`'s SymbolicUtils dispatch, or accept that the result will land as Undecidable rather than Holds. |
| Decidable returns a Verdict but the row contains an unexpected shape | The Verdict is `Value::Embedded(Resource)` with `ctor_name: "Holds"|"Fails"|"Undecidable"`. EigenQL projects the whole resource; project `ctor_name` directly if you want the string. | `RETURN [] { ctor: cap:qc(...).ctor_name }` or use postfix `:Holds`. |

For the substrate-level failure modes (`seed manifest drift`, depot bind-mount issues, manifest-hash mismatch), see [the intervals tutorial](intervals-institution-tutorial.md#common-failure-modes).
