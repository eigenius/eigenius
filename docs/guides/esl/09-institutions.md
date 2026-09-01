# 9. Institutions in ESL

An **institution** is a domain-specific reasoning system registered with the kernel. Under D14 ([Institution Realisation](../../design/d14-institution-realisation.md), supersedes D10) institutions are declared as ontology resources committed to the layer chain — `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, and `Comorphism` — plus a runtime that handles the boundary translations and any opaque reasoning. There is no `FiberDeclaration` struct any more: the chain *is* the declaration.

This chapter covers the institution surface from a program-author's perspective in ESL: how qualified-name function calls dispatch to a `Decidable` `QueryClass`, how the resulting `Verdict` flows through EigenTT, and the worked life-science example that drove the design.

The implementer view (writing a new institution as a WASM binary) lives in [platform §10](../platform/10-wasm-institutions.md). The query-language view of the same surface is [EigenQL §8](../eigenql/09-institutions.md). The protocol spec is [D14](../../design/d14-institution-realisation.md).

## 9.1. What an institution declares

Five resource shapes carry the institution surface (D14 §4). All of them are ordinary typed resources: an ESL author can read and write them with `class`, `property`, and `resource` declarations, but in practice they are usually shipped alongside the institution's WASM binary or substrate handler package as Eigon-JSON. The `Institution` resource's `runtime` property selects the host kind: `wasm` for sandboxed WASM institutions ([platform §10](../platform/10-wasm-institutions.md)), `external` for substrate-hosted institutions running in sibling containers ([platform §11](../platform/11-runtime-substrate.md), Julia in v1), or `in_process` for kernel-embedded Rust institutions. From ESL's perspective there's no difference between the three — the same compile-time classifier and runtime dispatch path applies, and the same `Institution::query` / `extract_typed` / `reify` trait surface answers calls.

| Shape | Carries |
|---|---|
| `Institution` | `institution_iri`, `name`, `runtime` (`wasm` / `external` / `in_process`). |
| `ExportFormat` | `from_class`, `payload_type`, `institution_ref`, `procedure`. |
| `ImportFormat` | `to_class`, `payload_type`, `institution_ref`, `procedure`. |
| `QueryClass` | `query_class` (input), `result_class`, `dispatch_role` ⊆ `{OnDemand, AutoOnLoad, Decidable}`, `query_handler`, `institution_ref`. |
| `Comorphism` | `export_format`, `transformation` (a EigenTT Component), `import_format`, `exact: bool`. |

These are ordinary Eigon resources, so an ESL author can read and reason about them through the layer. (`eigenius-wasm-sdk::institution` used to offer `InstitutionDecl` / `ExportFormatDecl` / `ImportFormatDecl` / `QueryClassDecl` / `ComorphismDecl` builders that constructed them programmatically; the SDK went with the WASM extension path on `2026-07-08`.)

The kernel's [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) is a derived index built by scanning the chain (D14 §3); ESL's compile-time classifier uses it. There is no separate `register()` call any more — committing the declarations to the chain *is* registration.

## 9.2. Classification at compile time

When the ESL compiler encounters a function-call expression with a qualified name, it asks the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) to classify the IRI. The classification lives in [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs); it returns one of:

| Classification | ESL emits |
|---|---|
| `Decidable` `QueryClass` | `Exp::NativeDecide(Constraint::Institution { iri, args }, Unit)` — a EigenTT term that reduces a `Verdict` to either `Refl(v)` (Holds), a failing neutral (Fails), or stays as a passthrough neutral (Undecidable). |
| Otherwise | Falls through to component dispatch / class constructor / unbound — same as today. If nothing matches, the call fails at compile or evaluate time with `unknown function`. |

The classifier recognises a second institution shape as well:

| Classification | ESL emits |
|---|---|
| `Comorphism` | `program:ComorphismInvokeApply`, which `program/expr.rs` decodes to `Exp::InstitutionInvoke { comorphism_iri, source, target_iri }` (D14 §9.3). Exactly one source argument, and no configuration block, or the compile fails. See [§9.5](#95-invoking-comorphisms-from-esl-programs). |

One shape stays out of expression position:

- **No `OnDemand` `QueryClass` from expression position.** OnDemand QueryClasses are reachable only from EigenQL `FIBER` clauses. ESL programs that need a structured response from an institution embed a FIBER call into an EigenQL query (and feed the bound response back as a EigenTT value if needed).

The classifier is consulted only by `compile_with_institutions`. The plain `compile` entry point still works for programs that don't reference institution-dispatched IRIs:

```rust
let resources = esl::compile_with_institutions(source, Arc::clone(&institution_index))?;
```

This is the same single-source-of-truth classification used by EigenQL ([EigenQL §8](../eigenql/09-institutions.md)) — both surface languages share `InstitutionIndex`, so the same IRI dispatches identically.

## 9.3. Invoking a Decidable QueryClass

```esl
namespace cap = "urn:eigenius:test";

program ex:check_program : ex:Thing -> ex:Thing {
    let v : urn:eigenius:institution:Verdict = cap:within_tolerance(input.ex:delta, 0.1);
    input
}
```

The qualified name `cap:within_tolerance` resolves to `urn:eigenius:test:within_tolerance`. If the index classifies this as a Decidable `QueryClass`, the compiled body becomes:

```
Lam(input,
    Let(v,
        NativeDecide(
            Constraint::Institution {
                iri: "urn:eigenius:test:within_tolerance",
                args: [input.ex:delta, 0.1]
            },
            Unit
        ),
        input))
```

**At type-check:** the kernel resolves the constraint IRI in the `InstitutionIndex`, reads the QueryClass's `institution_ref` and `query_handler`, and calls [`Institution::query`](../../../kernel/src/institution/runtime.rs) on the registered runtime. The institution returns a `Verdict` resource; the kernel reads its `ctor_name` property and reduces:

| `ctor_name` | Kernel behaviour |
|---|---|
| `Holds` | Reduces `NativeDecide` to `Refl(Unit)` — predicate accepted at check time. |
| `Fails` | Emits a failing neutral — type-check rejects the program with the institution's diagnostic. |
| `Undecidable` | Stays as a passthrough neutral — the predicate is left for runtime. |

**At runtime:** same dispatch path, through an IO-tier engine. An `Undecidable` from check time becomes a runtime call with the same outcome semantics.

**Synthetic input shape.** The kernel constructs a synthetic input resource of the QueryClass's declared `query_class`. Positional arguments are attached as the property `urn:eigenius:institution:decide_args` (an array). Institutions whose handlers expect named-property inputs (typical when the QC is also FIBER-callable) read the args off named properties; institutions whose handlers expect positional args read `decide_args`. The dock-assay demo showed both shapes side by side in `examples/wasm-d14-assay`, deleted with the WASM path; [`kernel/tests/dock_assay_demo.rs`](../../../kernel/tests/dock_assay_demo.rs) is the surviving in-process version.

**Default behaviour.** Institutions whose runtime returns `NotImplemented` for a procedure surface as a runtime evaluation error at dispatch time. There is no longer a "silent Undecidable" fallback (the D10 behaviour) — under D14, `Undecidable` is a *value the institution returns*, not a default the kernel substitutes when reasoning is unavailable.

## 9.4. Invoking an OnDemand QueryClass — through EigenQL

OnDemand QueryClasses are not directly callable from ESL expressions. They are reached only through an EigenQL `FIBER` clause. ESL programs that need a structured response from an institution typically embed the FIBER call into an EigenQL query and feed the bound response back as a EigenTT value.

This is a deliberate scope decision. Decidable handles the common cap-as-predicate case from program bodies; the FIBER form handles the multi-property-response case from EigenQL. Putting OnDemand calls back into ESL expression position would mean two return shapes (Verdict vs. arbitrary resource) and two dispatch paths in the compiler — the FIBER form covers that surface unambiguously.

## 9.5. Invoking comorphisms from ESL programs

Under D14, a `Comorphism` is a typed resource:

```
Comorphism dock_to_assay
  export_format: ef_dock_to_dg          // DockingResult → Float (dock institution)
  transformation: cm_arrhenius          // Float → Float EigenTT Component
  import_format: if_assay_from_ic50     // Float → AssayPrediction (assay institution)
  exact: false
```

The kernel statically type-checks at commit time that the transformation Component's signature matches `(payload_type(export_format)) → (payload_type(import_format))` (D14 §4.5). A comorphism with mismatched types is rejected.

A program body invokes a comorphism as a **qualified-name function call in expression position**. The compiler classifier resolves the qualified name through the `InstitutionIndex`; if it classifies as a `Comorphism`, the call lowers to `Exp::InstitutionInvoke { comorphism_iri, source, target_iri: None }` (D14 §9.3).

Example from the kinase-institutions notebook ([cell 13](../../../notebooks/examples/kinase-institutions.json)):

```esl
namespace symbolics   = "urn:eigenius:symbolics";
namespace jump        = "urn:eigenius:jump";
namespace comorphisms = "urn:eigenius:comorphisms";

program nb:produce_problem
    : symbolics:SymbolicsToJuMPInput -> jump:OptimisationProblem
{
    comorphisms:symbolics_to_jump(input)
}
```

The wrapper program takes a `SymbolicsToJuMPInput` (carrying a typed objective expression as `FormulaTerm`) and produces a JuMP `OptimisationProblem`. Running the program runs the four-step D14 §9.3 pipeline:

1. The kernel resolves the `Comorphism` resource and reads `export_format` / `transformation` / `import_format`.
2. **Extract.** Calls the source institution's `Institution::extract_typed(export_format.procedure, input, ctx)` → typed payload of the export's `payload_type`.
3. **Transform.** Applies the transformation Component to the payload via the EigenTT evaluator → typed payload of the import's `payload_type`.
4. **Reify.** Calls the target institution's `Institution::reify(import_format.procedure, payload, ctx)` → the target-class resource.

**Chain reinsertion.** The reify output commits to the chain at a deterministic content-hash IRI of the form `urn:eigenius:comorphism-output:<comorphism-tail>:<hex16>` (SHA-256 over the canonical Eigon-CBOR of the produced resource, with `@id` cleared). Re-running the same input dedupes to the same IRI — the cross-fibre identity property the Grothendieck construction wants.

**Audit.** The program trace gets a `Trace::Comorphism { comorphism_iri, source_trace, target_iri, target_class }` audit variant naming both endpoints. The reify output's commit goes through the same `commit_with_validation` machinery as any chain entry, so AutoOnLoad gates bound to the produced class fire on the resulting resource exactly as if it had been authored by hand.

**Two alternative shapes** — useful when the program already has the payload, or when chain reinsertion isn't wanted:

1. **Run the constituent Component directly.** The transformation is a regular EigenTT Component (`cm_arrhenius` in the example). If your ESL program already has the typed payload, it can apply the Component without the extract/reify round trip:
   ```esl
   let ic50 : core:float = cm_arrhenius(delta_g);
   ```
2. **Translate inside an EigenQL query.** Use a `FIBER` clause whose param value is `comorphism(source)` (see [EigenQL §8.6](../eigenql/09-institutions.md#86-comorphism-coercion-in-fiber-params)). The query runs the four-step pipeline and the resulting reified resource flows into the FIBER's input — without chain reinsertion unless `INTO "<iri>"` is added (see [EigenQL §7.6](../eigenql/08-fiber-clauses.md#76-into--pinning-the-response-iri)).

## 9.6. Constraints attached to properties

A property declaration can carry a Decidable QueryClass IRI as a constraint:

```esl
property ex:rmsd : core:float {
    description = "Root-mean-square deviation in angstroms";
    min_value = 0.0;
    // Institution-decided constraints are attached via the property's
    // metadata in the underlying resource graph; the surface for this
    // is informal as of Phase 12 (see compile.rs for the current
    // shape).
}
```

When a value flows through such a property position, the kernel iterates the constraints during type-check (Check mode) and dispatches each as a `NativeDecide`. A `Holds` accepts the value; a `Fails` rejects the type-check with the institution's diagnostic; an `Undecidable` defers the constraint to runtime.

This is the mechanism that makes domain reasoning *automatic* — the program author writes a normal ESL program, and the kernel verifies that values flowing through property positions satisfy whatever the institutions registered for those positions. There's no `assert` in the program text.

## 9.7. Worked example: docking + assay

The M8 worked example ships with the kernel ([`ontologies/examples/d14-dock-assay/dock-assay.json`](../../../ontologies/examples/dock-assay/dock-assay.json), tested in [`kernel/tests/d14_dock_assay_demo.rs`](../../../kernel/tests/dock_assay_demo.rs)). Two institutions:

- `dock` — owns the `DockingResult` class. Declares an `ExportFormat` (`ef_dock_to_dg`) extracting `delta_g` as a `Float`.
- `assay` — owns the `AssayPrediction` class. Declares an `ImportFormat` (`if_assay_from_ic50`) reifying a `Float` as `ic50`. Also declares three `QueryClass`es: `within_tolerance` (Decidable, three-arg predicate), `assay_prediction_validity` (AutoOnLoad, validates positive IC₅₀ on Load), and `validate_prediction` (OnDemand, FIBER-callable).

A `Comorphism` (`dock_to_assay`) ties them together via a EigenTT Component (`cm_arrhenius`, the Arrhenius approximation `IC₅₀ ≈ exp(-ΔG/RT)`).

An ESL program over this surface:

```esl
namespace core = "urn:eigenius:core";
namespace dock = "urn:eigenius:demo:d14";
namespace cap  = "urn:eigenius:demo:d14";

program dock:filter_acceptable
    : dock:DockingResult -> dock:DockingResult
{
    // Decidable QueryClass: kernel dispatches to assay institution,
    // reduces to Refl when verdict is Holds.
    let ok : urn:eigenius:institution:Verdict =
        cap:within_tolerance(
            input.dock:delta_g,           // predicted-IC50 source
            input.dock:delta_g_target,    // target
            0.1                           // tolerance
        );
    input
}
```

The first call invokes a Decidable QueryClass — at check time the kernel asks the assay institution whether the predicted-vs-target ΔG is within tolerance. The Verdict gates the type-check; if the answer is Holds the program's body type-checks with `Refl(Unit)` standing in for the predicate; if Fails, the type-check rejects the program; if Undecidable, the constraint is deferred to runtime.

To compile this program against the demo ontology:

```rust
let layer = ...;                                              // base + demo ontology
let (idx, _) = InstitutionIndex::from_layer(&layer);
let resources = esl::compile_with_institutions(source, Arc::new(idx))?;
```

To run it, the caller threads the `InstitutionRuntime` (with a runtime-registered or auto-registered `WasmInstitution`) into an `InstitutionEngine` and hands that to an effectful `EvalCtx` ([chapter 8](08-capability-modes.md)):

```rust
let engine = InstitutionEngine::for_io(
    Arc::clone(&layer),
    components,                              // Arc<ComponentRegistry>
    None,                                    // trace store
    Arc::new(Mutex::new(Vec::new())),        // dispatched traces
    Arc::new(Mutex::new(Vec::new())),        // produced resources
    None,                                    // task context
    Some(Arc::clone(&index)),
    Some(Arc::clone(&inst_runtime)),
);
let runtime_ctx = EvalCtx::effectful(Some(Arc::clone(&layer)), Arc::new(engine));
```

For a check-time context — institution constraints fire, components do not dispatch — use `InstitutionEngine::for_check(layer, index, runtime)` instead.

There was a WASM-hosted variant (`d14_dock_assay_demo_wasm.rs`) constructing the same surface with `WasmInstitution` instances auto-registered from a child layer carrying `runtime: wasm` + inline `wasm_binary` declarations. It went with the WASM extension path on `2026-07-08`; [`kernel/tests/dock_assay_demo.rs`](../../../kernel/tests/dock_assay_demo.rs) remains.

See [`docs/design/life-science-requirements.md`](../../design/life-science-requirements.md) for the original motivating discussion.

## 9.8. The classification table (cross-language)

The same classification mechanism powers ESL and EigenQL. When the same IRI appears in both languages, it dispatches identically:

| Index lookup | ESL emits | EigenQL emits | Runtime call |
|---|---|---|---|
| `Decidable` QueryClass | `Exp::NativeDecide(Constraint::Institution { … }, Unit)` | Same — projected to Boolean by postfix `HOLDS`/`FAILS`/`UNDECIDABLE` | `Institution::query(query_handler, synthetic_input, ctx)` |
| `OnDemand` QueryClass | not exposed in ESL expression position | `FIBER` clause | `Institution::query(query_handler, input, ctx)` |
| `Comorphism` | qualified-name function call in expression position — `comorphisms:foo(input)` — lowers to `Exp::InstitutionInvoke` (D14 §9.3); reify output commits at `urn:eigenius:comorphism-output:…` | FIBER param coercion (overlay-only); or `FIBER ... INTO "<iri>"` for chain reinsertion at a caller-named IRI | `extract_typed → transformation → reify` four-step pipeline; reify output commits to the chain |
| Class / primitive / literal | `Exp::EigonClass(iri)` etc. | resolved via layer | various |

Cross-link: [EigenQL chapter 8](../eigenql/09-institutions.md) covers the same table from the query-language side.

## 9.9. Where the implementation lives

| File | Purpose |
|---|---|
| [`kernel/src/institution/runtime.rs`](../../../kernel/src/institution/runtime.rs) | `Institution` trait (D14 §8), `InstitutionRuntime` |
| [`kernel/src/institution/registry.rs`](../../../kernel/src/institution/registry.rs) | `InstitutionIndex` (derived from chain scan) |
| [`kernel/src/institution/dispatch.rs`](../../../kernel/src/institution/dispatch.rs) | Auto-on-Load dispatch |
| [`kernel/src/institution/error.rs`](../../../kernel/src/institution/error.rs) | `InstitutionError` |
| [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs) | Compile-time classification of `Apply` expressions through `InstitutionIndex` |
| [`kernel/src/nbe/check/mod.rs`](../../../kernel/src/nbe/check/mod.rs) | `NativeDecide` check arm — fires Decidable QueryClasses at type-check time |
| [`kernel/src/nbe/eval/mod.rs`](../../../kernel/src/nbe/eval/mod.rs) | `decide_constraint` evaluation |
| [`kernel/src/capability/registration.rs`](../../../kernel/src/capability/registration.rs) | chain scan → backends: `register_in_process_institutions` (Rust, registered at startup) and `register_external_institutions` (gRPC to the orchestrator) |

The WASM row this table used to carry — `wasm_institution_d14.rs`, the host bridge to the
`eigenius-institution-d14` WIT world — named a file deleted with the WASM extension path on
`2026-07-08`. `build_wasm_institution_runtime` does not exist either. [Chapters 9](../platform/09-wasm-components.md)
and [10](../platform/10-wasm-institutions.md) of the platform guide are retained as a record of that path.

## 9.10. The justification vocabulary — D39 Justification Logic

Chain authors commit **conclusions**: a `justification:Conclusion` carrying one judgement, `holds(kernel, c, Certificate(j, P))`, read as *the kernel verified that certificate `c` grounds a claim to `P`*. It is the vocabulary that turns Eigenius's epistemic grounds into composable evidence inside the type theory — distinct evidence chains for the same proposition produce judgmentally-equal certificates ([§7.1 proof irrelevance](07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom)), and the audit trail from a checked conclusion to the chain artifacts that admitted its witnesses cannot be broken, because the [D49 chain-witness](../../design/d49-chainwitness-machinery.md) admission mechanism is the only path to the grounding constructors.

**This is not an institution, and it stopped being one.** A Reasoning institution used to own the check and dispatch it as an AutoOnLoad QueryClass. Checking a certificate is type checking, which the kernel does not delegate, so it moved into ordinary commit-time validation and the institution — having nothing else to host — was deleted along with its ExportFormat, its four QueryClasses, and the `urn:eigenius:reasoning` namespace entirely. What remains is vocabulary the kernel checks directly, which is what §9.11 means by a logic supplying vocabulary rather than authority.

Design: [D39](../../design/d39-justification-logic.md), superseded by [D73](../../design/d73-justification-logic-witnesses-and-traces.md) and the paper; ontology: [`ontologies/justification/justification.esl`](../../../ontologies/justification/justification.esl); the check: [`kernel/src/validation/rules/eigentt_value.rs`](../../../kernel/src/validation/rules/eigentt_value.rs).

### 9.10.1. The five `justification:Term` constructors

`justification:Term` is a closed (non-indexed) inductive — five constructors, no recursive references, not extensible without a versioned ontology change:

```esl
data justification:Term {
    Declared(core:string),                        // asserted by an accountable agent, by IRI
    Observed(core:string),                        // read off the world, by IRI
    Verified(core:string),                        // proof-checked, by IRI
    App(justification:Term, justification:Term),  // Artemov Application — modus ponens
    Sum(justification:Term, justification:Term),  // Artemov Sum — choice of evidence
}
```

Three grounds and the paper's two composition operators — *"an algebra of justification terms that supports application and sum."*

- **`Declared(iri)`** — cite an `axiom` declaration ([§4.4a](04-declarations.md#4-4a-axiom-postulated-propositions-d46-10)) or any other `justification:Claim` (a literature rule, a statistical-to-domain bridge, a claim that a plan denotes a function of its input). The chain attests *that an accountable agent asserted it*, not that it has been independently verified.
- **`Observed(iri)`** — cite a resource read off the world (bench measurement, instrument log, released dataset). The chain attests *that it was observed, by the activity its `prov:ObservationTrace` names* — not what its semantic interpretation is.
- **`Verified(iri)`** — cite a conclusion carrying a `justification:proof`, the judgement `holds(logic, t, P)`. The chain attests *that a checker verified `t` against `P` itself*.
- **`App(j1, j2)`** — Artemov Application: if `j1` justifies `A -> B` and `j2` justifies `A`, then `App(j1, j2)` justifies `B`.
- **`Sum(j1, j2)`** — Artemov Sum: two independent grounds for the same conclusion. `sum_l` / `sum_r` record which was preferred, and **both require a derivation for each branch** (see below).

**There is no `Derived` ground, and that is the design's central claim in miniature.** A computed claim does not rest on the fact that a computation ran. It rests on two things a run cannot supply: the assertion that the plan denotes a function `I -> O` — which an accountable agent makes, and which no execution establishes, because determinism is a fact about the environment rather than something recoverable from a run record — and the inputs it was applied to. So a computed ground is the APPLICATION `App(Declared(plan), Observed(inputs))`, and `Sampled` is a bare `Observed` leaf. Both are *term shapes*, not fundamental grounds.

**There is no `SpecStr` either.** It was a third operation the paper's algebra does not have, and its second field was the only unchecked argument anywhere in the algebra: `spec_poly` bound the instance `x : T` and the tag independently, with nothing relating them, so the tag was a free string the author picked and no rule validated. The RULE survives (below); only the term record went, and specialization now leaves the term unchanged — which is right, because eliminating a quantifier narrows the proposition and introduces no ground.

### 9.10.2. The `justification:Certificate` certificate predicate

`justification:Certificate : justification:Term -> Prop -> Type 2` is an [indexed inductive family](04-declarations.md#indexed-d48-indexed-families): its two indices are the term and the proposition. It lives in a `Type`, not `Prop`, so certificates are stored and re-checkable.

Seven constructors: three groundings, `app`, the two `Sum` arms, and `spec_poly`:

```esl
data justification:Certificate : justification:Term -> Prop -> Type 2 {
    declared : forall (iri, P) => witness:IsDeclaredAs(iri, P) -> justification:Certificate(Declared(iri), P),
    observed : forall (iri, P) => witness:IsObservedAs(iri, P) -> justification:Certificate(Observed(iri), P),
    verified : forall (iri, P) => witness:IsVerifiedAs(iri, P) -> justification:Certificate(Verified(iri), P),

    app : forall (A, B, j1, j2) =>
        justification:Certificate(j1, A -> B) -> justification:Certificate(j2, A) -> justification:Certificate(App(j1, j2), B),

    // BOTH branches must be justified — see below.
    sum_l : forall (P, j1, j2) =>
        justification:Certificate(j1, P) -> justification:Certificate(j2, P) -> justification:Certificate(Sum(j1, j2), P),
    sum_r : forall (P, j1, j2) =>
        justification:Certificate(j1, P) -> justification:Certificate(j2, P) -> justification:Certificate(Sum(j1, j2), P),

    spec_poly : forall (T : Type 1, P : T -> Prop, j, x : T) =>
        justification:Certificate(j, forall (y : T) => P(y)) ->
        justification:Certificate(j, P(x)),
}
```

The three grounding constructors each consume a [`ChainWitness.Is*As`](06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state) — a witness the kernel admits at type-check time from the layer's witness index. The author never writes it; the kernel synthesizes it from the cited IRI and proposition. If no admitted witness matches the (category, iri, proposition) triple, type-checking fails with a diagnostic naming the missing trace shape.

**`sum_l` / `sum_r` depart from LP's axiom deliberately.** Artemov's `t:F -> (t+s):F` quantifies over an arbitrary `s`, so the unused summand need not be justified or even name a resource that exists. That is unsound here, because `support` reads `Sum` disjunctively and reports the unchecked branch as a genuine alternative: `Sum(real_evidence, Declared("urn:does-not-exist"))` type-checked, and `survives_without(real_evidence)` then returned **true** — the conclusion "survived" losing its only ground by way of a branch nothing ever grounded. Requiring both branches makes the term and the certificate agree about `Sum`. Asserting a fallback obliges you to show the fallback works.

**`spec_poly` leaves the term index at `j`.** Specialization narrows the PROPOSITION and introduces no ground, so the term that certified the universal certifies the instance. One consequence, stated rather than discovered later: `spec_poly` and `declared` can both target `Certificate(Declared(rule), P(x))`, so the TERM stops determining which rule applies at that node. Checking is unaffected because a certificate names its own constructor.

**No implication introduction.** No constructor produces `justification:Certificate(_, A -> B)` — `app` yields `B`, `sum_l` / `sum_r` yield `P`, `spec_poly` yields `P` at an instance. An implication therefore enters only through a grounding: asserted as a resource, witnessed by a trace. There is no deduction theorem here, so a rule relating propositions cannot be *derived*; it must be Declared, and quantifying it and eliminating with `spec_poly` is what lets one rule serve many instances.

### 9.10.3. The `justification:Conclusion` resource

The chain-resident reasoning step. It carries **one** required slot, and that is the point.

```esl
class justification:Conclusion {
    requires justification:judgement;
    recommends justification:proof,
               justification:subject_iri,
               justification:refutes;
}
```

Property shapes:

- **`justification:judgement`** — an `eigentt:Judgement`: `holds(kernel, c, Certificate(j, P))`, read as *the kernel verified that certificate `c` grounds a claim to `P`*. It does **not** assert `P`: a certificate records grounds, and no rule turns `Certificate(j, P)` into `P`.

  This replaces the three slots the class used to carry — `proposition`, `term`, `certificate` — which were checked by three separate paths with nothing requiring them to be about the same claim. A certificate for one proposition could sit beside a different proposition and both checked clean. Folding them into the judgement's TYPE is what makes the pairing the thing that gets checked.
- **`justification:proof`** (recommended) — a second judgement, `holds(logic, t, P)`: a checker verified `t` against `P` itself. This is factive, and **only this admits an `IsVerifiedAs` witness**. A conclusion carrying no proof term is not citable as `Verified(iri)`, however well justified it is.
- **`justification:subject_iri`** (recommended) — the principal Resource this conclusion is about. A first-class EigenQL index for "what have I concluded about X?".
- **`justification:refutes`** (recommended) — IRI of a prior conclusion this one supersedes.

It subclasses nothing. It used to subclass `reflection:DerivedResource`, which made a later conclusion's `DerivedEvidence(prior_iri)` citation resolve — so citing an earlier conclusion by IRI grounded the citing one. Both halves are gone: the grade classes are deleted, and citation by IRI for an unproved conclusion was the laundering step. A synthesis composes with `Certificate.app` over the cited conclusion's certificate instead, which is what `app` was for.

### 9.10.4. Worked example — composing two evidence chains via `App`

The agent claims `StrongInhibitor(EIG_0291)`. The justification applies a literature rule (`HasLowIC50(EIG_0291) -> StrongInhibitor(EIG_0291)`) to a computed statistical claim. The computed half is itself an `App`: the plan DECLARED to denote a function of its input, applied to the OBSERVED input.

```esl
resource screen:concl_eig0291_strong : justification:Conclusion {
    justification:subject_iri = "urn:eigenius:demo:screen:EIG_0291";

    justification:judgement = type_expr(
        alias
            EIG  = "urn:eigenius:demo:screen:EIG_0291",
            SS   = "urn:eigenius:demo:screen:m_eig0291_sampleset",
            PLAN = "urn:eigenius:demo:screen:plan_yields_lowic50",
            RULE = "urn:eigenius:demo:screen:rule_strong",
            LOW  = screen:HasLowIC50(EIG),
            // the COMPUTED ground — not a leaf citing the run's output
            computed = justification:App(Declared(PLAN), Observed(SS)),
            cs = app( core:Asserts(SS), LOW,
                      Declared(PLAN), Observed(SS),
                      declared(PLAN, core:Asserts(SS) -> LOW),
                      observed(SS, core:Asserts(SS)) )
        in
        holds( eigentt:logic_kernel,
               app( LOW, screen:StrongInhibitor(EIG),
                    Declared(RULE), computed,
                    declared(RULE, LOW -> screen:StrongInhibitor(EIG)),
                    cs ),
               justification:Certificate(
                   justification:App(Declared(RULE), computed),
                   screen:StrongInhibitor(EIG) ) )
    );
}
```

At commit, the `ValidateJustification` AutoOnLoad gate fires:

1. Decode the judgement's three fields: the logic, the certificate term, and its type.
2. Check the type is a type, then check the certificate against it — the contract `eigentt:Judgement` states.
3. Checking walks `justification:Certificate.app`, which requires sub-certificates for `Certificate(j1, A -> B)` and `Certificate(j2, A)`.
4. Each grounding constructor requires a chain witness the kernel synthesizes:
   - `IsDeclaredAs("urn:…:rule_strong", HasLowIC50 -> StrongInhibitor)` — admitted if `rule_strong` was committed as a `justification:Claim` with matching `canonical_proposition` and a paired `prov:DeclarationTrace`.
   - `IsDeclaredAs("urn:…:plan_yields_lowic50", Asserts(s) -> HasLowIC50)` — the plan's reproducibility declaration.
   - `IsObservedAs("urn:…:m_eig0291_sampleset", Asserts(s))` — from the sample set's `prov:ObservationTrace`.
5. If every witness admits, the certificate type-checks; verdict is Holds. Otherwise Fails, with a diagnostic naming the missing family, IRI and proposition.

Note what is NOT consulted: the `StatisticalAnalysisResult` the institution emitted. It records what ran, and a run record grounds nothing.

The full fixture this snippet is drawn from lives at [`kernel/tests/fixtures/drug_screening.esl`](../../../kernel/tests/fixtures/drug_screening.esl); the matching test exercises the AutoOnLoad pipeline end-to-end.

### 9.10.5. Where the check happens

**At commit, in ordinary validation — there is no QueryClass.** `justification:judgement` is an
`eigentt:Judgement`-ranged slot, and **Rule 21** owns every such slot
([`kernel/src/validation/rules/eigentt_value.rs`](../../../kernel/src/validation/rules/eigentt_value.rs)):
decode the judgement, check its `type` is a type, then check its `term` against that type in check
mode. Checking `holds(kernel, c, Certificate(j, P))` therefore checks that `c` inhabits
`Certificate(j, P)` — the whole obligation, discharged by the rule that already existed for
annotated terms. Committing a conclusion whose certificate does not type-check fails validation and
the commit is rejected.

Checking `c` drives the kernel's `synthesize_chain_witness`, which admits a grounding constructor's
witness argument only when the layer's witness index holds a matching `(category, iri, proposition)`
key. That is the soundness boundary: a conclusion cannot cite a resource for a proposition the chain
never admitted. [`kernel/tests/certificate_admission.rs`](../../../kernel/tests/certificate_admission.rs)
exercises chain → witness index → synthesis → admission, and the three cases where admission must fail.

Three routes were deleted rather than rehomed, and the reasons are worth stating because each is a
claim about what belongs in an institution:

| was | why it went |
|---|---|
| `qc_validate_justification` (AutoOnLoad) | checking a certificate is type checking; the kernel does not delegate that |
| `qc_entailment_query` (OnDemand) | its question — *has a sentence claiming `P` been committed?* — is a witness-index lookup |
| `qc_consistency_check` (Decidable) | returned `Undecidable` for every non-empty input. A reserved IRI for an unbuilt decision procedure is a follow-up issue pretending to be a feature |

### 9.10.6. Cross-references

- [`kernel/src/validation/rules/eigentt_value.rs`](../../../kernel/src/validation/rules/eigentt_value.rs) — Rule 21, the check itself.
- [`kernel/src/justification/`](../../../kernel/src/justification/) — the support algebra over a retained term: `support`, `is_fully_verified`, `leaves_of`, `survives_without`, `cited_iris`.
- [`kernel/tests/certificate_admission.rs`](../../../kernel/tests/certificate_admission.rs) — the witness machinery end to end, including where admission must fail.
- [`ontologies/justification/justification.esl`](../../../ontologies/justification/justification.esl) — full ontology source: justification:Term, justification:Certificate, justification:Conclusion. The `witness:Is*As` predicates it references are declared in [`ontologies/core/core-ontology.json`](../../../ontologies/core/core-ontology.json), because the kernel constructs their inhabitants and a type the kernel inhabits cannot be owned by a layer above it.
- [D39 §3-§5](../../design/d39-justification-logic.md) — design rationale, the Justification Logic foundation, and the soundness story.
- [D49](../../design/d49-chainwitness-machinery.md) — chain-witness machinery the grounding constructors consume.
- [`platform/justification-logic/`](../platform/justification-logic/) — operational walkthrough: how to commit conclusions and compose with the D52 statistics institution. **Written against the institution that no longer exists**, so its verdict-inspection material is stale; the vocabulary and the worked chain are not.
- [Composition guide §1](../composition/01-introduction.md) — where the justification vocabulary sits in the composition story.

## 9.11. Hosting a logic — the operational protocol

The question this section answers is **not** whether a participating logic satisfies the definition
of an institution. That question is settled and uninteresting: the kernel itself is the degenerate
proof-theoretic case — no represented models, proofs as the entire content — and an entailment
system without models is a valid instantiation. The question that decides what a logic may do here
is operational: **can the system hold and re-check a witness for the claims that logic establishes?**

### 9.11.1. What a logic supplies, and what it may not

A participating logic supplies four things:

| supplies | example |
|---|---|
| **vocabulary** — classes, properties, inductives it declares on the chain | `lean:LeanProofTerm`, the D52 statistics claim schema |
| **a decision procedure yielding a verdict** | Lean's proof check; a statistics plan run against committed observations |
| **derivation resources** — what its run produced, as chain resources | a computed result, a trace |
| **optionally, a judgement** — `holds(logic, t, P)`, a checked triple | Lean, if it supplies the proof term; **not** statistics |

It supplies **none** of the following, and the separation is the point:

- **It does not assign a warrant.** Warrant is computed from a justification term, and nothing
  stores it. The grade classes and `reflection:epistemic_status` that used to are deleted, because a
  stored grade let the thing being graded nominate its own grade.
- **It does not define a witness kind.** The three `witness:Is*As` predicates are kernel base
  vocabulary in [`core-ontology.json`](../../../ontologies/core/core-ontology.json) — the kernel
  constructs their inhabitants, so no layer above it may declare or extend them. A protocol for
  institutions to supply their own witness kinds is unnecessary: a logic supplies a judgement in a
  logic the system checks, or its output is `Computed`.
- **It does not establish `Verified`.** A boolean verdict does not reach that grade; a checked
  judgement does. A logic that returns `Holds` but supplies no term produces output of `Computed`
  shape, however confident the verdict.

The last is worth stating concretely because it looks like a slight. The statistics institution
defines a satisfaction relation and provides no proof language: its output, `p < α`, is evidence
bearing on a claim rather than a derivation of it. That is not a deficiency to be fixed — a logic
whose satisfaction relation maps sentences to a dataset, a population, or the physical world has no
transportable witness object, so `Computed` is its ceiling by construction. The constraint is about
the host's capabilities, and takes no position on what counts as a logic.

**Veto is a separate power from grading, and a logic has it.** An institution may return `Fails`
and block a commit on its own authority. It may not establish `Verified` on its own authority. The
asymmetry is deliberate: an incorrect `Fails` loses data, which is recoverable; an incorrect `Holds`
that promoted a grade would corrupt the chain silently.

### 9.11.2. What admitting a new hosted checker costs

Two formal arguments, corresponding to the two ways a false `P'` could become an accepted `P`:

1. **Soundness of the external `⊢` against its `⊨`**, argued per hosted checker. For a
   type-theoretic checker under inhabitation semantics this is discharged *by construction* — "the
   checker accepts `t : P'`" and "`P'` holds" are the same statement. Where satisfaction is
   Tarskian, it is a theorem needing explicit proof.
2. **Satisfaction-preservation by its comorphism**, ensuring a `Verified` established externally
   transfers correctly. Sharpened to **inhabitation-preservation**: `α(φ)` is inhabited iff `φ` is.
   Non-executable, because the two inhabitants live in different type theories and the comorphism
   maps the proposition rather than the proof term.

Obligation 2 is where the real risk sits. The practical danger in admitting a Lean proof is not that
Lean's kernel accepts falsehoods; it is that the translated `P'` fails to denote the `P` you meant.

**And hosting is not a packaging decision.** It adds both obligations *and the checker's
implementation* to the trusted computing base. The verification institution links `nanoda_lib`
statically into the kernel, so admitting Lean expands the TCB by the whole of that implementation.

The TCB is exactly: the kernel's native type checker, each hosted external proof checker, each
formal comorphism, and the constant specification governing attributions. It excludes the automated
prover that *found* a proof, any unverified institutional verdict, and class-membership heuristics.

### 9.11.3. Why the justification vocabulary is not an institution

§9.10 is the worked case. Justification logic supplies vocabulary the kernel checks directly, so it
needs no hosted checker, no comorphism, and neither obligation above — and correspondingly it gets
no authority the kernel does not already have. When it was housed as an institution, the one thing
that institution did at commit was type-check a certificate, which the kernel does not delegate. It
was deleted, and nothing was lost, because an institution is the mechanism for a logic the kernel
*cannot* check itself.

That is the test to apply to a proposed institution: **if the kernel can already check it, it is
vocabulary, not an institution.**

---

Next: **[10. Error messages →](10-error-messages.md)**
