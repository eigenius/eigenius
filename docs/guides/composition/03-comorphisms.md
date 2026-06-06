# 3. Comorphisms — bridges between domains

A **comorphism** is the declared, type-checked bridge between two
institutions. It is the load-bearing concept of this guide, and the only
mechanism the platform provides for systematic cross-institution flow.

This chapter is the structured reference for how comorphisms work, what they
guarantee, and how to read one end-to-end. Five things to internalise:

1. A comorphism is **triadic** — `ExportFormat` + `transformation` Component
   + `ImportFormat` — and the kernel statically type-checks the alignment.
2. Dispatch follows a **four-step pipeline**: extract → transform → reify →
   (optionally) chain-reinsert.
3. Comorphisms come in two flavours: **identity** (when both endpoints share
   a payload language) and **structural** (when the transformation does
   real work).
4. The **`exact: bool`** Satisfaction-Condition annotation captures a real
   semantic claim about the bridge, not just a metadata flag.
5. **Authoring** a new comorphism is mostly a chain-commit exercise — the
   kernel does the validation.

The chapter closes with a note (§3.9) on the theoretical foundations of the
framework and an open research direction.

## 3.1. Triadic structure: ExportFormat, transformation, ImportFormat

A comorphism is a single chain-resident `Comorphism` resource that names
three other chain-resident resources:

```json
{
  "@id": "urn:eigenius:comorphisms:symbolics_to_jump",
  "core:is_a": ["institution:Comorphism"],
  "institution:export_format": "urn:eigenius:symbolics:formats:ef_symb_to_jump_input",
  "institution:transformation": "urn:eigenius:comorphisms:symbolics_to_jump:m_id_optimisation_problem",
  "institution:import_format": "urn:eigenius:jump:formats:if_jump_optimisation_problem",
  "institution:exact": false
}
```

| Piece | What it carries | Provides |
|---|---|---|
| **`ExportFormat`** | `from_class` (source-side resource class), `payload_type` (the typed EigenTT value the export produces), `institution_ref` (which institution owns the boundary), `procedure` (handler IRI) | The *outbound* boundary of the source institution. |
| **`transformation`** | A EigenTT Component with signature `payload_type(export) → payload_type(import)`. May be the identity function. | The structure-preserving translation between the two payloads. |
| **`ImportFormat`** | `to_class` (target-side resource class), `payload_type` (the typed value the import consumes), `institution_ref`, `procedure` (handler IRI) | The *inbound* boundary of the target institution. |

The triple is what's chain-resident; the actual handler code (the
`extract_typed` and `reify` implementations) lives in each institution's
runtime (Rust crate for in-process, WASM binary for sandboxed, or a
language-runtime worker container for substrate-hosted institutions).

The kernel resolves a comorphism dispatch by looking up the triple in the
[`InstitutionIndex`](../../../kernel/src/institution/registry.rs), then
calling the source institution's `extract_typed` handler, then evaluating the
transformation Component, then calling the target institution's `reify`
handler. The full path is the four-step pipeline (§3.2).

## 3.2. The four-step dispatch pipeline (D14 §9.3)

Whichever surface invokes the comorphism — ESL `Exp::InstitutionInvoke`,
EigenQL FIBER param coercion, or EigenQL `FIBER ... INTO` — the kernel runs
the same pipeline:

```
            source resource (e.g. SymbolicsToJuMPInput)
                              │
                              │  step 1: extract_typed
                              │   (source institution's handler)
                              ▼
                  typed payload (EigenTT value)
                  e.g. FormulaTerm
                              │
                              │  step 2: transformation Component
                              │   (EigenTT evaluator)
                              ▼
                  typed payload (EigenTT value)
                  e.g. FormulaTerm (unchanged for identity)
                              │
                              │  step 3: reify
                              │   (target institution's handler)
                              ▼
            target resource (e.g. OptimisationProblem)
                              │
                              │  step 4: chain reinsertion
                              │   (commit_with_validation; chapter 5)
                              ▼
              committed at urn:eigenius:comorphism-output:<tail>:<hex>
              (or at a caller-named IRI under FIBER ... INTO)
```

Walking through the kinase notebook's `symbolics_to_jump` comorphism:

1. **Extract.** The source institution is Symbolics; the source resource is
   a `SymbolicsToJuMPInput` carrying the SSE objective as a `SymbolicExpression`.
   Symbolics's `extract_typed` handler reads the expression's `term` field
   (a `FormulaTerm`) and returns it as the typed payload.
2. **Transform.** The transformation Component is the identity on
   `FormulaTerm` — `Lambda(t: FormulaTerm. Var(t))`. The EigenTT evaluator
   reduces the application; the payload comes out unchanged.
3. **Reify.** The target institution is JuMP-HiGHS; the target resource
   class is `OptimisationProblem`. JuMP-HiGHS's `reify` handler takes the
   `FormulaTerm` payload, packs it into an `OptimisationProblem` (with the
   `variable_names`, `variable_bounds`, and `sense` fields lifted from the
   `SymbolicsToJuMPInput`), and returns the new resource.
4. **Reinsert.** The kernel commits the produced `OptimisationProblem` to
   the chain at a deterministic content-hash IRI of the form
   `urn:eigenius:comorphism-output:symbolics_to_jump:<hex>`, where `<hex>`
   is SHA-256 over the canonical Eigon-CBOR of the resource (with `@id`
   cleared).

Steps 1–3 are the original D10 four-step machinery (D14 §9.3 retains them
verbatim); step 4 is what Phase 19i added — without chain reinsertion the
reified resource would be transport-only, alive for the duration of the
dispatch but not commitment-traceable. Chapter 5 covers reinsertion in
detail.

## 3.3. Identity transformations

When both endpoints of a comorphism share the payload language (chapter 2),
the `transformation` Component is **the identity function**:

```
m_id_formula_term : FormulaTerm → FormulaTerm
m_id_formula_term = Lambda(t: FormulaTerm. Var(t))
```

The chain bytes flow through unchanged. From
[`julia/comorphisms/symbolics-to-intervals.eigon.json`](../../../julia/comorphisms/symbolics-to-intervals.eigon.json):

```json
{
  "@id": "urn:eigenius:comorphisms:symbolics_to_intervals:m_id_formula_term",
  "core:is_a": ["program:Component"],
  "program:capability_level": "urn:eigenius:program:capability:Pure",
  "program:input_type":  "urn:eigenius:formulas:FormulaTerm",
  "program:output_type": "urn:eigenius:formulas:FormulaTerm",
  "program:body": {
    "ctor": "Lam",
    "args": [
      "t",
      {"ctor": "OpRef", "args": ["urn:eigenius:formulas:FormulaTerm"]},
      {"ctor": "Var", "args": ["t"]}
    ]
  }
}
```

What identity transformations buy:

- **Zero runtime cost.** The EigenTT evaluator reduces `Lam(t. Var(t))`
  applied to `<payload>` to `<payload>` in one β-step. No work happens at
  the wire.
- **Trivial type-check.** The transformation's signature is
  `FormulaTerm → FormulaTerm`. Both `payload_type(export)` and
  `payload_type(import)` are `FormulaTerm`. The alignment check (§3.6)
  passes without effort.
- **Audit-clarity.** The audit trail says exactly what happened: "Symbolics
  produced FormulaTerm `T`; the comorphism's identity transformation
  preserved `T`; JuMP consumed FormulaTerm `T`." Bit-for-bit equivalence is
  honest and recordable.

Two of the kinase notebook's three comorphisms (`symbolics_to_intervals` and
`symbolics_to_jump`) use identity transformations because both endpoints
agree on `FormulaTerm` as the payload. The third (`catalyst_to_diffeq`) is
structural — the next section.

## 3.4. Structural transformations: Catalyst → DiffEq

When the source and target speak different payload languages, the
transformation does *real work*. The canonical worked example is
`catalyst_to_diffeq`: Catalyst speaks `ReactionNetwork`, DiffEq speaks
`OdeProblem`. Compiling a network into an ODE right-hand side is structurally
interesting — it's not just renaming.

The shape of the transformation:

```
m_compile_to_ode : CatalystToOdeInput → OdeProblem

m_compile_to_ode(input) = {
    state_names:        species_of(input.network),
    parameter_names:    parameters_of(input.network),
    rhs:                [rhs_component_for(s, input.network) | s ∈ species_of(input.network)],
    initial_conditions: input.initial_conditions,
    parameters:         input.parameters,
    time_span_start:    input.time_span_start,
    time_span_end:      input.time_span_end
}
```

`rhs_component_for(species, network)` is the load-bearing piece: for each
species, walk the reaction network's rate equations and emit a `FormulaTerm`
for `dSpecies/dt`. The mass-action ODE for `A --k--> B` produces two
RhsComponents:

| Species | dSpecies/dt | FormulaTerm |
|---|---|---|
| A | `−k · A` | `App(App(OpRef(mul), App(App(OpRef(mul), LitFloat(-1)), Var(A))), Var(k))` |
| B | `k · A` | `App(App(OpRef(mul), Var(A)), Var(k))` |

The transformation's *input* is a `CatalystToOdeInput` (a typed wrapper
around a `ReactionNetwork` plus initial conditions / parameters). The
*output* is an `OdeProblem` with FormulaTerm-typed `RhsComponent`s. So the
transformation is structural at the wire (different shapes), but the
*pieces* of the output are FormulaTerm-shaped — which means the DiffEq side
of the bridge consumes the result the same way it consumes any other
`OdeProblem`.

Why this matters for the audit trail: a structural transformation can lose
information. The mass-action ODE compilation captures the *deterministic
limit* of the reaction kinetics; it loses the stochastic structure of the
underlying chemical master equation. That semantic claim is what the next
section's `exact` flag captures.

## 3.5. The `exact` flag and Satisfaction-Conditions

Every `Comorphism` carries a boolean `exact: bool` field. This is not
metadata — it's a substantive claim about the bridge:

| `exact` | What the comorphism asserts |
|---|---|
| `true` | The bridge preserves *every* sentence's truth. If a sentence holds in the source institution's model, the translated sentence holds in the target's model, and vice versa. Bit-for-bit payload preservation; no semantic loss. |
| `false` | The bridge is sound but lossy. Some structural information is collapsed, approximated, or projected during the transformation. The translated result is still a faithful representation of *some aspect* of the source, but not all of it. |

In institution theory this is the **Satisfaction-Condition** (Goguen &
Burstall, 1992): a comorphism `(Φ, α, β)` satisfies the SC iff for every
signature `Σ` and every Σ-sentence `e`, the model translation `β` and the
sentence translation `α` commute with the satisfaction relation. `exact:
true` is the chain-side declaration that the comorphism satisfies the SC;
`exact: false` declares that it doesn't.

The kinase notebook's three comorphisms:

| Comorphism | `exact` | Why |
|---|---|---|
| `symbolics_to_intervals` | ✓ true | Identity on FormulaTerm; the same expression in two institutions' vocabulary. |
| `catalyst_to_diffeq` | ✗ false | Mass-action ODE compilation is faithful only to the deterministic limit; the stochastic structure of the master equation is lost. |
| `symbolics_to_jump` | ✗ false | The framing ("this expression is an objective to minimise subject to these bounds") imposes structure that's not in the source `SymbolicExpression`. |

The flag is consumed by audit code, not by the dispatcher. A downstream
query asking "is this Verdict's source provenance exact?" can walk the
comorphism chain back through `Trace::Comorphism` events and check each
edge's `exact` flag. An exact chain means the verdict is provably about the
original sentence; a non-exact chain means the verdict is about the
translation.

For now the consumption is mostly informational. The platform doesn't yet
prevent non-exact chains from feeding into Decidable predicates that gate
downstream commits — but that's the natural direction (a covenant
constraint shouldn't be discharged by a verdict that travelled through a
lossy translation without an explicit "I accept the loss" rider).

## 3.6. Static type-checking of the triple at commit time

When a `Comorphism` resource is committed, the kernel runs a structural
check before accepting the commit ([D14 §4.5](../../design/d14-institution-realisation.md)):

```
1. Resolve `export_format` → ExportFormat resource.
   - Read its `payload_type` IRI (e.g. urn:eigenius:formulas:FormulaTerm).
2. Resolve `import_format` → ImportFormat resource.
   - Read its `payload_type` IRI.
3. Resolve `transformation` → Component resource.
   - Read its `input_type` and `output_type` IRIs.
4. Assert:
   - input_type(transformation)  == payload_type(export_format)
   - output_type(transformation) == payload_type(import_format)
5. Assert:
   - capability_level(transformation) ∈ {Pure, Read}
   (v1 restriction; IO-capability transformations are rejected with
    `comorphism_io_not_supported_in_v1`)
```

If any assertion fails, the chain commit is rejected with a structural
validation error pointing at the misaligned property.

This is the kernel-side guarantee that comorphisms are *honestly*
type-aligned. A misaligned triple — say, an ExportFormat producing
`FormulaTerm` paired with a transformation expecting `string` — fails at
chain-commit time, not at the first dispatch. Locality of blame: the error
points at the comorphism declaration, not at the runtime path that would
have hit it.

## 3.7. Authoring a new comorphism

The minimum workflow for a new comorphism between two FormulaTerm-speaking
institutions (the cheap case):

1. **Confirm the source institution exposes an `ExportFormat`** with
   `payload_type: formulas:FormulaTerm` for the source class you want to
   bridge from. If not, author one (a small chain commit + a handler that
   extracts the FormulaTerm out of the source resource).
2. **Confirm the target institution exposes an `ImportFormat`** with
   `payload_type: formulas:FormulaTerm` for the target class. Same shape;
   author one if missing.
3. **Reuse the identity transformation Component**
   `urn:eigenius:comorphisms:m_id_formula_term` — already on every chain
   that has the formulas bootstrap layer.
4. **Commit a Comorphism resource** linking the three:

   ```json
   {
     "@id": "urn:eigenius:comorphisms:my_new_bridge",
     "core:is_a": ["institution:Comorphism"],
     "institution:export_format": "urn:eigenius:source:formats:ef_my_export",
     "institution:transformation": "urn:eigenius:comorphisms:m_id_formula_term",
     "institution:import_format": "urn:eigenius:target:formats:if_my_import",
     "institution:exact": true
   }
   ```

5. **Test by dispatching** through ESL (`comorphisms:my_new_bridge(input)`)
   or EigenQL (`FIBER ... AS ?out INTO "<iri>"`).

For a structural comorphism (the Catalyst → DiffEq case), step 3 is
non-trivial: you need to author the transformation Component, get the chain
to type-check it (signature alignment per §3.6), and ensure its
`capability_level` is `Pure` or `Read` (no IO, no mutating effects).

Whether identity or structural, the chain validates the triple at commit
time. A successful commit means the comorphism is structurally well-formed;
the runtime correctness of `extract_typed` and `reify` is each institution's
responsibility.

## 3.8. Failure modes

The most common ways comorphism dispatch fails:

| Symptom | Cause | Where to look |
|---|---|---|
| Commit-time `comorphism_payload_type_mismatch` | The transformation's input/output type doesn't match the export/import payload type. | Comorphism, ExportFormat, ImportFormat declarations — confirm `payload_type` and `transformation`'s `input_type`/`output_type` align. |
| Commit-time `comorphism_io_not_supported_in_v1` | The transformation Component requires `IO` capability. | The Component's `capability_level`. v1 restricts comorphism transformations to `Pure` or `Read` so they can be evaluated inline. |
| Commit-time `unknown_export_format` / `unknown_import_format` | The cited boundary resource doesn't exist on the chain (or is misspelled). | Run `eigenius inspect <iri>` to confirm. |
| Dispatch-time `extract_typed_failed` | The source institution's `extract_typed` handler errored — malformed input, missing required field, etc. | The handler's error message; the Verdict / RuntimeInvocation provenance. |
| Dispatch-time `reify_failed` | The target institution's `reify` handler errored — payload didn't satisfy the target class's invariants. | Same: handler logs + RuntimeInvocation. |
| Dispatch-time `comorphism_coercion_class_mismatch` (FIBER param coercion only) | The reified resource's class doesn't match the FIBER input slot's expected class. | The FIBER clause's param binding declaration. |

Chapter 8 covers cross-composition failure modes more broadly — how
validation cascades, how Verdicts can go stale, and how to read provenance
gaps under mixed hosting.

## 3.9. Note: theoretical foundations and a research direction

Institution theory was developed by Joseph Goguen and Rod Burstall in the
1980s as a model-theoretic formalism for *abstract logical systems* — a way
to talk about "a logic" without committing to any particular syntax,
semantic carrier, or proof system. The original framework lives squarely
within classical set theory and Tarskian model theory: an institution is
given by a category of signatures, a functor producing sentences over each
signature, a functor producing **models**, and a satisfaction relation
linking sentences to models. Comorphisms are the structure-preserving
translations between institutions in that classical setting.

Eigenius implements this framework in a *constructive* setting. The kernel's
small dependent-type theory (EigenTT) plays the role of the meta-language,
and an institution's typed `Verdict` becomes a chain-resident witness rather
than a model-theoretic satisfaction relation. The platform realises enough
of the structure to work in practice — declared comorphisms, type-checked
transformations, chain-resident verdicts, audit-traceable provenance —
without claiming to discharge the meta-theoretic equivalence between the
model-theoretic original and the type-theoretic realisation.

Closing that gap — formulating institution theory cleanly in *constructive
type theory*, with models replaced by **typed witnesses** under the
propositions-as-types reading (the Curry–Howard correspondence introduced
in [formula §2.2](../formula/02-mini-tt-fragment.md#22-why-pi-and-lam-are-chain-resident))
— is an open research direction. It is widely believed feasible: EigenTT's
`Pi`-types already correspond to universal quantification, its `Sigma`-types
to existential quantification, and the kernel already carries typed claims
and typed verdicts as data. What's missing is the meta-theoretic story
tying the constructive realisation back to the original model-theoretic
framework with full equivalence proofs. For the platform's purposes the
practical realisation is sufficient; for a type theorist who wants to
*prove* that what Eigenius implements is "really" institution theory, the
translation remains to be written.

## Cross-references

- [D14 §4.5](../../design/d14-institution-realisation.md) — comorphism
  type-check
- [D14 §9.3](../../design/d14-institution-realisation.md) — four-step
  pipeline + chain reinsertion
- [Formula guide §6.4](../formula/06-sharing-across-institutions.md#64-the-kinase-notebooks-three-comorphisms)
  — the three v1 comorphisms in summary form
- [`julia/comorphisms/`](../../../julia/comorphisms/) — the v1 comorphism
  declarations

---

Next: **[4. The three dispatch roles in concert →](04-dispatch-roles-in-concert.md)**
