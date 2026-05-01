# 9. Institutions in ESL

An **institution** is a domain-specific reasoning system registered with the kernel. Under D14 ([Institution Realisation](../../design/d14-institution-realisation.md), supersedes D10) institutions are declared as ontology resources committed to the layer chain — `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, and `Comorphism` — plus a runtime that handles the boundary translations and any opaque reasoning. There is no `FiberDeclaration` struct any more: the chain *is* the declaration.

This chapter covers the institution surface from a program-author's perspective in ESL: how qualified-name function calls dispatch to a `Decidable` `QueryClass`, how the resulting `Verdict` flows through Mini-TT, and the worked life-science example that drove the design.

The implementer view (writing a new institution as a WASM binary) lives in [platform §10](../platform/10-wasm-institutions.md). The query-language view of the same surface is [EigenQL §8](../eigenql/08-institutions.md). The protocol spec is [D14](../../design/d14-institution-realisation.md).

## 9.1. What an institution declares

Five resource shapes carry the institution surface (D14 §4). All of them are ordinary typed resources: an ESL author can read and write them with `class`, `property`, and `resource` declarations, but in practice they are usually shipped alongside the institution's WASM binary as Eigon-JSON.

| Shape | Carries |
|---|---|
| `Institution` | `institution_iri`, `name`, `runtime` (`wasm` / `external` / `in_process`). |
| `ExportFormat` | `from_class`, `payload_type`, `institution_ref`, `procedure`. |
| `ImportFormat` | `to_class`, `payload_type`, `institution_ref`, `procedure`. |
| `QueryClass` | `query_class` (input), `result_class`, `dispatch_role` ⊆ `{OnDemand, AutoOnLoad, Decidable}`, `query_handler`, `institution_ref`. |
| `Comorphism` | `export_format`, `transformation` (a Mini-TT Component), `import_format`, `exact: bool`. |

ESL doesn't *require* you to write these by hand — the `eigenius-wasm-sdk::institution` module's [`InstitutionDecl`](../../../sdk/wasm-sdk/src/institution.rs), `ExportFormatDecl`, `ImportFormatDecl`, `QueryClassDecl`, and `ComorphismDecl` builders construct them programmatically. But the resources they produce are ordinary Eigon, and an ESL author can read and reason about them through the layer.

The kernel's [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) is a derived index built by scanning the chain (D14 §3); ESL's compile-time classifier uses it. There is no separate `register()` call any more — committing the declarations to the chain *is* registration.

## 9.2. Classification at compile time

When the ESL compiler encounters a function-call expression with a qualified name, it asks the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) to classify the IRI. The classification lives in [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs); it returns one of:

| Classification | ESL emits |
|---|---|
| `Decidable` `QueryClass` | `Exp::NativeDecide(Constraint::Institution { iri, args }, Unit)` — a Mini-TT term that reduces a `Verdict` to either `Refl(v)` (Holds), a failing neutral (Fails), or stays as a passthrough neutral (Undecidable). |
| Otherwise | Falls through to component dispatch / class constructor / unbound — same as today. If nothing matches, the call fails at compile or evaluate time with `unknown function`. |

Two changes from D10's classifier:

- **No `Comorphism` from expression position.** D10 emitted `Exp::InstitutionInvoke { iri, source }` for `cap:translate(source)` calls. Under D14, comorphisms surface only in EigenQL FIBER param coercion. ESL programs that need a translated resource construct it explicitly (or invoke a derived program over the comorphism Component, which is just a regular Mini-TT Component).
- **No `OnDemand` `QueryClass` from expression position.** OnDemand QueryClasses are reachable only from EigenQL `FIBER` clauses. ESL programs that need a structured response from an institution embed a FIBER call into an EigenQL query (and feed the bound response back as a Mini-TT value if needed).

The classifier is consulted only by `compile_with_institutions`. The plain `compile` entry point still works for programs that don't reference institution-dispatched IRIs:

```rust
let resources = esl::compile_with_institutions(source, Arc::clone(&institution_index))?;
```

This is the same single-source-of-truth classification used by EigenQL ([EigenQL §8](../eigenql/08-institutions.md)) — both surface languages share `InstitutionIndex`, so the same IRI dispatches identically.

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

**At type-check (Check mode):** the kernel resolves the constraint IRI in the `InstitutionIndex`, reads the QueryClass's `institution_ref` and `query_handler`, and calls [`Institution::query`](../../../kernel/src/institution/runtime.rs) on the registered runtime. The institution returns a `Verdict` resource; the kernel reads its `ctor_name` property and reduces:

| `ctor_name` | Kernel behaviour |
|---|---|
| `Holds` | Reduces `NativeDecide` to `Refl(Unit)` — predicate accepted at check time. |
| `Fails` | Emits a failing neutral — type-check rejects the program with the institution's diagnostic. |
| `Undecidable` | Stays as a passthrough neutral — the predicate is left for runtime. |

**At runtime (IO mode):** same dispatch path. An `Undecidable` from check time becomes a runtime call with the same outcome semantics.

**Synthetic input shape.** The kernel constructs a synthetic input resource of the QueryClass's declared `query_class`. Positional arguments are attached as the property `urn:eigenius:institution:decide_args` (an array). Institutions whose handlers expect named-property inputs (typical when the QC is also FIBER-callable) read the args off named properties; institutions whose handlers expect positional args read `decide_args`. The dock-assay demo's [`AssayInstitution::within_tolerance_verdict`](../../../examples/wasm-d14-assay/src/lib.rs) shows both shapes side by side.

**Default behaviour.** Institutions whose runtime returns `NotImplemented` for a procedure surface as a runtime evaluation error at dispatch time. There is no longer a "silent Undecidable" fallback (the D10 behaviour) — under D14, `Undecidable` is a *value the institution returns*, not a default the kernel substitutes when reasoning is unavailable.

## 9.4. Invoking an OnDemand QueryClass — through EigenQL

OnDemand QueryClasses are not directly callable from ESL expressions. They are reached only through an EigenQL `FIBER` clause. ESL programs that need a structured response from an institution typically embed the FIBER call into an EigenQL query and feed the bound response back as a Mini-TT value.

This is a deliberate scope decision. Decidable handles the common cap-as-predicate case from program bodies; the FIBER form handles the multi-property-response case from EigenQL. Putting OnDemand calls back into ESL expression position would mean two return shapes (Verdict vs. arbitrary resource) and two dispatch paths in the compiler — the FIBER form covers that surface unambiguously.

## 9.5. Comorphisms — declared, not invoked from ESL

Under D14, a `Comorphism` is a typed resource:

```
Comorphism dock_to_assay
  export_format: ef_dock_to_dg          // DockingResult → Float (dock institution)
  transformation: cm_arrhenius          // Float → Float Mini-TT Component
  import_format: if_assay_from_ic50     // Float → AssayPrediction (assay institution)
  exact: false
```

The kernel statically type-checks at commit time that the transformation Component's signature matches `(payload_type(export_format)) → (payload_type(import_format))` (D14 §4.5). A comorphism with mismatched types is rejected.

ESL doesn't surface comorphisms as expressions — there is no `cap:dock_to_assay(input)` call form. Three ways to use a comorphism from program code:

1. **Run the constituent Component directly.** The transformation is a regular Mini-TT Component (`cm_arrhenius` in the example). If your ESL program already has the typed payload, it can apply the Component:
   ```esl
   let ic50 : core:float = cm_arrhenius(delta_g);
   ```
   The composition extract → component → reify is what the kernel does on the user's behalf inside FIBER param coercion; programs that have the payload already only need the middle step.

2. **Translate inside an EigenQL query.** Use a `FIBER` clause whose param value is `comorphism(source)` (see [EigenQL §8.6](../eigenql/08-institutions.md)). The query runs the four-step pipeline and the resulting reified resource flows into the FIBER's input.

3. **Wrap as a Component-implemented OnDemand QueryClass and invoke via FIBER.** A `OnDemand` QueryClass whose `implementation` is a Component IRI dispatches as `extract → component → reify` automatically (D14 §6.2). EigenQL FIBER reaches it; from ESL, embed the FIBER call into an EigenQL query.

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

The M8 worked example ships with the kernel ([`ontologies/examples/d14-dock-assay/dock-assay.json`](../../../ontologies/examples/d14-dock-assay/dock-assay.json), tested in [`kernel/tests/d14_dock_assay_demo.rs`](../../../kernel/tests/d14_dock_assay_demo.rs)). Two institutions:

- `dock` — owns the `DockingResult` class. Declares an `ExportFormat` (`ef_dock_to_dg`) extracting `delta_g` as a `Float`.
- `assay` — owns the `AssayPrediction` class. Declares an `ImportFormat` (`if_assay_from_ic50`) reifying a `Float` as `ic50`. Also declares three `QueryClass`es: `within_tolerance` (Decidable, three-arg predicate), `assay_prediction_validity` (AutoOnLoad, validates positive IC₅₀ on Load), and `validate_prediction` (OnDemand, FIBER-callable).

A `Comorphism` (`dock_to_assay`) ties them together via a Mini-TT Component (`cm_arrhenius`, the Arrhenius approximation `IC₅₀ ≈ exp(-ΔG/RT)`).

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

To run it under Check or IO mode, the caller threads the `InstitutionRuntime` (with a runtime-registered or auto-registered `WasmInstitution`) through the kernel's `EvalCtx::IO`:

```rust
let runtime_ctx = EvalCtx::IO {
    layer: Arc::clone(&layer),
    registry: components,
    institution_index: Some(Arc::clone(&index)),
    institution_runtime: Some(Arc::clone(&inst_runtime)),
    trace_store: None,
    dispatched_traces: Default::default(),
    task_context: None,
};
```

For the WASM-hosted variant, see [`kernel/tests/d14_dock_assay_demo_wasm.rs`](../../../kernel/tests/d14_dock_assay_demo_wasm.rs) — it constructs the same surface but with `WasmInstitution` instances auto-registered from a child layer carrying `runtime: wasm` + inline `wasm_binary` declarations.

See [`docs/design/life-science-requirements.md`](../../design/life-science-requirements.md) for the original motivating discussion.

## 9.8. The classification table (cross-language)

The same classification mechanism powers ESL and EigenQL. When the same IRI appears in both languages, it dispatches identically:

| Index lookup | ESL emits | EigenQL emits | Runtime call |
|---|---|---|---|
| `Decidable` QueryClass | `Exp::NativeDecide(Constraint::Institution { … }, Unit)` | Same — projected to Boolean by postfix `HOLDS`/`FAILS`/`UNDECIDABLE` | `Institution::query(query_handler, synthetic_input, ctx)` |
| `OnDemand` QueryClass | not exposed in ESL expression position | `FIBER` clause | `Institution::query(query_handler, input, ctx)` |
| `Comorphism` | declared, not invoked from ESL expressions | only in FIBER param coercion | `extract_typed → transformation → reify` four-step pipeline |
| Class / primitive / literal | `Exp::EigonClass(iri)` etc. | resolved via layer | various |

Cross-link: [EigenQL chapter 8](../eigenql/08-institutions.md) covers the same table from the query-language side.

## 9.9. Where the implementation lives

| File | Purpose |
|---|---|
| [`kernel/src/institution/runtime.rs`](../../../kernel/src/institution/runtime.rs) | `Institution` trait (D14 §8), `InstitutionRuntime` |
| [`kernel/src/institution/registry.rs`](../../../kernel/src/institution/registry.rs) | `InstitutionIndex` (derived from chain scan) |
| [`kernel/src/institution/dispatch.rs`](../../../kernel/src/institution/dispatch.rs) | Auto-on-Load dispatch |
| [`kernel/src/institution/error.rs`](../../../kernel/src/institution/error.rs) | `InstitutionError` |
| [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs) | Compile-time classification of `Apply` expressions through `InstitutionIndex` |
| [`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs) | `NativeDecide` check arm — fires Decidable QueryClasses at type-check time |
| [`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs) | `decide_constraint` evaluation |
| [`kernel/src/capability/registration.rs`](../../../kernel/src/capability/registration.rs) | `build_wasm_institution_runtime` — auto-registration from chain scan |
| [`kernel/src/capability/wasm_institution_d14.rs`](../../../kernel/src/capability/wasm_institution_d14.rs) | `WasmInstitution` host bridge to the `eigenius-institution-d14` WIT world |

---

Next: **[10. Error messages →](10-error-messages.md)**
