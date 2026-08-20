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
                              │   (source institution's handler,
                              │    named by ExportFormat.procedure)
                              ▼
                  typed payload (EigenTT value)
                  at ExportFormat.payload_type
                              │
                              │  step 2: transformation term
                              │   (EigenTT evaluator)
                              ▼
                  typed payload (EigenTT value)
                  unchanged, for an identity middle
                              │
                              │  step 3: reify
                              │   (target institution's handler,
                              │    named by ImportFormat.procedure)
                              ▼
            target resource (e.g. OptimisationProblem)
                              │
                              │  step 4: chain reinsertion
                              │   (chapter 5)
                              ▼
              committed at urn:eigenius:comorphism-output:<tail>:<hex>
              (or at a caller-named IRI under FIBER ... INTO)
```

Walking through the kinase notebook's `symbolics_to_jump` comorphism. Note
that its payload type is **`jump:OptimisationProblem`**, not `FormulaTerm` —
the FormulaTerm rides inside the composite rather than being the payload:

1. **Extract.** The source institution is Symbolics; the source resource is
   a `SymbolicsToJuMPInput` carrying the SSE objective as a
   `SymbolicExpression` plus the JuMP-side framing (variable names, bounds,
   sense, constraints). The ExportFormat `ef_symb_to_jump_input` names the
   procedure `frame_as_optimisation_problem`, which assembles a fully-formed
   `jump:OptimisationProblem` — identity on the wrapped FormulaTerm, but a
   real packaging step around it. That is the typed payload.
2. **Transform.** The transformation is `m_id_optimisation_problem`, the
   identity `program:Lambda` on `jump:OptimisationProblem`. The evaluator
   reduces the application; the payload comes out unchanged.
3. **Reify.** The target institution is JuMP-HiGHS; the ImportFormat
   `if_jump_optimisation_problem` declares `to_class` and `payload_type`
   both as `jump:OptimisationProblem` and names the procedure
   `reify_problem`, whose body is `return problem`. The declarations say why
   a no-op procedure exists at all: it keeps the kernel's dispatch path
   uniform rather than special-casing an empty import.
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

When both endpoints of a comorphism agree on a payload type, the
`transformation` is **the identity function** on that type. It is declared
as a `program:Lambda` whose body is a `program:Var` naming its own binder:

```
m_id_formula_term : FormulaTerm → FormulaTerm
m_id_formula_term = λ t : FormulaTerm . t
```

The chain bytes flow through unchanged. This is the complete first resource
of
[`julia/comorphisms/symbolics-to-intervals.eigon.json`](../../../julia/comorphisms/symbolics-to-intervals.eigon.json),
with the `core:description` elided:

```json
{
  "@id": "urn:eigenius:comorphisms:symbolics_to_intervals:m_id_formula_term",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Lambda"],
  "urn:eigenius:core:short_name": "m_id_formula_term",
  "urn:eigenius:program:parameter": "t",
  "urn:eigenius:program:parameter_type": "urn:eigenius:formulas:FormulaTerm",
  "urn:eigenius:program:body": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
    "urn:eigenius:program:name": "t"
  }
}
```

Two things to read off this rather than assume. The resource is a
`program:Lambda`, not a `program:Component`: it carries `parameter` and
`parameter_type` and an embedded `Var` body, and it declares **no**
`capability_level`, `input_type` or `output_type` — a Lambda's type is read
off its binder and body. And the IRIs are written in full; the shorthand
`core:` / `program:` prefixes used elsewhere in this guide are a reading
convenience, not the on-disk form.

What identity transformations buy:

- **Zero runtime cost.** The EigenTT evaluator reduces `Lam(t. Var(t))`
  applied to `<payload>` to `<payload>` in one β-step. No work happens at
  the wire.
- **Trivial type-check.** Both `payload_type(export)` and
  `payload_type(import)` are the same IRI, and the middle is the identity at
  that type. There is nothing for a signature check to reconcile — though
  see §3.6 for how little the kernel actually checks today.
- **Audit-clarity.** The audit trail says exactly what happened: "Symbolics
  produced FormulaTerm `T`; the comorphism's identity transformation
  preserved `T`; IntervalArithmetic consumed FormulaTerm `T`." Bit-for-bit
  equivalence is honest and recordable.

**All three** of the kinase notebook's comorphisms declare an identity
middle — but at three different types, and only one of them is the identity
on `FormulaTerm`:

| Comorphism | Identity at | `exact` | Where the work went |
|---|---|---|---|
| `symbolics_to_intervals` | `formulas:FormulaTerm` | `true` | nowhere — export, middle and import are each one statement |
| `catalyst_to_diffeq` | `diffeq:OdeProblem` | `false` | Catalyst's `compile_to_ode` export procedure |
| `symbolics_to_jump` | `jump:OptimisationProblem` | `false` | Symbolics' `frame_as_optimisation_problem` export procedure |

The next section is the one that pays for a close reading: the case where the
declared middle is an identity and the translation is somewhere else
entirely.

## 3.4. Relocated transformations: Catalyst → DiffEq

Catalyst speaks `ReactionNetwork`; DiffEq speaks `OdeProblem`. Compiling a
network into an ODE right-hand side is structurally interesting — it's not
just renaming. So this is the case where you would expect the comorphism's
middle to do real work.

It doesn't. The shipped triple is

```
(ef_cat_to_ode_input, m_id_ode_problem, if_diffeq_problem)
```

where `m_id_ode_problem` is an identity `program:Lambda` on
`diffeq:OdeProblem` — the same shape as `m_id_formula_term` above, one type
up. The compilation lives in the **ExportFormat's procedure**,
`compile_to_ode`, on the Catalyst side. The declaration says so: "the actual
compilation work happens inside the ExportFormat's `procedure`".

So the pipeline for this comorphism is

```
CatalystToOdeInput
      │  (1) extract: Catalyst's compile_to_ode  ← the real transformation
      ▼
  OdeProblem  (FormulaTerm-typed RhsComponents)
      │  (2) transform: m_id_ode_problem         ← identity
      ▼
  OdeProblem
      │  (3) reify: DiffEq's reify_problem       ← `return problem`
      ▼
  chain-resident diffeq:OdeProblem
```

What `compile_to_ode` computes, conceptually:

```
compile_to_ode : CatalystToOdeInput → OdeProblem

compile_to_ode(input) = {
    state_names:        species_of(input.network),
    parameter_names:    parameters_of(input.network),
    rhs:                [rhs_component_for(s, input.network) | s ∈ species_of(input.network)],
    initial_conditions: input.initial_conditions,
    parameters:         input.parameter_values,
    time_span_start:    input.time_span_start,
    time_span_end:      input.time_span_end
}
```

In the shipped handler that is `Catalyst.netstoichmat(rn) *
Catalyst.oderatelaw.(reactions(rn))` for the symbolic right-hand side,
followed by a `Symbolics.Num`-to-`FormulaTerm` walk per component.

`rhs_component_for(species, network)` is the load-bearing piece: for each
species, walk the reaction network's rate equations and emit a `FormulaTerm`
for `dSpecies/dt`. The mass-action ODE for `A --k--> B` produces two
RhsComponents:

| Species | dSpecies/dt | FormulaTerm |
|---|---|---|
| A | `−k · A` | `App(App(OpRef(mul), App(App(OpRef(mul), LitFloat(-1)), Var(A))), Var(k))` |
| B | `k · A` | `App(App(OpRef(mul), Var(A)), Var(k))` |

The export procedure's *input* is a `CatalystToOdeInput` (a typed wrapper
around a `ReactionNetwork` plus initial conditions / parameters). Its
*output* is an `OdeProblem` with FormulaTerm-typed `RhsComponent`s. So the
translation is structural at the wire (different shapes), but the *pieces*
of the output are FormulaTerm-shaped — which means the DiffEq side of the
bridge consumes the result the same way it consumes any other `OdeProblem`.

Two consequences of putting the work here rather than in the middle. It is
attributed to Catalyst, which is where the reaction-network expertise lives,
and it shows up in the trace as a Catalyst runtime dispatch with its own
`RuntimeInvocation`. And the `OdeProblem` class it targets is declared by
**DiffEq**, not by Catalyst: Catalyst declares no FormulaTerm-typed property
of its own, so its route to the shared payload runs entirely through this
procedure's Julia code
([chapter 2 §2.2](02-shared-payload-languages.md#2-2-formulaterm-as-a-coordination-mechanism)).

Why this matters for the audit trail: a translation like this can lose
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
| `catalyst_to_diffeq` | ✗ false | Identity on `diffeq:OdeProblem`, but the Catalyst-side export's mass-action ODE compilation is faithful only to the deterministic limit; the stochastic structure of the master equation is lost. |
| `symbolics_to_jump` | ✗ false | Identity on `jump:OptimisationProblem`, but the Symbolics-side export's framing ("this expression is an objective to minimise subject to these bounds") imposes structure that's not in the source `SymbolicExpression`. |

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

## 3.6. What the kernel checks about the triple at commit time

When a `Comorphism` resource is committed, validation Rule 15
([`kernel/src/validation/mod.rs`](../../../kernel/src/validation/mod.rs),
`check_comorphism_well_formedness`) runs a **resolution and class** check —
not a signature check:

```
1. Resolve `export_format`.
   - It must resolve in the layer chain, AND
   - resolve to an instance of institution:ExportFormat.
2. Resolve `import_format`.
   - Same two conditions, against institution:ImportFormat.
3. Resolve `transformation`.
   - It must resolve to *some* resource in the layer chain.
     Its class is not checked; its type is not read.
```

All three failures are reported as
`ValidationRule::UnresolvedClassReference` on the offending property, with a
message of the form `Comorphism.export_format: '<iri>' does not resolve to
any resource in the layer chain` or `… resolves to a resource that is not an
instance of ExportFormat`. There are no dedicated error codes for
comorphism-shape failures; in particular the strings
`comorphism_payload_type_mismatch`, `unknown_export_format` and
`unknown_import_format` appear nowhere in the kernel.

**What is *not* checked at commit time**, despite being the natural
expectation:

- `payload_type(export_format)` against the transformation's input type.
- The transformation's output type against `payload_type(import_format)`.
- The transformation's `capability_level`.

The rule's own doc comment records why: the full EigenTT signature-equality
check between the referenced term and the export/import payload types is
deferred to M5 of the implementation plan. **A comorphism whose
transformation has the wrong signature commits cleanly and fails at first
dispatch**, so locality of blame is weaker here than the rest of this
chapter's error taxonomy suggests. Rule 15 does close the failure that
would otherwise be silent: `check_class_types` deliberately skips *missing*
references on instance properties (they may be forward references filled
later in the same batch), and for a Comorphism the two formats must already
exist when it enters the chain.

The capability restriction is real but lives elsewhere. `comorphism_io_not_supported_in_v1`
is an **EigenQL type-check** error raised on the FIBER param-coercion path
([`kernel/src/query/type_check.rs`](../../../kernel/src/query/type_check.rs)),
not a commit rule: it fires when a query cites a comorphism whose
transformation resource carries
`capability_level = urn:eigenius:program:capability_levels:io`. A comorphism
with an IO transformation that is never cited from a FIBER clause is never
rejected.

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
3. **Declare an identity `program:Lambda` at the shared payload type.**
   There is no global one to reuse: each shipped comorphism file declares its
   own, alongside the `Comorphism` that names it (e.g.
   `urn:eigenius:comorphisms:symbolics_to_intervals:m_id_formula_term`). The
   resource is six lines; copy the shape from
   [`symbolics-to-intervals.eigon.json`](../../../julia/comorphisms/symbolics-to-intervals.eigon.json)
   and give it an IRI under your own comorphism.
4. **Commit a Comorphism resource** linking the three:

   ```json
   {
     "@id": "urn:eigenius:comorphisms:my_new_bridge",
     "core:is_a": ["institution:Comorphism"],
     "institution:export_format": "urn:eigenius:source:formats:ef_my_export",
     "institution:transformation": "urn:eigenius:comorphisms:my_new_bridge:m_id_formula_term",
     "institution:import_format": "urn:eigenius:target:formats:if_my_import",
     "institution:exact": true
   }
   ```

5. **Test by dispatching** through ESL (`comorphisms:my_new_bridge(input)`)
   or EigenQL (`FIBER ... AS ?out INTO "<iri>"`).

For a comorphism whose endpoints do *not* already agree on a payload type
(the Catalyst → DiffEq case), you have two options and the shipped code takes
the second: author a transformation term that does the work, or put the work
in the source institution's `ExportFormat` procedure and keep the middle an
identity at the composite type. If you author a real transformation, keep its
`capability_level` `Pure` or `Read` — the FIBER coercion path rejects `IO`
(§3.6) — and be aware that nothing checks its signature at commit time.

The chain validates the triple's *references* at commit time (§3.6). A
successful commit means the cited formats exist and are the right classes; it
does **not** mean the transformation's type lines up, and it says nothing
about the runtime correctness of `extract_typed` and `reify`, which is each
institution's responsibility.

## 3.8. Failure modes

The most common ways comorphism dispatch fails:

Diagnostics below are quoted as the kernel emits them. Only two of the rows
carry a stable machine-readable code; the rest are plain messages.

| Symptom | Cause | Where to look |
|---|---|---|
| Commit-time `Comorphism.export_format: '<iri>' does not resolve …` / `… is not an instance of ExportFormat` (also for `import_format`, `transformation`) | The cited boundary resource doesn't exist on the chain, is misspelled, or is the wrong class. Rule 15. | Run `eigenius inspect <iri>` to confirm. |
| Type-check `comorphism_io_not_supported_in_v1` | A FIBER param coercion cites a comorphism whose transformation carries `capability_level: …:capability_levels:io`. | The transformation's `capability_level`. v1 restricts FIBER-coerced transformations to `Pure` or `Read` so they can be evaluated inline. |
| Type-check `comorphism_target_class_mismatch` | The comorphism's import-side `to_class` isn't among the FIBER param's declared `class_types`. | The FIBER clause's param binding declaration. |
| Evaluation-time `comorphism '<iri>' not registered in InstitutionIndex` | The comorphism isn't indexed on the layer being queried. | Confirm the declaration committed; check the startup `institution_register` logs. |
| Evaluation-time `comorphism '<iri>': source institution '<iri>' not registered in runtime` (and the `target institution` variant) | The declaration indexed but the institution's runtime isn't reachable. | The institution's `runtime` value and its `RuntimeEnvironment`. |
| Dispatch-time handler error, surfaced verbatim from the institution | The source `extract_typed` or the target `reify` errored — malformed input, missing required field, payload violating a target-class invariant. | The handler's error message; the Verdict / RuntimeInvocation provenance. |
| First-dispatch type error on a comorphism that committed cleanly | The transformation's signature never matched the payload types; Rule 15 does not check this (§3.6). | Compare `payload_type` on both formats against the transformation's binder type. |

Signature misalignment has no commit-time symptom at all — it presents as
the last row.

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
